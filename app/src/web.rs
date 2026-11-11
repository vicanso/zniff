//! Operator console for a headless zniff, including the Docker image.
//!
//! The browser talks only to this HTTP server. The capture proxy is a second
//! socket (`listen_port`, default 8877) and is bound only after the console
//! starts capture. This server never accepts CONNECT and never forwards.

use std::io::{ErrorKind, Result};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::ca::Ca;
use crate::proxy::{BreakDecision, BreakSide, Capture, Hub, ReplayOverride};
use crate::session::{self, Direction, Flow};
use crate::share::prefers_zh;

const WEB_DEFAULT_PORT: u16 = 8080;
const HEADER_CAP: usize = 16 * 1024;
const BODY_CAP: usize = 8 * 1024;
/// Rules, HAR, and replay bodies. The small cap still applies to capture and clear.
const HUB_BODY_CAP: usize = 1024 * 1024;
const LIST_CAP: usize = 400;
const PREVIEW_CAP: usize = 256 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

pub struct WebSettings {
    pub web_bind: IpAddr,
    pub web_port: u16,
    pub capture_bind: IpAddr,
    pub capture_port: u16,
}

impl WebSettings {
    pub fn web_addr(&self) -> SocketAddr {
        SocketAddr::new(self.web_bind, self.web_port)
    }
}

/// `--web` flags. `config_port` is the capture port when `--port` is omitted.
/// Parsing does not open either socket.
pub fn cli_settings(args: &[String], config_port: u16) -> std::result::Result<WebSettings, String> {
    let mut web_port = None;
    let mut web_bind = None;
    let mut capture_port = None;
    let mut capture_bind = None;
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if let Some(rest) = arg.strip_prefix("--web-port=") {
            web_port = Some(parse_port(rest)?);
        } else if arg == "--web-port" {
            index += 1;
            let next = args.get(index).ok_or_else(|| "缺少网页端口".to_string())?;
            web_port = Some(parse_port(next)?);
        } else if let Some(rest) = arg.strip_prefix("--web-bind=") {
            web_bind = Some(parse_bind(rest)?);
        } else if arg == "--web-bind" {
            index += 1;
            let next = args.get(index).ok_or_else(|| "缺少网页地址".to_string())?;
            web_bind = Some(parse_bind(next)?);
        } else if let Some(rest) = arg.strip_prefix("--port=") {
            capture_port = Some(parse_port(rest)?);
        } else if arg == "--port" || arg == "-p" {
            index += 1;
            let next = args.get(index).ok_or_else(|| "缺少抓包端口".to_string())?;
            capture_port = Some(parse_port(next)?);
        } else if let Some(rest) = arg.strip_prefix("--bind=") {
            capture_bind = Some(parse_bind(rest)?);
        } else if arg == "--bind" {
            index += 1;
            let next = args.get(index).ok_or_else(|| "缺少抓包地址".to_string())?;
            capture_bind = Some(parse_bind(next)?);
        }
        index += 1;
    }
    Ok(WebSettings {
        web_bind: web_bind.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        web_port: web_port.unwrap_or(WEB_DEFAULT_PORT),
        capture_bind: capture_bind.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        capture_port: capture_port.unwrap_or(config_port),
    })
}

/// Serves the console until Ctrl-C. Capture stays closed until the page starts it.
/// Does not change the system proxy and does not read `capturing` from config.
pub fn run_cli(args: &[String], config_port: u16) -> i32 {
    let settings = match cli_settings(args, config_port) {
        Ok(settings) => settings,
        Err(message) => {
            eprintln!("{message}");
            return 2;
        }
    };
    let ca = match Ca::open_default() {
        Ok(ca) => Arc::new(ca),
        Err(err) => {
            eprintln!("zniff web 没有打开证书: {err:#}");
            return 1;
        }
    };
    let hub = match Hub::new(ca) {
        Ok(hub) => Arc::new(hub),
        Err(err) => {
            eprintln!("zniff web 没有准备好: {err:#}");
            return 1;
        }
    };
    // The server process applies saved rules. It does not write `capturing`
    // and it does not change the system proxy. Tests call `WebServer::start`
    // directly and do not load this file.
    let dev_mode = crate::config::is_dev_mode();
    let data_dir = crate::ca::data_dir();
    let loaded = crate::config::load_mode(&data_dir, dev_mode);
    hub.set_bypass(&loaded.bypass_hosts);
    hub.set_rules(&loaded.rules);
    hub.set_upstream(&loaded.upstream_proxy);
    hub.set_rules_enabled(loaded.rules_enabled);
    hub.set_breaks_enabled(loaded.breaks_enabled);
    hub.set_show_tunnel_failures(loaded.show_tunnel_failures);
    hub.set_relay_ingress(loaded.relay_accept, &loaded.relay_token);
    let prefs = WebPrefs::from_loaded(&loaded, data_dir, dev_mode);
    let web_root = packaged_web_root();
    match &web_root {
        Some(dir) => println!("界面目录 {}", dir.display()),
        None => eprintln!("没有共用界面，这次是简易操作台。在仓库根目录运行 make web-ui。"),
    }
    let started = WebServer::start_with_prefs(
        settings.web_addr(),
        hub,
        settings.capture_bind,
        settings.capture_port,
        web_root,
        prefs,
    );
    let server = match started {
        Ok(server) => server,
        Err(err) => {
            eprintln!("zniff web 没有绑上 {}: {err}", settings.web_addr());
            return 1;
        }
    };
    println!("zniff web listening on http://{}", server.addr());
    println!("抓包端口 {} 在页面里开始之后才打开", settings.capture_port);
    if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        runtime.block_on(async {
            let _ = tokio::signal::ctrl_c().await;
        });
    }
    drop(server);
    0
}

pub struct WebServer {
    addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WebServer {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn start(
        web_bind: SocketAddr,
        hub: Arc<Hub>,
        capture_ip: IpAddr,
        capture_port: u16,
    ) -> Result<Self> {
        Self::start_with(web_bind, hub, capture_ip, capture_port, None)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn start_with(
        web_bind: SocketAddr,
        hub: Arc<Hub>,
        capture_ip: IpAddr,
        capture_port: u16,
        web_root: Option<PathBuf>,
    ) -> Result<Self> {
        Self::start_with_prefs(
            web_bind,
            hub,
            capture_ip,
            capture_port,
            web_root,
            WebPrefs::default(),
        )
    }

    fn start_with_prefs(
        web_bind: SocketAddr,
        hub: Arc<Hub>,
        capture_ip: IpAddr,
        capture_port: u16,
        web_root: Option<PathBuf>,
        prefs: WebPrefs,
    ) -> Result<Self> {
        if capture_port == 0 {
            return Err(std::io::Error::new(
                ErrorKind::InvalidInput,
                "capture port is 0",
            ));
        }
        let listener = std::net::TcpListener::bind(web_bind)?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&shutdown);
        let thread = std::thread::Builder::new()
            .name("zniff-web".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        log::error!("web runtime: {err}");
                        return;
                    }
                };
                runtime.block_on(async move {
                    let listener = match tokio::net::TcpListener::from_std(listener) {
                        Ok(listener) => listener,
                        Err(err) => {
                            log::error!("web listen: {err}");
                            return;
                        }
                    };
                    let slot = Arc::new(Mutex::new(Slot {
                        hub,
                        capture: None,
                        capture_ip,
                        capture_port,
                        error: String::new(),
                        relay_push: None,
                        web_root,
                        prefs,
                    }));
                    loop {
                        if flag.load(Ordering::Relaxed) {
                            break;
                        }
                        tokio::select! {
                            _ = wait_shutdown(&flag) => break,
                            accepted = listener.accept() => {
                                match accepted {
                                    Ok((socket, _)) => {
                                        let slot = Arc::clone(&slot);
                                        tokio::spawn(async move {
                                            if let Err(err) = serve_one(socket, slot).await {
                                                log::debug!("web connection: {err}");
                                            }
                                        });
                                    }
                                    Err(err) => {
                                        log::debug!("web accept: {err}");
                                        tokio::time::sleep(Duration::from_millis(20)).await;
                                    }
                                }
                            }
                        }
                    }
                    if let Some(capture) = lock(&slot).capture.take() {
                        capture.stop();
                    }
                    if let Some(handle) = lock(&slot).relay_push.take() {
                        handle.stop();
                    }
                });
            })?;
        Ok(Self {
            addr,
            shutdown,
            thread: Some(thread),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for WebServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Clone, Default)]
struct WebPrefs {
    config_dir: Option<PathBuf>,
    dev_mode: bool,
    active_profile: String,
    filter: String,
    saved_profiles: Vec<crate::config::SavedProfile>,
    profile_history: Vec<crate::config::SavedProfile>,
    redact_export: bool,
    redact_strict: bool,
    redact_fields: String,
    relay_url: String,
}

impl WebPrefs {
    fn from_loaded(loaded: &crate::config::Config, config_dir: PathBuf, dev_mode: bool) -> Self {
        Self {
            config_dir: Some(config_dir),
            dev_mode,
            active_profile: loaded.active_profile.clone(),
            filter: loaded.filter.clone(),
            saved_profiles: loaded.saved_profiles.clone(),
            profile_history: loaded.profile_history.clone(),
            redact_export: loaded.redact_export,
            redact_strict: loaded.redact_strict,
            redact_fields: loaded.redact_fields.clone(),
            relay_url: loaded.relay_url.clone(),
        }
    }
}

struct Slot {
    hub: Arc<Hub>,
    capture: Option<Capture>,
    capture_ip: IpAddr,
    capture_port: u16,
    error: String,
    relay_push: Option<crate::relay::PushHandle>,
    /// Packaged nsh page. `None` keeps the small HTML console.
    web_root: Option<PathBuf>,
    prefs: WebPrefs,
}

#[derive(Serialize)]
struct StateBody {
    capturing: bool,
    listen: String,
    listen_port: u16,
    listen_all: bool,
    flows: usize,
    error: String,
    share_id: String,
    generation: u64,
    paused: Option<PausedBody>,
    paused_queue: Vec<PausedBody>,
    prefs: PrefsBody,
}

#[derive(Serialize)]
struct PausedBody {
    id: u64,
    side: &'static str,
}

#[derive(Serialize)]
struct PrefsBody {
    bypass: String,
    rules: String,
    upstream: String,
    rules_enabled: bool,
    breaks_enabled: bool,
    show_tunnels: bool,
    relay_accept: bool,
    relay_url: String,
    active_profile: String,
    filter: String,
    saved_profiles: Vec<crate::config::SavedProfile>,
    profile_history: Vec<crate::config::SavedProfile>,
    redact_export: bool,
    redact_strict: bool,
    redact_fields: String,
}

struct Head {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
}

struct Reply {
    status: u16,
    reason: &'static str,
    content_type: std::borrow::Cow<'static, str>,
    attachment: Option<String>,
    body: Vec<u8>,
}

async fn serve_one(mut socket: TcpStream, slot: Arc<Mutex<Slot>>) -> Result<()> {
    let (head_bytes, rest) = read_header(&mut socket).await?;
    let (is_head, reply) = match parse_head(&head_bytes) {
        Ok(head) => {
            let is_head = head.method.eq_ignore_ascii_case("HEAD");
            let reply = route(&mut socket, &head, &rest, &slot).await;
            (is_head, reply)
        }
        Err(_) => (false, text(400, "Bad Request", "bad request")),
    };
    let bytes = encode(&reply, is_head);
    let _ = tokio::time::timeout(IO_TIMEOUT, socket.write_all(&bytes)).await;
    Ok(())
}

async fn route(socket: &mut TcpStream, head: &Head, rest: &[u8], slot: &Mutex<Slot>) -> Reply {
    if head.method.eq_ignore_ascii_case("CONNECT") || head.target.contains("://") {
        return text(403, "Forbidden", "web server does not proxy");
    }
    let path = head.target.split('?').next().unwrap_or("/");
    let method = head.method.as_str();
    let web_root = lock(slot).web_root.clone();
    if method.eq_ignore_ascii_case("GET") || method.eq_ignore_ascii_case("HEAD") {
        if path == "/" {
            if let Some(file) = static_reply(path, web_root.as_deref()) {
                return file;
            }
            let zh = prefers_zh(header(head, "accept-language"));
            return html(200, "OK", page(zh).into_bytes());
        }
        if path == "/api/state" {
            return json_reply(200, "OK", &state_value(&lock(slot)));
        }
        if path == "/api/flows" {
            let filter = query_value(&head.target, "filter");
            let hub = Arc::clone(&lock(slot).hub);
            return json_reply(200, "OK", &flow_list(&hub, &filter));
        }
        if path == "/api/export.har" {
            let filter = query_value(&head.target, "filter");
            let (hub, redact) = {
                let guard = lock(slot);
                (
                    Arc::clone(&guard.hub),
                    redact_options_for_target(&guard.prefs, &head.target),
                )
            };
            return export_har_reply(&hub, &filter, &redact);
        }
        if path == "/api/export.html" {
            let filter = query_value(&head.target, "filter");
            let zh_override = query_value(&head.target, "zh");
            let lang_override = query_value(&head.target, "lang");
            let zh = match (zh_override.as_str(), lang_override.as_str()) {
                ("1" | "true", _) | (_, "zh") => true,
                ("0" | "false", _) | (_, "en") => false,
                _ => prefers_zh(header(head, "accept-language")),
            };
            let (hub, redact) = {
                let guard = lock(slot);
                (
                    Arc::clone(&guard.hub),
                    redact_options_for_target(&guard.prefs, &head.target),
                )
            };
            return export_html_reply(&hub, &filter, &redact, zh);
        }
        if let Some(rest_flow) = path.strip_prefix("/api/flows/") {
            if let Some(id_str) = rest_flow.strip_suffix("/body")
                && let Ok(id) = id_str.parse::<u64>()
            {
                let (hub, redact) = {
                    let guard = lock(slot);
                    (
                        Arc::clone(&guard.hub),
                        redact_options_for_target(&guard.prefs, &head.target),
                    )
                };
                return flow_body_reply(&hub, id, &redact);
            }
            if let Ok(id) = rest_flow.parse::<u64>() {
                let hub = Arc::clone(&lock(slot).hub);
                return match flow_detail(&hub, id) {
                    Some(value) => json_reply(200, "OK", &value),
                    None => json_reply(404, "Not Found", &json!({"error": "not found"})),
                };
            }
        }
        if let Some(file) = static_reply(path, web_root.as_deref()) {
            return file;
        }
        return text(404, "Not Found", "not found");
    }
    if method.eq_ignore_ascii_case("POST") {
        let cap = if path == "/api/hub" {
            HUB_BODY_CAP
        } else {
            BODY_CAP
        };
        let body = match read_body(socket, head, rest, cap).await {
            Ok(body) => body,
            Err(reply) => return reply,
        };
        if path == "/api/capture" {
            let value = match serde_json::from_slice::<Value>(&body) {
                Ok(value) => value,
                Err(_) => return text(400, "Bad Request", "need JSON"),
            };
            let Some(on) = value.get("on").and_then(Value::as_bool) else {
                return text(400, "Bad Request", "need {\"on\":true|false}");
            };
            let listen_all = value.get("listen_all").and_then(Value::as_bool);
            let port = match value.get("port") {
                None | Some(Value::Null) => None,
                Some(value) => match value.as_u64().and_then(|port| u16::try_from(port).ok()) {
                    Some(port) => Some(port),
                    None => return text(400, "Bad Request", "bad port"),
                },
            };
            let (status, value) = {
                let mut slot = lock(slot);
                let (status, body) = set_capture(&mut slot, on, listen_all, port);
                (status, state_value_from(&body))
            };
            let reason = match status {
                200 => "OK",
                400 => "Bad Request",
                _ => "Conflict",
            };
            return json_reply(status, reason, &value);
        }
        if path == "/api/clear" {
            lock(slot).hub.clear_session();
            return json_reply(200, "OK", &state_value(&lock(slot)));
        }
        if path == "/api/hub" {
            let value = match serde_json::from_slice::<Value>(&body) {
                Ok(value) => value,
                Err(_) => return text(400, "Bad Request", "need JSON"),
            };
            return hub_dispatch(slot, head, value).await;
        }
        return text(404, "Not Found", "not found");
    }
    text(405, "Method Not Allowed", "method not allowed")
}

fn set_capture(
    slot: &mut Slot,
    on: bool,
    listen_all: Option<bool>,
    port: Option<u16>,
) -> (u16, StateBody) {
    if on {
        if slot.capture.is_some() {
            // Already listening. A new address applies on the next start.
            slot.error.clear();
            return (200, state_of(slot));
        }
        if let Some(0) = port {
            slot.error = "端口无效".to_string();
            return (400, state_of(slot));
        }
        if let Some(port) = port {
            slot.capture_port = port;
        }
        if let Some(all) = listen_all {
            slot.capture_ip = if all {
                IpAddr::V4(Ipv4Addr::UNSPECIFIED)
            } else {
                IpAddr::V4(Ipv4Addr::LOCALHOST)
            };
        }
        let addr = SocketAddr::new(slot.capture_ip, slot.capture_port);
        match Capture::spawn(Arc::clone(&slot.hub), addr) {
            Ok(capture) => {
                slot.capture = Some(capture);
                slot.error.clear();
                (200, state_of(slot))
            }
            Err(err) => {
                slot.error = format!("{err:#}");
                (409, state_of(slot))
            }
        }
    } else {
        if let Some(capture) = slot.capture.take() {
            capture.stop();
        }
        if let Some(handle) = slot.relay_push.take() {
            handle.stop();
        }
        slot.hub.end_share();
        slot.error.clear();
        (200, state_of(slot))
    }
}

fn state_of(slot: &Slot) -> StateBody {
    let listen = slot
        .capture
        .as_ref()
        .map(|capture| capture.addr.to_string())
        .unwrap_or_default();
    let listen_port = slot
        .capture
        .as_ref()
        .map(|capture| capture.addr.port())
        .unwrap_or(slot.capture_port);
    let flows = slot.hub.with_session(|session| session.len());
    let queue = slot.hub.paused_queue();
    StateBody {
        capturing: slot.capture.is_some(),
        listen,
        listen_port,
        listen_all: slot.capture_ip.is_unspecified(),
        flows,
        error: slot.error.clone(),
        share_id: slot.hub.share_id().unwrap_or_default(),
        generation: slot.hub.generation(),
        paused: queue.first().map(|(id, side)| PausedBody {
            id: *id,
            side: break_side_name(*side),
        }),
        paused_queue: queue
            .iter()
            .map(|(id, side)| PausedBody {
                id: *id,
                side: break_side_name(*side),
            })
            .collect(),
        prefs: PrefsBody {
            bypass: slot.hub.bypass_text(),
            rules: slot.hub.rules_text(),
            upstream: slot.hub.upstream_text(),
            rules_enabled: slot.hub.rules_enabled(),
            breaks_enabled: slot.hub.breaks_enabled(),
            show_tunnels: slot.hub.tunnel_failures_shown(),
            relay_accept: slot.hub.relay_accepts(),
            relay_url: slot.prefs.relay_url.clone(),
            active_profile: slot.prefs.active_profile.clone(),
            filter: slot.prefs.filter.clone(),
            saved_profiles: slot.prefs.saved_profiles.clone(),
            profile_history: slot.prefs.profile_history.clone(),
            redact_export: slot.prefs.redact_export,
            redact_strict: slot.prefs.redact_strict,
            redact_fields: slot.prefs.redact_fields.clone(),
        },
    }
}

fn break_side_name(side: BreakSide) -> &'static str {
    match side {
        BreakSide::Response => "response",
        BreakSide::Request => "request",
    }
}

fn state_value(slot: &Slot) -> Value {
    state_value_from(&state_of(slot))
}

fn state_value_from(body: &StateBody) -> Value {
    serde_json::to_value(body).unwrap_or(json!({}))
}

fn flow_list(hub: &Hub, filter: &str) -> Value {
    // `finish` reads spilled bodies and must not run under the session lock.
    let plan = hub.with_session(|session| session.plan_filter(filter));
    let matched = plan.finish();
    let (session_total, rows) = hub.with_session(|session| {
        let rows = matched
            .into_iter()
            .filter_map(|(id, _)| session.get(id).map(flow_summary))
            .collect::<Vec<_>>();
        (session.len(), rows)
    });
    let total = rows.len();
    let start = total.saturating_sub(LIST_CAP);
    json!({
        "total": total,
        "session_total": session_total,
        "omitted": start,
        "flows": &rows[start..],
    })
}

fn conn_info_json(conn: &session::ConnInfo) -> Value {
    json!({
        "dns_ms": conn.dns_ms,
        "tcp_ms": conn.tcp_ms,
        "tls_ms": conn.tls_ms,
        "reused": conn.reused,
        "server_addr": conn.server_addr,
        "tls_version": conn.tls_version,
        "dialed_before_start": conn.dialed_before_start,
    })
}

fn flow_summary(flow: &Flow) -> Value {
    json!({
        "id": flow.id,
        "method": flow.method,
        "host": flow.host,
        "path": flow.path,
        "status": flow.status,
        "error": flow.error,
        "protocol": flow.protocol.label(),
        "client_ip": flow.client_ip,
        "conn_info": conn_info_json(&flow.conn_info),
        "response_bytes": flow.response_bytes,
        "relay": flow.relay_source.as_deref().map(session::relay_short).unwrap_or_default(),
        "rev": flow.rev,
        "relay_source": flow.relay_source,
        "url": flow.url,
        "pinned": flow.pinned,
        "rule_hit": flow.rule_hit,
        "comment": flow.comment,
        "started_ms": unix_millis(flow.started),
        "response_ms": flow.response_at.map(unix_millis),
        "finished_ms": flow.finished_at.map(unix_millis),
        "request_bytes": flow.request_bytes,
    })
}

fn unix_millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn flow_detail(hub: &Hub, id: u64) -> Option<Value> {
    hub.with_session(|session| {
        let flow = session.hydrated(id)?;
        let (request_body, request_body_b64, request_truncated, request_binary) =
            preview(&flow.request_body);
        let (response_body, response_body_b64, response_truncated, response_binary) =
            preview(&flow.response_body);
        Some(json!({
            "id": flow.id,
            "rev": flow.rev,
            "method": flow.method,
            "url": flow.url,
            "host": flow.host,
            "path": flow.path,
            "status": flow.status,
            "error": flow.error,
            "protocol": flow.protocol.label(),
            "client_ip": flow.client_ip,
            "conn_info": conn_info_json(&flow.conn_info),
            "rule_hit": flow.rule_hit,
            "relay": flow.relay_source.as_deref().map(session::relay_short).unwrap_or_default(),
            "request_headers": pairs(&flow.request_headers),
            "response_headers": pairs(&flow.response_headers),
            "request_body": request_body,
            "request_body_b64": request_body_b64,
            "request_truncated": request_truncated || flow.request_truncated,
            "request_binary": request_binary,
            "request_encoding_error": flow.request_encoding_error,
            "request_expired": flow.request_expired,
            "response_body": response_body,
            "response_body_b64": response_body_b64,
            "response_truncated": response_truncated || flow.response_truncated,
            "response_binary": response_binary,
            "response_encoding_error": flow.response_encoding_error,
            "response_expired": flow.response_expired,
            "messages_truncated": flow.messages_truncated,
            "messages": flow.messages.iter().map(|message| json!({
                "direction": match message.direction {
                    Direction::Response => "response",
                    Direction::Request => "request",
                },
                "kind": message.kind,
                "text": message.text,
            })).collect::<Vec<_>>(),
        }))
    })
}

fn redact_options_for_target(prefs: &WebPrefs, target: &str) -> crate::share::RedactOptions {
    let enabled = match query_value_opt(target, "redact").as_deref() {
        Some("1" | "true") => true,
        Some("0" | "false") => false,
        _ => prefs.redact_export,
    };
    let strict_override = match query_value_opt(target, "redact_strict").as_deref() {
        Some("1" | "true") => Some(true),
        Some("0" | "false") => Some(false),
        _ => None,
    };
    let fields =
        query_value_opt(target, "redact_fields").unwrap_or_else(|| prefs.redact_fields.clone());
    let mut opts = crate::share::RedactOptions::new(enabled, &fields);
    if let Some(strict) = strict_override {
        opts.strict = strict;
    } else if prefs.redact_strict {
        opts.strict = true;
    }
    opts
}

fn body_extension_for_mime(content_type: &str) -> &'static str {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match mime.as_str() {
        "application/json" | "text/json" => "json",
        "text/html" | "application/xhtml+xml" => "html",
        "application/xml" | "text/xml" => "xml",
        "text/plain" | "text/event-stream" => "txt",
        "text/csv" => "csv",
        "text/css" => "css",
        "text/javascript" | "application/javascript" | "application/x-javascript" => "js",
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/x-icon" | "image/vnd.microsoft.icon" => "ico",
        "application/pdf" => "pdf",
        "application/wasm" => "wasm",
        "application/zip" => "zip",
        _ if mime.ends_with("+json") => "json",
        _ if mime.ends_with("+xml") => "xml",
        _ if mime.starts_with("text/") => "txt",
        _ => "bin",
    }
}

fn flow_body_reply(hub: &Hub, id: u64, redact: &crate::share::RedactOptions) -> Reply {
    let Some(mut flow) = hub.with_session(|session| session.hydrated(id)) else {
        return json_reply(404, "Not Found", &json!({"error": "not found"}));
    };
    if redact.enabled {
        let (sanitized, _) = crate::share::redact_flow(&flow, redact);
        flow = sanitized;
    }
    let raw_ct = crate::present::content_type(&flow.response_headers)
        .unwrap_or("")
        .replace(['\r', '\n'], "")
        .trim()
        .to_string();
    let content_type = if raw_ct.is_empty() {
        std::borrow::Cow::Borrowed("application/octet-stream")
    } else {
        std::borrow::Cow::Owned(raw_ct)
    };
    let ext = body_extension_for_mime(&content_type);
    Reply {
        status: 200,
        reason: "OK",
        content_type,
        attachment: Some(format!("flow-{id}.{ext}")),
        body: flow.response_body,
    }
}

fn export_har_reply(hub: &Hub, filter: &str, redact: &crate::share::RedactOptions) -> Reply {
    let value = hub.export_har_matching_with_redact(filter, redact);
    let body = serde_json::to_vec_pretty(&value).unwrap_or_else(|_| b"{}".to_vec());
    let secs = crate::clock::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Reply {
        status: 200,
        reason: "OK",
        content_type: std::borrow::Cow::Borrowed("application/json; charset=utf-8"),
        attachment: Some(format!("zniff-{secs}.har")),
        body,
    }
}

fn export_html_reply(
    hub: &Hub,
    filter: &str,
    redact: &crate::share::RedactOptions,
    zh: bool,
) -> Reply {
    let (matched, _) = hub.matching_ids(filter);
    let flows = hub.with_session(|session| {
        matched
            .into_iter()
            .filter_map(|(id, _)| session.hydrated(id))
            .collect::<Vec<_>>()
    });
    let doc = crate::share::document_with_redact(&flows, crate::clock::now(), filter, redact);
    let page_html = crate::share::page(&doc, "", zh);
    let secs = crate::clock::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Reply {
        status: 200,
        reason: "OK",
        content_type: std::borrow::Cow::Borrowed("text/html; charset=utf-8"),
        attachment: Some(format!("zniff-{secs}.html")),
        body: page_html.into_bytes(),
    }
}

fn pairs(headers: &[(String, String)]) -> Value {
    Value::Array(
        headers
            .iter()
            .map(|(name, value)| json!([name, value]))
            .collect(),
    )
}

fn preview(bytes: &[u8]) -> (String, Option<String>, bool, bool) {
    let truncated = bytes.len() > PREVIEW_CAP;
    let slice = if truncated {
        &bytes[..PREVIEW_CAP]
    } else {
        bytes
    };
    match std::str::from_utf8(slice) {
        Ok(valid) => (valid.to_string(), None, truncated, false),
        Err(err) if truncated && err.error_len().is_none() => {
            let valid = &slice[..err.valid_up_to()];
            (
                String::from_utf8_lossy(valid).into_owned(),
                None,
                true,
                false,
            )
        }
        Err(_) => (
            String::from_utf8_lossy(slice).into_owned(),
            Some(session::base64_encode(slice)),
            truncated,
            true,
        ),
    }
}

fn page(zh: bool) -> String {
    let dict = if zh {
        r#"{"start":"开始","stop":"停止","clear":"清空","closed":"抓包端口未打开","listening":"正在抓包","filter":"过滤","empty":"没有流量","detail":"选择一条流量"}"#
    } else {
        r#"{"start":"Start","stop":"Stop","clear":"Clear","closed":"Capture port is closed","listening":"Capturing","filter":"Filter","empty":"No flows","detail":"Select a flow"}"#
    };
    PAGE.replace("/*DICT*/", dict)
}

const PAGE: &str = r##"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>zniff</title>
<style>
  :root { color-scheme: dark; }
  body { margin: 0; background: #0d1017; color: #f1f5f9; font: 15px/1.45 -apple-system, BlinkMacSystemFont, sans-serif; }
  header, main { padding: 0.8rem 1rem; }
  header { display: flex; gap: 0.6rem; align-items: center; background: #131822; border-bottom: 1px solid #222938; }
  h1 { font-size: 1.05rem; margin: 0 0.4rem 0 0; }
  button { border: 0; border-radius: 8px; padding: 0.45rem 0.8rem; font-weight: 650; color: #fff; background: #2563eb; cursor: pointer; }
  button.stop { background: #dc2626; }
  button.quiet { background: #1c2331; color: #cbd5e1; }
  #state { color: #cbd5e1; }
  input { flex: 1; min-width: 8rem; background: #0f131b; color: #f1f5f9; border: 1px solid #2b3448; border-radius: 8px; padding: 0.4rem 0.55rem; }
  main { display: grid; grid-template-columns: minmax(16rem, 1.1fr) minmax(16rem, 1fr); gap: 0.8rem; }
  section { background: #131822; border: 1px solid #222938; border-radius: 10px; overflow: auto; max-height: calc(100vh - 6.5rem); }
  table { width: 100%; border-collapse: collapse; }
  th, td { text-align: left; padding: 0.35rem 0.5rem; border-bottom: 1px solid #19202e; font-size: 13px; }
  td.mono, th { font-family: ui-monospace, Menlo, monospace; }
  tr { cursor: pointer; }
  tr:hover, tr.on { background: #172554; }
  pre { margin: 0; padding: 0.8rem; white-space: pre-wrap; word-break: break-word; font: 12.5px/1.45 ui-monospace, Menlo, monospace; }
  @media (max-width: 800px) { main { grid-template-columns: 1fr; } }
</style>
</head>
<body>
<header>
  <h1>zniff</h1>
  <button id="toggle" type="button"></button>
  <button id="clear" class="quiet" type="button"></button>
  <input id="filter" type="text" autocomplete="off">
  <span id="state"></span>
</header>
<main>
  <section><table><tbody id="rows"></tbody></table></section>
  <section><pre id="detail"></pre></section>
</main>
<script>
const I = /*DICT*/;
const stateEl = document.querySelector("#state");
const rowsEl = document.querySelector("#rows");
const detailEl = document.querySelector("#detail");
const filterEl = document.querySelector("#filter");
const toggle = document.querySelector("#toggle");
const clearBtn = document.querySelector("#clear");
filterEl.placeholder = I.filter;
clearBtn.textContent = I.clear;
detailEl.textContent = I.detail;
let selected = null;
function cell(row, text, mono) {
  const td = document.createElement("td");
  if (mono) td.className = "mono";
  td.textContent = text;
  row.appendChild(td);
}
function paint(flows) {
  rowsEl.replaceChildren();
  if (!flows.length) {
    const tr = document.createElement("tr");
    cell(tr, I.empty, false);
    rowsEl.appendChild(tr);
    return;
  }
  for (const flow of flows) {
    const tr = document.createElement("tr");
    if (flow.id === selected) tr.className = "on";
    cell(tr, flow.method || "", true);
    cell(tr, flow.status == null ? (flow.error ? "ERR" : "—") : String(flow.status), true);
    cell(tr, flow.host || "", true);
    cell(tr, flow.path || "", true);
    cell(tr, flow.relay ? ("relay " + flow.relay) : "", true);
    tr.onclick = () => { selected = flow.id; load(flow.id); };
    rowsEl.appendChild(tr);
  }
}
async function load(id) {
  const res = await fetch("/api/flows/" + id);
  if (!res.ok) { detailEl.textContent = I.detail; return; }
  const flow = await res.json();
  const lines = [
    flow.method + " " + (flow.url || flow.path || ""),
    "status " + (flow.status == null ? "—" : flow.status),
    flow.relay ? ("relay " + flow.relay) : "",
    "",
    flow.request_body || "",
    "",
    flow.response_body || ""
  ];
  detailEl.textContent = lines.filter((line, index) => line !== "" || index > 2).join("\n");
}
async function tick() {
  const state = await fetch("/api/state").then((res) => res.json());
  toggle.textContent = state.capturing ? I.stop : I.start;
  toggle.className = state.capturing ? "stop" : "";
  toggle.dataset.on = state.capturing ? "0" : "1";
  stateEl.textContent = (state.capturing ? (I.listening + " " + state.listen) : (I.closed + " " + state.listen_port)) + (state.error ? (" — " + state.error) : "");
  const list = await fetch("/api/flows?filter=" + encodeURIComponent(filterEl.value || "")).then((res) => res.json());
  paint(list.flows || []);
}
toggle.onclick = async () => {
  await fetch("/api/capture", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ on: toggle.dataset.on === "1" })
  });
  await tick();
};
clearBtn.onclick = async () => {
  await fetch("/api/clear", { method: "POST" });
  selected = null;
  detailEl.textContent = I.detail;
  await tick();
};
filterEl.oninput = () => { tick(); };
tick();
setInterval(tick, 1000);
</script>
</body>
</html>
"##;

fn html(status: u16, reason: &'static str, body: Vec<u8>) -> Reply {
    Reply {
        status,
        reason,
        content_type: std::borrow::Cow::Borrowed("text/html; charset=utf-8"),
        attachment: None,
        body,
    }
}

fn text(status: u16, reason: &'static str, message: &str) -> Reply {
    Reply {
        status,
        reason,
        content_type: std::borrow::Cow::Borrowed("text/plain; charset=utf-8"),
        attachment: None,
        body: message.as_bytes().to_vec(),
    }
}

fn json_reply(status: u16, reason: &'static str, value: &Value) -> Reply {
    Reply {
        status,
        reason,
        content_type: std::borrow::Cow::Borrowed("application/json; charset=utf-8"),
        attachment: None,
        body: serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec()),
    }
}

fn encode(reply: &Reply, is_head: bool) -> Vec<u8> {
    let disposition = reply
        .attachment
        .as_deref()
        .map(|name| format!("Content-Disposition: attachment; filename=\"{name}\"\r\n"))
        .unwrap_or_default();
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\n{disposition}Content-Length: {len}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        status = reply.status,
        reason = reply.reason,
        content_type = reply.content_type,
        len = reply.body.len(),
    );
    let mut bytes = head.into_bytes();
    if !is_head {
        bytes.extend_from_slice(&reply.body);
    }
    bytes
}

fn header<'a>(head: &'a Head, name: &str) -> Option<&'a str> {
    head.headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn query_value_opt(target: &str, key: &str) -> Option<String> {
    let query = target.split_once('?').map(|(_, query)| query)?;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        if name == key {
            return Some(decode(value));
        }
    }
    None
}

fn query_value(target: &str, key: &str) -> String {
    query_value_opt(target, key).unwrap_or_default()
}

fn decode(value: &str) -> String {
    let value = value.replace('+', " ");
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = &value[index + 1..index + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_head(bytes: &[u8]) -> std::result::Result<Head, ()> {
    let text = std::str::from_utf8(bytes).map_err(|_| ())?;
    let mut lines = text.split("\r\n");
    let start = lines.next().unwrap_or("").trim();
    if start.is_empty() {
        return Err(());
    }
    let mut parts = start.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    if method.is_empty() {
        return Err(());
    }
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }
    Ok(Head {
        method,
        target,
        headers,
    })
}

async fn read_header(socket: &mut TcpStream) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut tmp = [0_u8; 2048];
    loop {
        if let Some(end) = header_end(&buf) {
            let rest = buf.split_off(end);
            return Ok((buf, rest));
        }
        if buf.len() > HEADER_CAP {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "header too large",
            ));
        }
        let count = tokio::time::timeout(IO_TIMEOUT, socket.read(&mut tmp))
            .await
            .map_err(|_| std::io::Error::new(ErrorKind::TimedOut, "header timeout"))??;
        if count == 0 {
            return Err(std::io::Error::new(
                ErrorKind::UnexpectedEof,
                "header ended",
            ));
        }
        buf.extend_from_slice(&tmp[..count]);
    }
}

fn header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

async fn read_body(
    socket: &mut TcpStream,
    head: &Head,
    rest: &[u8],
    cap: usize,
) -> std::result::Result<Vec<u8>, Reply> {
    if head.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
    }) {
        return Err(text(
            411,
            "Length Required",
            "chunked bodies are not accepted",
        ));
    }
    let Some(raw) = header(head, "content-length") else {
        return Err(text(411, "Length Required", "content-length required"));
    };
    let len: usize = match raw.parse() {
        Ok(len) => len,
        Err(_) => return Err(text(400, "Bad Request", "bad content-length")),
    };
    if len > cap {
        return Err(text(413, "Content Too Large", "body is too large"));
    }
    let mut body = Vec::with_capacity(len);
    let take = rest.len().min(len);
    body.extend_from_slice(&rest[..take]);
    while body.len() < len {
        let mut buf = [0_u8; 1024];
        let read = match tokio::time::timeout(IO_TIMEOUT, socket.read(&mut buf)).await {
            Ok(Ok(0)) => return Err(text(400, "Bad Request", "body ended early")),
            Ok(Ok(count)) => count,
            _ => return Err(text(400, "Bad Request", "body read failed")),
        };
        let need = len - body.len();
        body.extend_from_slice(&buf[..read.min(need)]);
    }
    Ok(body)
}

async fn wait_shutdown(flag: &AtomicBool) {
    while !flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn lock(slot: &Mutex<Slot>) -> std::sync::MutexGuard<'_, Slot> {
    slot.lock().unwrap_or_else(|err| err.into_inner())
}

fn parse_port(text: &str) -> std::result::Result<u16, String> {
    crate::present::parse_port(text).ok_or_else(|| "端口无效".to_string())
}

fn parse_bind(text: &str) -> std::result::Result<IpAddr, String> {
    text.parse()
        .map_err(|_| "监听地址要写成 0.0.0.0 或 127.0.0.1".to_string())
}

/// Directory of the packaged nsh page. `ZNIFF_WEB_ROOT` wins. Otherwise look
/// for `app/.nsh/web` from the repo root, or `.nsh/web` when the cwd is `app/`.
fn packaged_web_root() -> Option<PathBuf> {
    let env = std::env::var_os("ZNIFF_WEB_ROOT").map(PathBuf::from);
    let cwd = std::env::current_dir().ok();
    choose_web_root(env.as_deref(), cwd.as_deref())
}

fn choose_web_root(env: Option<&Path>, cwd: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = env.filter(|dir| dir.join("index.html").is_file()) {
        return Some(dir.to_path_buf());
    }
    let cwd = cwd?;
    for rel in ["app/.nsh/web", ".nsh/web"] {
        let dir = cwd.join(rel);
        if dir.join("index.html").is_file() {
            return Some(dir);
        }
    }
    None
}

/// The packaged nsh page. `None` keeps the hand-written console, so tests do
/// not serve a developer package that happens to sit on disk.
fn static_reply(path: &str, root: Option<&Path>) -> Option<Reply> {
    let root = root?;
    let rel = path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if rel.split(['/', '\\']).any(|part| part == "..") {
        return Some(text(400, "Bad Request", "bad path"));
    }
    let root_canon = root.canonicalize().ok()?;
    let full_canon = root.join(rel).canonicalize().ok()?;
    if !full_canon.starts_with(&root_canon) {
        return Some(text(403, "Forbidden", "bad path"));
    }
    let bytes = std::fs::read(&full_canon).ok()?;
    Some(Reply {
        status: 200,
        reason: "OK",
        content_type: std::borrow::Cow::Borrowed(static_mime(&full_canon)),
        attachment: None,
        body: bytes,
    })
}

fn static_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        _ => "application/octet-stream",
    }
}

async fn hub_dispatch(slot: &Mutex<Slot>, head: &Head, value: Value) -> Reply {
    let op = value
        .get("op")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if matches!(op.as_str(), "replay" | "replay_batch" | "send_composed") {
        let hub = Arc::clone(&lock(slot).hub);
        return replay_dispatch(hub, &op, &value).await;
    }
    let mut guard = lock(slot);
    if op == "begin_share" {
        return begin_share_op(&mut guard, head, &value);
    }
    sync_hub_op(&mut guard, &op, &value)
}

fn persist_slot_config(slot: &Slot, change: impl FnOnce(&mut crate::config::Config)) {
    if let Some(dir) = &slot.prefs.config_dir {
        let _ = crate::config::update_mode(dir, slot.prefs.dev_mode, change);
    }
}

fn sync_hub_op(slot: &mut Slot, op: &str, value: &Value) -> Reply {
    match op {
        "set_bypass" => {
            let text = json_text(value);
            slot.hub.set_bypass(&text);
            persist_slot_config(slot, |cfg| cfg.bypass_hosts = text);
        }
        "set_rules" => {
            let text = json_text(value);
            slot.hub.set_rules(&text);
            persist_slot_config(slot, |cfg| cfg.rules = text);
        }
        "set_upstream" => {
            let text = json_text(value);
            slot.hub.set_upstream(&text);
            persist_slot_config(slot, |cfg| cfg.upstream_proxy = text);
        }
        "set_relay_token" => {
            let text = json_text(value);
            slot.hub.set_relay_token(&text);
            let trimmed = text.trim().to_string();
            persist_slot_config(slot, |cfg| cfg.relay_token = trimmed);
        }
        "set_relay_url" => {
            let text = json_text(value).trim().to_string();
            slot.prefs.relay_url = text.clone();
            persist_slot_config(slot, |cfg| cfg.relay_url = text);
        }
        "set_filter" => {
            let text = json_text(value);
            slot.prefs.filter = text.clone();
            persist_slot_config(slot, |cfg| cfg.filter = text);
        }
        "set_active_profile" => {
            let name = json_text(value).trim().to_string();
            slot.prefs.active_profile = name.clone();
            persist_slot_config(slot, |cfg| cfg.active_profile = name);
        }
        "select_profile" => {
            let name = value
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| json_text(value));
            let trimmed = name.trim();
            if let Some(found) = slot
                .prefs
                .saved_profiles
                .iter()
                .find(|item| item.name.eq_ignore_ascii_case(trimmed))
                .cloned()
            {
                slot.prefs.active_profile = found.name.clone();
                slot.prefs.filter = found.filter.clone();
                slot.hub.set_bypass(&found.bypass_hosts);
                slot.hub.set_rules(&found.rules);
                slot.hub.set_upstream(&found.upstream_proxy);
                slot.hub.set_rules_enabled(found.rules_enabled);
                slot.hub.set_breaks_enabled(found.breaks_enabled);
                slot.hub
                    .set_show_tunnel_failures(found.show_tunnel_failures);
                persist_slot_config(slot, |cfg| {
                    cfg.active_profile = found.name;
                    cfg.filter = found.filter;
                    cfg.bypass_hosts = found.bypass_hosts;
                    cfg.rules = found.rules;
                    cfg.upstream_proxy = found.upstream_proxy;
                    cfg.rules_enabled = found.rules_enabled;
                    cfg.breaks_enabled = found.breaks_enabled;
                    cfg.show_tunnel_failures = found.show_tunnel_failures;
                });
            } else if trimmed.is_empty() {
                slot.prefs.active_profile.clear();
                persist_slot_config(slot, |cfg| cfg.active_profile.clear());
            } else {
                return json_reply(404, "Not Found", &json!({"error": "profile not found"}));
            }
        }
        "delete_profile" => {
            let name = value
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| json_text(value));
            let trimmed = name.trim();
            slot.prefs
                .saved_profiles
                .retain(|item| !item.name.eq_ignore_ascii_case(trimmed));
            if slot.prefs.active_profile.eq_ignore_ascii_case(trimmed) {
                slot.prefs.active_profile.clear();
            }
            let saved_next = slot.prefs.saved_profiles.clone();
            let active_next = slot.prefs.active_profile.clone();
            persist_slot_config(slot, |cfg| {
                cfg.saved_profiles = saved_next;
                cfg.active_profile = active_next;
            });
        }
        "sync_profiles" => {
            if let Some(raw_saved) = value.get("saved_profiles")
                && let Ok(mut saved) =
                    serde_json::from_value::<Vec<crate::config::SavedProfile>>(raw_saved.clone())
            {
                saved.truncate(16);
                slot.prefs.saved_profiles = saved;
            }
            if let Some(raw_hist) = value.get("profile_history")
                && let Ok(mut hist) =
                    serde_json::from_value::<Vec<crate::config::SavedProfile>>(raw_hist.clone())
            {
                hist.truncate(10);
                slot.prefs.profile_history = hist;
            }
            if let Some(active) = value.get("active_profile").and_then(Value::as_str) {
                slot.prefs.active_profile = active.trim().to_string();
            }
            let saved_next = slot.prefs.saved_profiles.clone();
            let hist_next = slot.prefs.profile_history.clone();
            let active_next = slot.prefs.active_profile.clone();
            persist_slot_config(slot, |cfg| {
                cfg.saved_profiles = saved_next;
                cfg.profile_history = hist_next;
                cfg.active_profile = active_next;
            });
        }
        "set_redact" => {
            if let Some(on) = json_flag(value) {
                slot.prefs.redact_export = on;
            }
            if let Some(strict) = value.get("strict").and_then(Value::as_bool) {
                slot.prefs.redact_strict = strict;
            }
            if let Some(fields) = value
                .get("fields")
                .or_else(|| value.get("text"))
                .and_then(Value::as_str)
            {
                slot.prefs.redact_fields = fields.to_string();
            }
            let on = slot.prefs.redact_export;
            let strict = slot.prefs.redact_strict;
            let fields = slot.prefs.redact_fields.clone();
            persist_slot_config(slot, |cfg| {
                cfg.redact_export = on;
                cfg.redact_strict = strict;
                cfg.redact_fields = fields;
            });
        }
        "set_rules_enabled" => {
            let Some(on) = json_flag(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need on"}));
            };
            slot.hub.set_rules_enabled(on);
            persist_slot_config(slot, |cfg| cfg.rules_enabled = on);
        }
        "set_breaks_enabled" => {
            let Some(on) = json_flag(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need on"}));
            };
            slot.hub.set_breaks_enabled(on);
            persist_slot_config(slot, |cfg| cfg.breaks_enabled = on);
        }
        "set_show_tunnel_failures" => {
            let Some(on) = json_flag(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need on"}));
            };
            slot.hub.set_show_tunnel_failures(on);
            persist_slot_config(slot, |cfg| cfg.show_tunnel_failures = on);
        }
        "set_relay_accept" => {
            let Some(on) = json_flag(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need on"}));
            };
            slot.hub.set_relay_accept(on);
            persist_slot_config(slot, |cfg| cfg.relay_accept = on);
        }
        "set_pinned" => {
            let Some(id) = json_id(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need id"}));
            };
            let Some(on) = json_flag(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need on"}));
            };
            slot.hub.set_pinned(id, on);
        }
        "set_comment" => {
            let Some(id) = json_id(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need id"}));
            };
            slot.hub.set_comment(id, json_text(value));
        }
        "clear" => slot.hub.clear_session(),
        "clear_unpinned" => {
            slot.hub.clear_unpinned();
        }
        "resolve_break" => {
            slot.hub.resolve_break(BreakDecision {
                drop: value.get("drop").and_then(Value::as_bool).unwrap_or(false),
                headers: json_headers(value.get("headers")),
                body: json_body(value),
                status: value
                    .get("status")
                    .and_then(Value::as_u64)
                    .and_then(|code| u16::try_from(code).ok()),
            });
        }
        "resolve_all_breaks" => {
            slot.hub.resolve_all_breaks();
        }
        "promote_break" => {
            let Some(id) = json_id(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need id"}));
            };
            slot.hub.promote_break(id);
        }
        "touch_break" => slot.hub.touch_break(),
        "end_share" => {
            if let Some(handle) = slot.relay_push.take() {
                handle.stop();
            }
            slot.hub.end_share();
        }
        "import_har" => {
            let document = value.get("value").unwrap_or(&Value::Null);
            slot.hub.import_har(document);
        }
        _ => return json_reply(400, "Bad Request", &json!({"error": "unknown op"})),
    }
    json_reply(200, "OK", &state_value(slot))
}

fn begin_share_op(slot: &mut Slot, head: &Head, value: &Value) -> Reply {
    if slot.capture.is_none() {
        return json_reply(409, "Conflict", &json!({"error": "需要先开始抓包"}));
    }
    let minutes = value
        .get("minutes")
        .and_then(Value::as_u64)
        .unwrap_or(15)
        .clamp(1, 480);
    let filter = value.get("filter").and_then(Value::as_str).unwrap_or("");
    let strict = value
        .get("redact_strict")
        .and_then(Value::as_bool)
        .unwrap_or(slot.prefs.redact_strict);
    let redact = crate::share::RedactOptions::new(
        value
            .get("redact")
            .and_then(Value::as_bool)
            .unwrap_or(slot.prefs.redact_export),
        value
            .get("redact_fields")
            .and_then(Value::as_str)
            .unwrap_or(&slot.prefs.redact_fields),
    )
    .with_strict(strict);
    let relay_url = value.get("relay_url").and_then(Value::as_str).unwrap_or("");
    let relay_token = value
        .get("relay_token")
        .and_then(Value::as_str)
        .unwrap_or("");
    let port = slot
        .capture
        .as_ref()
        .map(|capture| capture.addr.port())
        .unwrap_or(slot.capture_port);
    let host = share_host(header(head, "host").unwrap_or(""));
    let local_base = format!("http://{host}:{port}");
    let local_only = !slot.capture_ip.is_unspecified();
    let route = match crate::relay::share_route(relay_url, relay_token, &local_base, local_only) {
        Ok(route) => route,
        Err(crate::relay::ShareRouteError::BadUrl) => {
            return json_reply(400, "Bad Request", &json!({"error": "中继地址无效"}));
        }
        Err(crate::relay::ShareRouteError::EmptyToken) => {
            return json_reply(400, "Bad Request", &json!({"error": "中继需要口令"}));
        }
    };
    if let Some(handle) = slot.relay_push.take() {
        handle.stop();
    }
    let (id, _expires) = slot.hub.begin_share_with_redact(
        Duration::from_secs(minutes.saturating_mul(60)),
        filter,
        redact,
    );
    let url = match route {
        crate::relay::ShareRoute::Local { base, .. } => format!("{base}/s/{id}"),
        crate::relay::ShareRoute::Relay(base) => {
            let url = base.share_url(&id);
            let handle = crate::relay::spawn_push(
                Arc::clone(&slot.hub),
                id.clone(),
                base,
                relay_token.trim().to_string(),
                |_| {},
            );
            crate::relay::remember_push(handle.clone());
            slot.relay_push = Some(handle);
            url
        }
    };
    json_reply(200, "OK", &json!({"ok": true, "id": id, "url": url}))
}

fn share_host(raw: &str) -> String {
    let host = if let Some(inner) = raw.strip_prefix('[') {
        inner.split(']').next().unwrap_or("")
    } else {
        raw.split(':').next().unwrap_or("")
    };
    if host.is_empty() || host == "0.0.0.0" {
        "127.0.0.1".to_string()
    } else {
        host.to_string()
    }
}

async fn replay_dispatch(hub: Arc<Hub>, op: &str, value: &Value) -> Reply {
    let (tx, rx) = tokio::sync::oneshot::channel();
    match op {
        "replay" => {
            let Some(id) = json_id(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need id"}));
            };
            let edit = override_from(value);
            hub.replay_later(id, edit, move |result| {
                let _ = tx.send(match result {
                    Ok(id) => json!({"id": id}),
                    Err(err) => json!({"error": err.to_string()}),
                });
            });
        }
        "replay_batch" => {
            let Some(id) = json_id(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need id"}));
            };
            let count = json_u32(value, "count", 1);
            let concurrency = json_u32(value, "concurrency", 1);
            let interval_ms = value
                .get("interval_ms")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let edit = override_from(value);
            hub.replay_batch_later(id, count, concurrency, interval_ms, edit, move |result| {
                let _ = tx.send(match result {
                    Ok(ids) => json!({"ids": ids}),
                    Err(err) => json!({"error": err.to_string()}),
                });
            });
        }
        "send_composed" => {
            let edit = override_from(value);
            hub.send_composed_later(edit, move |result| {
                let _ = tx.send(match result {
                    Ok(id) => json!({"id": id}),
                    Err(err) => json!({"error": err.to_string()}),
                });
            });
        }
        _ => return json_reply(400, "Bad Request", &json!({"error": "unknown op"})),
    }
    match tokio::time::timeout(Duration::from_secs(30), rx).await {
        Ok(Ok(value)) => json_reply(200, "OK", &value),
        _ => json_reply(200, "OK", &json!({"error": "重放超时"})),
    }
}

fn override_from(value: &Value) -> ReplayOverride {
    ReplayOverride {
        method: value
            .get("method")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_string),
        url: value
            .get("url")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_string),
        headers: json_headers(value.get("headers")),
        body: json_body(value),
    }
}

fn json_body(value: &Value) -> Option<Vec<u8>> {
    if let Some(b64) = value.get("body_b64").and_then(Value::as_str)
        && let Some(bytes) = session::base64_decode(b64)
    {
        return Some(bytes);
    }
    value
        .get("body")
        .and_then(Value::as_str)
        .map(|text| text.as_bytes().to_vec())
}

fn json_text(value: &Value) -> String {
    value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn json_flag(value: &Value) -> Option<bool> {
    value.get("on").and_then(Value::as_bool)
}

fn json_id(value: &Value) -> Option<u64> {
    value.get("id").and_then(Value::as_u64)
}

fn json_u32(value: &Value, key: &str, default: u32) -> u32 {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|number| u32::try_from(number).ok())
        .unwrap_or(default)
}

fn json_headers(value: Option<&Value>) -> Option<Vec<(String, String)>> {
    let items = value?.as_array()?;
    Some(
        items
            .iter()
            .filter_map(|item| {
                let pair = item.as_array()?;
                Some((
                    pair.first()?.as_str()?.to_string(),
                    pair.get(1)?.as_str()?.to_string(),
                ))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Protocol, RelayDraft};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_string()).collect()
    }

    #[test]
    fn web_settings_keep_capture_closed_until_a_later_start() {
        let settings = cli_settings(&args(&["--web"]), 8877).unwrap();
        assert_eq!(settings.web_port, 8080);
        assert_eq!(settings.web_bind, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(settings.capture_port, 8877);
        assert_eq!(settings.capture_bind, IpAddr::V4(Ipv4Addr::LOCALHOST));
        let docker = cli_settings(
            &args(&[
                "--web",
                "--web-bind",
                "0.0.0.0",
                "--bind",
                "0.0.0.0",
                "--port",
                "8877",
            ]),
            9,
        )
        .unwrap();
        assert_eq!(docker.web_bind, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        assert_eq!(docker.capture_bind, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        assert_eq!(docker.capture_port, 8877);
        assert!(cli_settings(&args(&["--web", "--port", "0"]), 8877).is_err());
        assert!(cli_settings(&args(&["--web", "--web-port", "0"]), 8877).is_err());
    }

    #[test]
    fn choose_web_root_uses_env_then_repo_then_app_dir() {
        let root = std::env::temp_dir().join(format!("zniff-web-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let env_dir = root.join("env");
        let repo = root.join("repo");
        let app = root.join("appdir");
        let empty = root.join("empty");
        std::fs::create_dir_all(&env_dir).unwrap();
        std::fs::create_dir_all(repo.join("app/.nsh/web")).unwrap();
        std::fs::create_dir_all(app.join(".nsh/web")).unwrap();
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::write(env_dir.join("index.html"), b"e").unwrap();
        std::fs::write(repo.join("app/.nsh/web/index.html"), b"r").unwrap();
        std::fs::write(app.join(".nsh/web/index.html"), b"a").unwrap();
        assert_eq!(
            choose_web_root(Some(&env_dir), Some(&repo)).unwrap(),
            env_dir
        );
        assert_eq!(
            choose_web_root(Some(&empty), Some(&repo)).unwrap(),
            repo.join("app/.nsh/web")
        );
        assert_eq!(
            choose_web_root(None, Some(&app)).unwrap(),
            app.join(".nsh/web")
        );
        assert!(choose_web_root(None, Some(&root)).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn packaged_root_replaces_the_small_console() {
        let dir = std::env::temp_dir().join(format!("zniff-web-shared-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), b"<!doctype html>shared-ui").unwrap();
        let ca_dir = dir.join("ca");
        std::fs::create_dir_all(&ca_dir).unwrap();
        let ca = Arc::new(Ca::open(&ca_dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let server = WebServer::start_with(
            "127.0.0.1:0".parse().unwrap(),
            hub,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            reserve_port(),
            Some(dir.clone()),
        )
        .unwrap();
        let (status, page) = exchange(
            server.addr(),
            "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 200);
        assert!(page.contains("shared-ui"), "{page}");
        assert!(!page.contains("开始"), "{page}");
        drop(server);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn console_stays_up_and_capture_port_opens_only_while_started() {
        let dir = std::env::temp_dir().join(format!("zniff-web-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let capture_port = reserve_port();
        let server = WebServer::start(
            "127.0.0.1:0".parse().unwrap(),
            hub.clone(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            capture_port,
        )
        .unwrap();
        let web = server.addr();
        let capture = SocketAddr::from((Ipv4Addr::LOCALHOST, capture_port));

        let (status, page) = exchange(
            web,
            "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 200);
        assert!(page.contains("开始"), "{page}");
        assert!(page.contains("/api/capture"), "{page}");
        assert!(!page.contains("zniff-ca.crt"), "{page}");
        let (status, english) = exchange(
            web,
            "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept-Language: en\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 200);
        assert!(english.contains("Start"), "{english}");

        let (status, state) = exchange(
            web,
            "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 200);
        assert!(state.contains("\"capturing\":false"), "{state}");
        assert!(refused(capture).await);

        let (status, _) = exchange(
            web,
            "CONNECT beta.test:443 HTTP/1.1\r\nHost: beta.test:443\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 403);
        let (status, _) = exchange(
            web,
            "GET http://beta.test/secret HTTP/1.1\r\nHost: beta.test\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 403);
        let (status, text) = exchange(
            web,
            "POST /api/capture HTTP/1.1\r\nHost: 127.0.0.1\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 411, "{text}");
        assert_eq!(hub.with_session(|session| session.len()), 0);
        let (status, empty) = exchange(
            web,
            "GET /api/flows HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 200, "{empty}");
        let empty_json: Value = serde_json::from_str(body_of(&empty)).unwrap();
        assert_eq!(empty_json["total"], 0, "{empty}");
        assert_eq!(
            empty_json["flows"].as_array().map(Vec::len),
            Some(0),
            "{empty}"
        );

        let (status, started) = post(web, "/api/capture", r#"{"on":true}"#).await;
        assert_eq!(status, 200, "{started}");
        assert!(started.contains("\"capturing\":true"), "{started}");
        assert!(started.contains(&capture.to_string()), "{started}");

        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = origin.accept().await.unwrap();
            let mut buf = vec![0_u8; 4096];
            let _ = sock.read(&mut buf).await;
            let body = b"hello-web";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.write_all(body).await;
        });
        let (status, proxied) = exchange(
            capture,
            &format!(
                "GET http://{origin_addr}/hi HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert_eq!(status, 200, "{proxied}");
        assert!(proxied.contains("hello-web"), "{proxied}");

        let (status, flows) = exchange(
            web,
            "GET /api/flows?filter=path:/hi HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 200, "{flows}");
        let listed: Value = serde_json::from_str(body_of(&flows)).unwrap();
        assert_eq!(listed["total"], 1, "{flows}");
        let id = listed["flows"][0]["id"].as_u64().unwrap();
        assert_eq!(listed["flows"][0]["path"], "/hi");
        let (status, detail) = exchange(
            web,
            &format!(
                "GET /api/flows/{id} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert_eq!(status, 200, "{detail}");
        assert!(body_of(&detail).contains("hello-web"), "{detail}");

        let (status, install) = exchange(
            capture,
            "GET /zniff HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 200, "{install}");
        assert!(install.contains("zniff-ca.crt"), "{install}");

        let share = "550e8400-e29b-41d4-a716-446655440000";
        hub.with_session(|session| {
            session.sync_relay_share(
                share,
                vec![RelayDraft {
                    remote_id: 3,
                    protocol: Protocol::Http11,
                    method: "GET".into(),
                    host: "beta.test".into(),
                    path: "/relayed".into(),
                    url: "http://beta.test/relayed".into(),
                    status: Some(200),
                    error: None,
                    request_headers: Vec::new(),
                    response_headers: Vec::new(),
                    request_body: Vec::new(),
                    response_body: b"from-relay".to_vec(),
                    request_bytes: 0,
                    response_bytes: 10,
                    ttfb: None,
                    total: Some(Duration::from_millis(4)),
                    comment: String::new(),
                    rule_hit: String::new(),
                    messages: Vec::new(),
                }],
            );
        });
        let (status, relayed) = exchange(
            web,
            "GET /api/flows?filter=host:beta.test HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 200, "{relayed}");
        assert!(body_of(&relayed).contains("550e8400"), "{relayed}");
        assert!(!body_of(&relayed).contains("/hi"), "{relayed}");

        let (status, cleared) = post(web, "/api/clear", "{}").await;
        assert_eq!(status, 200, "{cleared}");
        assert!(cleared.contains("\"flows\":0"), "{cleared}");
        assert!(cleared.contains("\"capturing\":true"), "{cleared}");

        let (status, stopped) = post(web, "/api/capture", r#"{"on":false}"#).await;
        assert_eq!(status, 200, "{stopped}");
        assert!(stopped.contains("\"capturing\":false"), "{stopped}");
        assert!(refused(capture).await);
        let (status, _) = exchange(
            web,
            "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 200);

        let (status, again) = post(web, "/api/capture", r#"{"on":true}"#).await;
        assert_eq!(status, 200, "{again}");
        assert!(again.contains("\"capturing\":true"), "{again}");
        drop(server);
        assert!(refused(capture).await);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn capture_bind_failure_leaves_the_console_up() {
        let dir = std::env::temp_dir().join(format!("zniff-web-busy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = held.local_addr().unwrap().port();
        let server = WebServer::start(
            "127.0.0.1:0".parse().unwrap(),
            hub,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
        )
        .unwrap();
        let (status, body) = post(server.addr(), "/api/capture", r#"{"on":true}"#).await;
        assert_eq!(status, 409, "{body}");
        assert!(body.contains("\"capturing\":false"), "{body}");
        assert!(body.contains("\"error\":"), "{body}");
        assert!(!body.contains("\"error\":\"\""), "{body}");
        let (status, page) = exchange(
            server.addr(),
            "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 200, "{page}");
        drop(held);
        drop(server);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn hub_set_rules_stays_on_the_server_and_capture_stays_closed() {
        let dir = std::env::temp_dir().join(format!("zniff-web-hub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let capture_port = reserve_port();
        let server = WebServer::start(
            "127.0.0.1:0".parse().unwrap(),
            hub,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            capture_port,
        )
        .unwrap();
        let capture = SocketAddr::from((Ipv4Addr::LOCALHOST, capture_port));
        let (status, body) = post(
            server.addr(),
            "/api/hub",
            r#"{"op":"set_rules","text":"delay example.test / 1ms"}"#,
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert!(body.contains("delay example.test / 1ms"), "{body}");
        assert!(body.contains("\"capturing\":false"), "{body}");
        assert!(refused(capture).await);
        let (status, state) = exchange(
            server.addr(),
            "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 200, "{state}");
        assert!(
            body_of(&state).contains("delay example.test / 1ms"),
            "{state}"
        );
        let (status, unknown) = post(server.addr(), "/api/hub", r#"{"op":"nope"}"#).await;
        assert_eq!(status, 400, "{unknown}");
        drop(server);
        assert!(refused(capture).await);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn binary_bodies_metadata_exports_and_profile_prefs_roundtrip() {
        let dir = std::env::temp_dir().join(format!("zniff-web-parity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());

        let raw_req = vec![0x00, 0xff, 0x89, b'R', b'E', b'Q'];
        let raw_res = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0xff, 0x00];
        let flow_id = hub.with_session(|session| {
            let id = session.open(session::FlowDraft {
                protocol: Protocol::Http2,
                method: "POST".into(),
                host: "img.example.test".into(),
                path: "/icon.png".into(),
                url: "https://img.example.test/icon.png".into(),
                request_headers: vec![("Content-Type".into(), "application/octet-stream".into())],
            });
            session.set_conn_info(
                id,
                session::ConnInfo {
                    dns_ms: Some(3),
                    tcp_ms: Some(9),
                    tls_ms: Some(14),
                    reused: false,
                    server_addr: "93.184.216.34:443".into(),
                    tls_version: "TLSv1.3".into(),
                    dialed_before_start: true,
                },
            );
            session.set_rule_hit(id, "header");
            session.append_body(id, Direction::Request, &raw_req);
            session.set_response(
                id,
                200,
                vec![("Content-Type".into(), "image/png".into())],
                Some(Protocol::Http2),
            );
            session.append_body(id, Direction::Response, &raw_res);
            session.set_encoding_error(id, Direction::Response, "trailing garbage");
            session.mark_finished(id);
            let other = session.open(session::FlowDraft {
                protocol: Protocol::Http11,
                method: "GET".into(),
                host: "other.example.test".into(),
                path: "/ping".into(),
                url: "http://other.example.test/ping".into(),
                request_headers: vec![("Authorization".into(), "Bearer top-secret".into())],
            });
            session.set_response(other, 204, Vec::new(), None);
            session.mark_finished(other);
            id
        });

        let initial_cfg = crate::config::Config::default_for(false);
        let prefs = WebPrefs::from_loaded(&initial_cfg, dir.clone(), false);
        let server = WebServer::start_with_prefs(
            "127.0.0.1:0".parse().unwrap(),
            hub.clone(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            reserve_port(),
            None,
            prefs,
        )
        .unwrap();
        let web = server.addr();

        // Filtered `/api/flows` returns matched `total = 1` AND full `session_total = 2`,
        // plus `rule_hit`, `response_ms`, and `conn_info`.
        let (status, list_raw) = exchange(
            web,
            "GET /api/flows?filter=host:img.example.test HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 200, "{list_raw}");
        let listed: Value = serde_json::from_str(body_of(&list_raw)).unwrap();
        assert_eq!(listed["total"], 1);
        assert_eq!(listed["session_total"], 2);
        assert_eq!(listed["flows"][0]["rule_hit"], "header");
        assert!(listed["flows"][0]["response_ms"].as_u64().is_some());
        assert_eq!(listed["flows"][0]["conn_info"]["tls_version"], "TLSv1.3");
        assert_eq!(listed["flows"][0]["conn_info"]["dns_ms"], 3);

        // `/api/flows/{id}` preserves exact binary bytes via base64 and surfaces encoding error.
        let (status, detail_raw) = exchange(
            web,
            &format!(
                "GET /api/flows/{flow_id} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert_eq!(status, 200, "{detail_raw}");
        let detail: Value = serde_json::from_str(body_of(&detail_raw)).unwrap();
        assert_eq!(detail["request_binary"], true);
        assert_eq!(detail["response_binary"], true);
        assert_eq!(
            session::base64_decode(detail["request_body_b64"].as_str().unwrap()).unwrap(),
            raw_req
        );
        assert_eq!(
            session::base64_decode(detail["response_body_b64"].as_str().unwrap()).unwrap(),
            raw_res
        );
        assert_eq!(detail["response_encoding_error"], "trailing garbage");

        // `override_from` decodes binary `body_b64` losslessly.
        let b64_payload = session::base64_encode(&raw_res);
        let parsed_override = override_from(&json!({
            "method": "POST",
            "url": "http://127.0.0.1:9/bin",
            "body_b64": b64_payload,
        }));
        assert_eq!(parsed_override.body.as_deref(), Some(raw_res.as_slice()));

        // Profile sync, select, redact, and HAR/HTML export endpoints.
        let sync_req = json!({
            "op": "sync_profiles",
            "active_profile": "Staging",
            "saved_profiles": [{
                "name": "Staging",
                "filter": "host:img.example.test",
                "bypass_hosts": "internal.local",
                "rules": "delay img.example.test / 5ms",
                "upstream_proxy": "",
                "rules_enabled": true,
                "breaks_enabled": false,
                "show_tunnel_failures": true
            }]
        })
        .to_string();
        let (status, _) = post(web, "/api/hub", &sync_req).await;
        assert_eq!(status, 200);
        let (status, selected_state) = post(
            web,
            "/api/hub",
            r#"{"op":"select_profile","name":"staging"}"#,
        )
        .await;
        assert_eq!(status, 200, "{selected_state}");
        let state_json: Value = serde_json::from_str(body_of(&selected_state)).unwrap();
        assert_eq!(state_json["prefs"]["active_profile"], "Staging");
        assert_eq!(state_json["prefs"]["filter"], "host:img.example.test");
        assert_eq!(state_json["prefs"]["breaks_enabled"], false);
        assert_eq!(state_json["prefs"]["show_tunnels"], true);

        let (status, _) = post(
            web,
            "/api/hub",
            r#"{"op":"set_redact","on":true,"fields":"x-extra"}"#,
        )
        .await;
        assert_eq!(status, 200);

        let persisted = crate::config::load_mode(&dir, false);
        assert_eq!(persisted.active_profile, "Staging");
        assert_eq!(persisted.saved_profiles.len(), 1);
        assert!(persisted.redact_export);

        let (status, har_resp) = exchange(
            web,
            "GET /api/export.har HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 200);
        assert!(!har_resp.contains("top-secret"), "{har_resp}");
        assert!(har_resp.contains("\"entries\""), "{har_resp}");

        let (status, html_resp) = exchange(
            web,
            "GET /api/export.html?zh=0 HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 200);
        assert!(!html_resp.contains("top-secret"), "{html_resp}");

        drop(server);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn flow_body_download_reads_spilled_bytes_and_respects_redaction() {
        let dir = std::env::temp_dir().join(format!(
            "zniff-web-body-{}-{}",
            std::process::id(),
            crate::clock::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        // > 16 KB so `mark_finished` spills the unpinned response body to disk.
        let large_padding = "x".repeat(20 * 1024);
        let json_payload =
            format!(r#"{{"access_token":"tok_download_123","pad":"{large_padding}"}}"#);
        let bin_payload: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0xff];

        let (json_id, bin_id) = hub.with_session(|session| {
            let j_id = session.open(session::FlowDraft {
                protocol: Protocol::Http11,
                method: "GET".into(),
                host: "api.example.test".into(),
                path: "/v1/token".into(),
                url: "https://api.example.test/v1/token".into(),
                request_headers: Vec::new(),
            });
            session.set_response(
                j_id,
                200,
                vec![(
                    "Content-Type".into(),
                    "application/json; charset=utf-8".into(),
                )],
                None,
            );
            session.append_body(j_id, Direction::Response, json_payload.as_bytes());
            session.mark_finished(j_id);

            let b_id = session.open(session::FlowDraft {
                protocol: Protocol::Http11,
                method: "GET".into(),
                host: "cdn.example.test".into(),
                path: "/logo".into(),
                url: "https://cdn.example.test/logo".into(),
                request_headers: Vec::new(),
            });
            session.set_response(b_id, 200, Vec::new(), None);
            session.append_body(b_id, Direction::Response, &bin_payload);
            session.mark_finished(b_id);
            (j_id, b_id)
        });

        let initial_cfg = crate::config::Config::default_for(false);
        let prefs = WebPrefs::from_loaded(&initial_cfg, dir.clone(), false);
        let server = WebServer::start_with_prefs(
            "127.0.0.1:0".parse().unwrap(),
            hub,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            reserve_port(),
            None,
            prefs,
        )
        .unwrap();
        let web = server.addr();

        // 1. Spilled JSON body downloads intact with Content-Type and .json filename when redaction is off.
        let (status, resp_raw) = exchange(
            web,
            &format!(
                "GET /api/flows/{json_id}/body HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert_eq!(status, 200, "{resp_raw}");
        assert!(
            resp_raw.contains("Content-Type: application/json; charset=utf-8\r\n"),
            "{resp_raw}"
        );
        assert!(
            resp_raw.contains(&format!(
                "Content-Disposition: attachment; filename=\"flow-{json_id}.json\"\r\n"
            )),
            "{resp_raw}"
        );
        assert_eq!(body_of(&resp_raw), json_payload);

        // 2. Binary body without Content-Type falls back to application/octet-stream + .bin.
        let mut sock = TcpStream::connect(web).await.unwrap();
        sock.write_all(
            format!(
                "GET /api/flows/{bin_id}/body HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
        let mut raw_bytes = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), sock.read_to_end(&mut raw_bytes))
            .await
            .unwrap()
            .unwrap();
        let sep = header_end(&raw_bytes).unwrap();
        let head_str = std::str::from_utf8(&raw_bytes[..sep]).unwrap();
        assert!(head_str.contains("Content-Type: application/octet-stream\r\n"));
        assert!(head_str.contains(&format!(
            "Content-Disposition: attachment; filename=\"flow-{bin_id}.bin\"\r\n"
        )));
        assert_eq!(&raw_bytes[sep..], bin_payload.as_slice());

        // 3. Enabling redact_export + redact_strict sanitizes JSON and masks binary bodies.
        let (status, _) = post(
            web,
            "/api/hub",
            r#"{"op":"set_redact","on":true,"strict":true,"fields":""}"#,
        )
        .await;
        assert_eq!(status, 200);

        let (status, redacted_json_resp) = exchange(
            web,
            &format!(
                "GET /api/flows/{json_id}/body HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert_eq!(status, 200);
        assert!(!body_of(&redacted_json_resp).contains("tok_download_123"));
        assert!(body_of(&redacted_json_resp).contains("[REDACTED]"));

        let (status, strict_bin_resp) = exchange(
            web,
            &format!(
                "GET /api/flows/{bin_id}/body HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(
            body_of(&strict_bin_resp).as_bytes(),
            crate::share::STRICT_REDACTED_BODY
        );

        // 4. Unknown flow returns 404.
        let (status, _) = exchange(
            web,
            "GET /api/flows/999999/body HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert_eq!(status, 404);

        drop(server);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn reserve_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    async fn refused(addr: SocketAddr) -> bool {
        !matches!(
            tokio::time::timeout(Duration::from_secs(1), TcpStream::connect(addr)).await,
            Ok(Ok(_))
        )
    }

    async fn post(addr: SocketAddr, path: &str, body: &str) -> (u16, String) {
        exchange(
            addr,
            &format!(
                "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        )
        .await
    }

    async fn exchange(addr: SocketAddr, request: &str) -> (u16, String) {
        let mut socket = TcpStream::connect(addr).await.unwrap();
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), socket.read_to_end(&mut buf))
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8_lossy(&buf).into_owned();
        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        (status, text)
    }

    fn body_of(response: &str) -> &str {
        response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .unwrap_or("")
    }
}
