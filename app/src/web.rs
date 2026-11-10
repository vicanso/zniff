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
const PREVIEW_CAP: usize = 32 * 1024;
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
    let loaded = crate::config::load_mode(&crate::ca::data_dir(), crate::config::is_dev_mode());
    hub.set_bypass(&loaded.bypass_hosts);
    hub.set_rules(&loaded.rules);
    hub.set_upstream(&loaded.upstream_proxy);
    hub.set_show_tunnel_failures(loaded.show_tunnel_failures);
    hub.set_relay_ingress(loaded.relay_accept, &loaded.relay_token);
    let web_root = packaged_web_root();
    match &web_root {
        Some(dir) => println!("界面目录 {}", dir.display()),
        None => eprintln!("没有共用界面，这次是简易操作台。在仓库根目录运行 make web-ui。"),
    }
    let started = if let Some(dir) = web_root {
        WebServer::start_with(
            settings.web_addr(),
            hub,
            settings.capture_bind,
            settings.capture_port,
            Some(dir),
        )
    } else {
        WebServer::start(
            settings.web_addr(),
            hub,
            settings.capture_bind,
            settings.capture_port,
        )
    };
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
    pub fn start(
        web_bind: SocketAddr,
        hub: Arc<Hub>,
        capture_ip: IpAddr,
        capture_port: u16,
    ) -> Result<Self> {
        Self::start_with(web_bind, hub, capture_ip, capture_port, None)
    }

    fn start_with(
        web_bind: SocketAddr,
        hub: Arc<Hub>,
        capture_ip: IpAddr,
        capture_port: u16,
        web_root: Option<PathBuf>,
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

struct Slot {
    hub: Arc<Hub>,
    capture: Option<Capture>,
    capture_ip: IpAddr,
    capture_port: u16,
    error: String,
    relay_push: Option<crate::relay::PushHandle>,
    /// Packaged nsh page. `None` keeps the small HTML console.
    web_root: Option<PathBuf>,
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
}

struct Head {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
}

struct Reply {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

async fn serve_one(mut socket: TcpStream, slot: Arc<Mutex<Slot>>) -> Result<()> {
    let (head_bytes, rest) = read_header(&mut socket).await?;
    let reply = match parse_head(&head_bytes) {
        Ok(head) => route(&mut socket, &head, &rest, &slot).await,
        Err(_) => text(400, "Bad Request", "bad request"),
    };
    let bytes = encode(&reply);
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
            return json_reply(200, "OK", &flow_list(&lock(slot).hub, &filter));
        }
        if let Some(id) = path.strip_prefix("/api/flows/")
            && let Ok(id) = id.parse::<u64>()
        {
            return match flow_detail(&lock(slot).hub, id) {
                Some(value) => json_reply(200, "OK", &value),
                None => json_reply(404, "Not Found", &json!({"error": "not found"})),
            };
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
    let rows = hub.with_session(|session| {
        matched
            .into_iter()
            .filter_map(|(id, _)| session.get(id).map(flow_summary))
            .collect::<Vec<_>>()
    });
    let total = rows.len();
    let start = total.saturating_sub(LIST_CAP);
    json!({
        "total": total,
        "omitted": start,
        "flows": &rows[start..],
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
        "response_bytes": flow.response_bytes,
        "relay": flow.relay_source.as_deref().map(session::relay_short).unwrap_or_default(),
        "rev": flow.rev,
        "relay_source": flow.relay_source,
        "url": flow.url,
        "pinned": flow.pinned,
        "comment": flow.comment,
        "started_ms": unix_millis(flow.started),
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
        let (request_body, request_truncated, request_binary) = preview(&flow.request_body);
        let (response_body, response_truncated, response_binary) = preview(&flow.response_body);
        Some(json!({
            "id": flow.id,
            "method": flow.method,
            "url": flow.url,
            "host": flow.host,
            "path": flow.path,
            "status": flow.status,
            "error": flow.error,
            "protocol": flow.protocol.label(),
            "client_ip": flow.client_ip,
            "relay": flow.relay_source.as_deref().map(session::relay_short).unwrap_or_default(),
            "request_headers": pairs(&flow.request_headers),
            "response_headers": pairs(&flow.response_headers),
            "request_body": request_body,
            "request_truncated": request_truncated || flow.request_truncated,
            "request_binary": request_binary,
            "response_body": response_body,
            "response_truncated": response_truncated || flow.response_truncated,
            "response_binary": response_binary,
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

fn pairs(headers: &[(String, String)]) -> Value {
    Value::Array(
        headers
            .iter()
            .map(|(name, value)| json!([name, value]))
            .collect(),
    )
}

fn preview(bytes: &[u8]) -> (String, bool, bool) {
    let truncated = bytes.len() > PREVIEW_CAP;
    let slice = if truncated {
        &bytes[..PREVIEW_CAP]
    } else {
        bytes
    };
    let binary = std::str::from_utf8(slice).is_err();
    (
        String::from_utf8_lossy(slice).into_owned(),
        truncated,
        binary,
    )
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
        content_type: "text/html; charset=utf-8",
        body,
    }
}

fn text(status: u16, reason: &'static str, message: &str) -> Reply {
    Reply {
        status,
        reason,
        content_type: "text/plain; charset=utf-8",
        body: message.as_bytes().to_vec(),
    }
}

fn json_reply(status: u16, reason: &'static str, value: &Value) -> Reply {
    Reply {
        status,
        reason,
        content_type: "application/json; charset=utf-8",
        body: serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec()),
    }
}

fn encode(reply: &Reply) -> Vec<u8> {
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {len}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        status = reply.status,
        reason = reply.reason,
        content_type = reply.content_type,
        len = reply.body.len(),
    );
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(&reply.body);
    bytes
}

fn header<'a>(head: &'a Head, name: &str) -> Option<&'a str> {
    head.headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn query_value(target: &str, key: &str) -> String {
    let Some(query) = target.split_once('?').map(|(_, query)| query) else {
        return String::new();
    };
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        if name == key {
            return decode(value);
        }
    }
    String::new()
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
        content_type: static_mime(&full_canon),
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

fn sync_hub_op(slot: &mut Slot, op: &str, value: &Value) -> Reply {
    let hub = &slot.hub;
    match op {
        "set_bypass" => hub.set_bypass(&json_text(value)),
        "set_rules" => hub.set_rules(&json_text(value)),
        "set_upstream" => hub.set_upstream(&json_text(value)),
        "set_relay_token" => hub.set_relay_token(&json_text(value)),
        "set_rules_enabled" => {
            let Some(on) = json_flag(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need on"}));
            };
            hub.set_rules_enabled(on);
        }
        "set_breaks_enabled" => {
            let Some(on) = json_flag(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need on"}));
            };
            hub.set_breaks_enabled(on);
        }
        "set_show_tunnel_failures" => {
            let Some(on) = json_flag(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need on"}));
            };
            hub.set_show_tunnel_failures(on);
        }
        "set_relay_accept" => {
            let Some(on) = json_flag(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need on"}));
            };
            hub.set_relay_accept(on);
        }
        "set_pinned" => {
            let Some(id) = json_id(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need id"}));
            };
            let Some(on) = json_flag(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need on"}));
            };
            hub.set_pinned(id, on);
        }
        "set_comment" => {
            let Some(id) = json_id(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need id"}));
            };
            hub.set_comment(id, json_text(value));
        }
        "clear" => hub.clear_session(),
        "clear_unpinned" => {
            hub.clear_unpinned();
        }
        "resolve_break" => {
            hub.resolve_break(BreakDecision {
                drop: value.get("drop").and_then(Value::as_bool).unwrap_or(false),
                headers: json_headers(value.get("headers")),
                body: value
                    .get("body")
                    .and_then(Value::as_str)
                    .map(|text| text.as_bytes().to_vec()),
                status: value
                    .get("status")
                    .and_then(Value::as_u64)
                    .and_then(|code| u16::try_from(code).ok()),
            });
        }
        "resolve_all_breaks" => {
            hub.resolve_all_breaks();
        }
        "promote_break" => {
            let Some(id) = json_id(value) else {
                return json_reply(400, "Bad Request", &json!({"error": "need id"}));
            };
            hub.promote_break(id);
        }
        "touch_break" => hub.touch_break(),
        "end_share" => {
            if let Some(handle) = slot.relay_push.take() {
                handle.stop();
            }
            hub.end_share();
        }
        "import_har" => {
            let document = value.get("value").unwrap_or(&Value::Null);
            hub.import_har(document);
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
    let redact = crate::share::RedactOptions::new(
        value
            .get("redact")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        value
            .get("redact_fields")
            .and_then(Value::as_str)
            .unwrap_or(""),
    );
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
        body: value
            .get("body")
            .and_then(Value::as_str)
            .map(|text| text.as_bytes().to_vec()),
    }
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
