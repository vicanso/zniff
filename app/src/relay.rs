//! Live-share relay.
//!
//! The capturing zniff pushes snapshots to another zniff that both people can
//! reach. `zniff --relay` is the headless listener: it stores the latest
//! document and serves `/s/{id}`. It does not capture and it does not record.
//! A normal zniff accepts the same uploads on its capture port when relay
//! receive is on, and copies those flows into the session list.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::proxy::{Hub, ShareExport};

/// Largest accepted upload. A share is at most 400 flows with clipped bodies.
pub(crate) const MAX_BODY: usize = 32 * 1024 * 1024;
/// Same ceiling as the longest share in the menu.
const MAX_TTL: Duration = Duration::from_secs(8 * 3600);
pub(crate) const SLOT_CAP: usize = 32;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const PUSH_TICK: Duration = Duration::from_millis(100);
const PUSH_TICKS: u32 = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayBase {
    pub origin: String,
    pub host: String,
    pub port: u16,
    pub tls: bool,
}

impl RelayBase {
    pub fn share_url(&self, id: &str) -> String {
        format!("{}/s/{id}", self.origin)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShareRouteError {
    BadUrl,
    EmptyToken,
}

#[derive(Debug)]
pub enum ShareRoute {
    Local { base: String, local_only: bool },
    Relay(RelayBase),
}

/// Empty `relay_url` keeps the link on this computer. A filled-in address
/// must be `http://host:port` or `https://host:port`, with a one-word token.
pub fn share_route(
    relay_url: &str,
    relay_token: &str,
    local_base: &str,
    local_only: bool,
) -> Result<ShareRoute, ShareRouteError> {
    if relay_url.trim().is_empty() {
        return Ok(ShareRoute::Local {
            base: local_base.to_string(),
            local_only,
        });
    }
    let base = normalize_base(relay_url).ok_or(ShareRouteError::BadUrl)?;
    if usable_token(relay_token).is_empty() {
        return Err(ShareRouteError::EmptyToken);
    }
    Ok(ShareRoute::Relay(base))
}

/// A one-word secret. Blank and any ASCII whitespace mean "no token".
pub(crate) fn usable_token(raw: &str) -> String {
    let token = raw.trim();
    if token.is_empty() || token.bytes().any(|byte| byte.is_ascii_whitespace()) {
        String::new()
    } else {
        token.to_string()
    }
}

pub(crate) struct RelayStatus {
    pub status: u16,
    pub reason: &'static str,
    pub message: &'static str,
}

pub(crate) fn bearer_ok(expected: &str, authorization: Option<&str>) -> bool {
    authorization.is_some_and(|value| tokens_equal(expected, value))
}

pub fn normalize_base(raw: &str) -> Option<RelayBase> {
    let raw = raw.trim().trim_end_matches('/');
    let (tls, rest) = strip_ignore_case(raw, "https://")
        .map(|rest| (true, rest))
        .or_else(|| strip_ignore_case(raw, "http://").map(|rest| (false, rest)))?;
    if rest.is_empty()
        || rest.contains('/')
        || rest.contains('?')
        || rest.contains('#')
        || rest.contains('@')
        || rest.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return None;
    }
    let default_port = if tls { 443 } else { 80 };
    let (host, port) = split_host_port(rest, default_port)?;
    let origin = if host.contains(':') {
        format!("{}://[{host}]:{port}", if tls { "https" } else { "http" })
    } else {
        format!("{}://{host}:{port}", if tls { "https" } else { "http" })
    };
    Some(RelayBase {
        origin,
        host,
        port,
        tls,
    })
}

#[derive(Debug)]
pub struct CliSettings {
    pub bind: IpAddr,
    pub port: u16,
    pub token: String,
}

impl CliSettings {
    pub fn addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind, self.port)
    }
}

/// `--relay` flags. `config_port` and `config_token` fill in anything the
/// command line leaves out. An empty token is an error: the listener would
/// otherwise accept uploads from anyone who can open the port.
pub fn cli_settings(
    args: &[String],
    config_port: u16,
    config_token: &str,
) -> Result<CliSettings, String> {
    let mut port = None;
    let mut token = None;
    let mut bind = None;
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if let Some(rest) = arg.strip_prefix("--port=") {
            port = Some(parse_cli_port(rest)?);
        } else if arg == "--port" || arg == "-p" {
            index += 1;
            let next = args.get(index).ok_or_else(|| "缺少端口".to_string())?;
            port = Some(parse_cli_port(next)?);
        } else if let Some(rest) = arg.strip_prefix("--token=") {
            token = Some(rest.to_string());
        } else if arg == "--token" {
            index += 1;
            let next = args.get(index).ok_or_else(|| "缺少密钥".to_string())?;
            token = Some((*next).clone());
        } else if let Some(rest) = arg.strip_prefix("--bind=") {
            bind = Some(parse_bind(rest)?);
        } else if arg == "--bind" {
            index += 1;
            let next = args.get(index).ok_or_else(|| "缺少监听地址".to_string())?;
            bind = Some(parse_bind(next)?);
        } else if arg == "--data-dir" || arg == "--proxy-auth" {
            index += 1;
        }
        index += 1;
    }
    let port = port.unwrap_or(config_port);
    if port == 0 {
        return Err("端口无效".to_string());
    }
    let token = token.unwrap_or_else(|| config_token.trim().to_string());
    let token = token.trim().to_string();
    if token.is_empty() || token.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return Err(
            "需要中转密钥。用法: zniff --relay --token <secret> [--port N] [--bind 0.0.0.0]"
                .to_string(),
        );
    }
    Ok(CliSettings {
        bind: bind.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
        port,
        token,
    })
}

pub fn run_cli(args: &[String], config_port: u16, config_token: &str) -> i32 {
    let settings = match cli_settings(args, config_port, config_token) {
        Ok(settings) => settings,
        Err(message) => {
            eprintln!("{message}");
            return 2;
        }
    };
    let relay = match Relay::start(settings.addr(), &settings.token) {
        Ok(relay) => relay,
        Err(err) => {
            eprintln!("zniff relay 没有绑上 {}: {err}", settings.addr());
            return 1;
        }
    };
    println!("zniff relay listening on http://{}", relay.addr());
    if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        runtime.block_on(async {
            let _ = tokio::signal::ctrl_c().await;
        });
    }
    drop(relay);
    0
}

pub struct Relay {
    addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Relay {
    pub fn start(bind: SocketAddr, token: &str) -> std::io::Result<Self> {
        let listener = std::net::TcpListener::bind(bind)?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&shutdown);
        let token = token.to_string();
        let thread = std::thread::Builder::new()
            .name("zniff-relay".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        log::error!("relay runtime: {err}");
                        return;
                    }
                };
                runtime.block_on(async move {
                    let listener = match tokio::net::TcpListener::from_std(listener) {
                        Ok(listener) => listener,
                        Err(err) => {
                            log::error!("relay listen: {err}");
                            return;
                        }
                    };
                    let state = Arc::new(Mutex::new(State {
                        token,
                        slots: HashMap::new(),
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
                                        let state = Arc::clone(&state);
                                        tokio::spawn(async move {
                                            if let Err(err) = serve_one(socket, state).await {
                                                log::debug!("relay connection: {err}");
                                            }
                                        });
                                    }
                                    Err(err) => {
                                        log::debug!("relay accept: {err}");
                                        tokio::time::sleep(Duration::from_millis(20)).await;
                                    }
                                }
                            }
                        }
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

impl Drop for Relay {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub enum PushEvent {
    Connected,
    Failed(String),
}

#[derive(Clone)]
pub struct PushHandle {
    stop: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
}

impl PushHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Signals stop, then waits until the push thread has finished its DELETE.
    pub fn stop_and_wait(&self, timeout: Duration) {
        self.stop();
        let started = std::time::Instant::now();
        while !self.finished.load(Ordering::Relaxed) && started.elapsed() < timeout {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Pushes the live share until [`PushHandle::stop`] or the local share ends,
/// then asks the relay to delete it.
pub fn spawn_push(
    hub: Arc<Hub>,
    id: String,
    base: RelayBase,
    token: String,
    on_event: impl Fn(PushEvent) + Send + Sync + 'static,
) -> PushHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let done = Arc::clone(&finished);
    let on_event = Arc::new(on_event);
    let spawned = std::thread::Builder::new()
        .name("zniff-relay-push".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    on_event(PushEvent::Failed(err.to_string()));
                    done.store(true, Ordering::Relaxed);
                    return;
                }
            };
            runtime.block_on(push_loop(hub, id, base, token, flag, on_event));
            done.store(true, Ordering::Relaxed);
        });
    if spawned.is_err() {
        finished.store(true, Ordering::Relaxed);
    }
    PushHandle { stop, finished }
}

fn remembered_push() -> &'static Mutex<Option<PushHandle>> {
    static SLOT: Mutex<Option<PushHandle>> = Mutex::new(None);
    &SLOT
}

/// Keeps the latest push so process exit can still delete the remote share.
pub fn remember_push(handle: PushHandle) {
    let mut slot = remembered_push()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    if let Some(previous) = slot.replace(handle) {
        previous.stop();
    }
}

/// Stops the remembered push and waits briefly for its DELETE.
pub fn shutdown_relay_push() {
    let handle = remembered_push()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take();
    if let Some(handle) = handle {
        handle.stop_and_wait(Duration::from_secs(3));
    }
}

async fn push_loop(
    hub: Arc<Hub>,
    id: String,
    base: RelayBase,
    token: String,
    stop: Arc<AtomicBool>,
    on_event: Arc<dyn Fn(PushEvent) + Send + Sync>,
) {
    let mut last_rev = None;
    let mut pushed_bodies = HashSet::new();
    let mut failing = false;
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let export = hub.share_export(&id);
        let Some(export) = export else {
            break;
        };
        match push_export(
            &base,
            &token,
            &id,
            &export,
            &mut last_rev,
            &mut pushed_bodies,
        )
        .await
        {
            Ok(()) => {
                if failing {
                    failing = false;
                    on_event(PushEvent::Connected);
                }
            }
            Err(err) => {
                if !failing {
                    failing = true;
                    on_event(PushEvent::Failed(err.to_string()));
                }
            }
        }
        for _ in 0..PUSH_TICKS {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            tokio::time::sleep(PUSH_TICK).await;
        }
    }
    let _ = delete_share(&base, &token, &id).await;
}

async fn push_export(
    base: &RelayBase,
    token: &str,
    id: &str,
    export: &ShareExport,
    last_rev: &mut Option<u64>,
    pushed_bodies: &mut HashSet<u64>,
) -> Result<(), PushError> {
    if *last_rev != Some(export.rev) {
        put(
            base,
            token,
            &format!("/s/{id}/flows.json"),
            export.document.as_bytes(),
            "application/json",
        )
        .await?;
        *last_rev = Some(export.rev);
    }
    for (flow_id, body) in &export.bodies {
        if pushed_bodies.contains(flow_id) {
            continue;
        }
        if let Err(err) = put(
            base,
            token,
            &format!("/s/{id}/flows/{flow_id}.json"),
            body.as_bytes(),
            "application/json",
        )
        .await
        {
            if err.status == 409 {
                *last_rev = None;
            }
            return Err(err);
        }
        pushed_bodies.insert(*flow_id);
    }
    Ok(())
}

async fn delete_share(base: &RelayBase, token: &str, id: &str) -> Result<(), PushError> {
    request(base, "DELETE", &format!("/s/{id}"), token, &[]).await?;
    Ok(())
}

struct State {
    token: String,
    slots: HashMap<String, Slot>,
}

struct Slot {
    flows: Vec<u8>,
    bodies: HashMap<u64, Vec<u8>>,
    expires: SystemTime,
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
    encoding: Option<&'static str>,
    body: Vec<u8>,
}

#[derive(Debug)]
struct PushError {
    status: u16,
    message: String,
}

impl std::fmt::Display for PushError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.status == 0 {
            write!(formatter, "{}", self.message)
        } else {
            write!(formatter, "HTTP {}", self.status)
        }
    }
}

async fn serve_one(mut socket: TcpStream, state: Arc<Mutex<State>>) -> std::io::Result<()> {
    let (head_bytes, rest) = read_header(&mut socket).await?;
    let reply = match parse_head(&head_bytes) {
        Ok(head) => dispatch(&mut socket, &head, &rest, &state).await,
        Err(_) => plain(400, "Bad Request", "bad request"),
    };
    let bytes = encode(&reply);
    let _ = tokio::time::timeout(IO_TIMEOUT, socket.write_all(&bytes)).await;
    Ok(())
}

async fn dispatch(socket: &mut TcpStream, head: &Head, rest: &[u8], state: &Mutex<State>) -> Reply {
    if head.method.eq_ignore_ascii_case("CONNECT") || head.target.contains("://") {
        return plain(403, "Forbidden", "relay does not proxy");
    }
    let Some((id, kind)) = crate::share::parse_share_path(&head.target) else {
        return plain(404, "Not Found", "not found");
    };
    let zh = crate::share::prefers_zh(header(head, "accept-language"));
    let method = head.method.as_str();
    if method.eq_ignore_ascii_case("PUT") {
        let body = match read_upload(socket, head, rest).await {
            Ok(body) => body,
            Err(reply) => return reply,
        };
        if !authorized(state, head) {
            return plain(401, "Unauthorized", "unauthorized");
        }
        return match kind {
            crate::share::SharePath::Data { .. } => put_flows(state, id, &body),
            crate::share::SharePath::FlowBody(flow_id) => put_body(state, id, flow_id, &body),
            crate::share::SharePath::Page => plain(405, "Method Not Allowed", "upload flows.json"),
        };
    }
    if method.eq_ignore_ascii_case("DELETE") {
        if !authorized(state, head) {
            return plain(401, "Unauthorized", "unauthorized");
        }
        return match kind {
            crate::share::SharePath::Page => {
                let mut state = lock(state);
                state.slots.remove(&id);
                plain(204, "No Content", "")
            }
            _ => plain(405, "Method Not Allowed", "delete the share url"),
        };
    }
    if method.eq_ignore_ascii_case("GET") || method.eq_ignore_ascii_case("HEAD") {
        return match kind {
            crate::share::SharePath::Page => get_page(state, &id, zh),
            crate::share::SharePath::Data { .. } => {
                get_flows(state, &id, zh, header(head, "accept-encoding"))
            }
            crate::share::SharePath::FlowBody(flow_id) => get_body(state, &id, flow_id),
        };
    }
    plain(405, "Method Not Allowed", "method not allowed")
}

async fn read_upload(socket: &mut TcpStream, head: &Head, rest: &[u8]) -> Result<Vec<u8>, Reply> {
    if header(head, "transfer-encoding").is_some_and(|value| {
        value
            .split(',')
            .any(|part| part.trim().eq_ignore_ascii_case("chunked"))
    }) {
        return Err(plain(
            411,
            "Length Required",
            "chunked bodies are not accepted",
        ));
    }
    let Some(raw_len) = header(head, "content-length") else {
        return Err(plain(411, "Length Required", "content-length required"));
    };
    let len: usize = match raw_len.parse() {
        Ok(len) => len,
        Err(_) => return Err(plain(400, "Bad Request", "bad content-length")),
    };
    if len > MAX_BODY {
        return Err(plain(413, "Content Too Large", "body is too large"));
    }
    let mut body = Vec::with_capacity(len);
    let take = rest.len().min(len);
    body.extend_from_slice(&rest[..take]);
    while body.len() < len {
        let mut buf = [0_u8; 8192];
        let read = match tokio::time::timeout(IO_TIMEOUT, socket.read(&mut buf)).await {
            Ok(Ok(0)) => return Err(plain(400, "Bad Request", "body ended early")),
            Ok(Ok(count)) => count,
            Ok(Err(_)) | Err(_) => return Err(plain(400, "Bad Request", "body read failed")),
        };
        let need = len - body.len();
        body.extend_from_slice(&buf[..read.min(need)]);
    }
    Ok(body)
}

fn put_flows(state: &Mutex<State>, id: String, body: &[u8]) -> Reply {
    let now = SystemTime::now();
    let (bytes, expires) = match prepare_flows(body, now) {
        Ok(pair) => pair,
        Err(status) => return plain(status.status, status.reason, status.message),
    };
    let mut state = lock(state);
    gc(&mut state.slots, now);
    if !state.slots.contains_key(&id) && state.slots.len() >= SLOT_CAP {
        return plain(429, "Too Many Requests", "too many shares");
    }
    match state.slots.get_mut(&id) {
        Some(slot) => {
            slot.flows = bytes;
            slot.expires = expires;
        }
        None => {
            state.slots.insert(
                id,
                Slot {
                    flows: bytes,
                    bodies: HashMap::new(),
                    expires,
                },
            );
        }
    }
    plain(204, "No Content", "")
}

fn put_body(state: &Mutex<State>, id: String, flow_id: u64, body: &[u8]) -> Reply {
    if body.is_empty() || body.len() > MAX_BODY {
        return plain(413, "Content Too Large", "body is too large");
    }
    if serde_json::from_slice::<serde_json::Value>(body).is_err() {
        return plain(400, "Bad Request", "flow body must be JSON");
    }
    let now = SystemTime::now();
    let mut state = lock(state);
    gc(&mut state.slots, now);
    let Some(slot) = state.slots.get_mut(&id) else {
        return plain(409, "Conflict", "upload flows.json first");
    };
    slot.bodies.insert(flow_id, body.to_vec());
    plain(204, "No Content", "")
}

fn get_page(state: &Mutex<State>, id: &str, zh: bool) -> Reply {
    let now = SystemTime::now();
    let mut state = lock(state);
    gc(&mut state.slots, now);
    let Some(slot) = state.slots.get(id) else {
        return expired(zh);
    };
    let json = String::from_utf8_lossy(&slot.flows).into_owned();
    let live = format!("/s/{id}/flows.json");
    let html = crate::share::page(&json, &live, zh);
    Reply {
        status: 200,
        reason: "OK",
        content_type: "text/html; charset=utf-8",
        encoding: None,
        body: html.into_bytes(),
    }
}

fn get_flows(state: &Mutex<State>, id: &str, zh: bool, accept_encoding: Option<&str>) -> Reply {
    let now = SystemTime::now();
    let bytes = {
        let mut state = lock(state);
        gc(&mut state.slots, now);
        match state.slots.get(id) {
            Some(slot) => slot.flows.clone(),
            None => return expired(zh),
        }
    };
    if accepts_gzip(accept_encoding)
        && let Some(packed) = gzip_bytes(&bytes)
    {
        return Reply {
            status: 200,
            reason: "OK",
            content_type: "application/json; charset=utf-8",
            encoding: Some("gzip"),
            body: packed,
        };
    }
    Reply {
        status: 200,
        reason: "OK",
        content_type: "application/json; charset=utf-8",
        encoding: None,
        body: bytes,
    }
}

fn get_body(state: &Mutex<State>, id: &str, flow_id: u64) -> Reply {
    let now = SystemTime::now();
    let mut state = lock(state);
    gc(&mut state.slots, now);
    let body = state
        .slots
        .get(id)
        .and_then(|slot| slot.bodies.get(&flow_id))
        .cloned();
    match body {
        Some(body) => Reply {
            status: 200,
            reason: "OK",
            content_type: "application/json; charset=utf-8",
            encoding: None,
            body,
        },
        None => Reply {
            status: 404,
            reason: "Not Found",
            content_type: "application/json; charset=utf-8",
            encoding: None,
            body: b"{}".to_vec(),
        },
    }
}

pub(crate) fn prepare_flows(
    body: &[u8],
    now: SystemTime,
) -> Result<(Vec<u8>, SystemTime), RelayStatus> {
    if body.len() > MAX_BODY {
        return Err(reject(413, "Content Too Large", "body is too large"));
    }
    let mut value: serde_json::Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(_) => {
            return Err(reject(
                400,
                "Bad Request",
                "flows.json must be a JSON object",
            ));
        }
    };
    let Some(obj) = value.as_object_mut() else {
        return Err(reject(
            400,
            "Bad Request",
            "flows.json must be a JSON object",
        ));
    };
    obj.insert("incremental".to_string(), serde_json::Value::Bool(false));
    let expires = obj
        .get("expires")
        .and_then(|item| item.as_u64())
        .unwrap_or(0);
    let Some(when) = slot_deadline(expires, now) else {
        return Err(reject(400, "Bad Request", "share is already expired"));
    };
    let secs = when
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    obj.insert("expires".to_string(), serde_json::Value::from(secs));
    match serde_json::to_vec(&value) {
        Ok(bytes) => Ok((bytes, when)),
        Err(_) => Err(reject(
            400,
            "Bad Request",
            "flows.json must be a JSON object",
        )),
    }
}

fn reject(status: u16, reason: &'static str, message: &'static str) -> RelayStatus {
    RelayStatus {
        status,
        reason,
        message,
    }
}

fn slot_deadline(expires_secs: u64, now: SystemTime) -> Option<SystemTime> {
    if expires_secs == 0 {
        return None;
    }
    let when = UNIX_EPOCH + Duration::from_secs(expires_secs);
    let cap = now + MAX_TTL;
    let when = if when > cap { cap } else { when };
    if when <= now { None } else { Some(when) }
}

fn gc(slots: &mut HashMap<String, Slot>, now: SystemTime) {
    slots.retain(|_, slot| now < slot.expires);
}

fn authorized(state: &Mutex<State>, head: &Head) -> bool {
    let expected = lock(state).token.clone();
    bearer_ok(&expected, header(head, "authorization"))
}

fn tokens_equal(expected: &str, header_value: &str) -> bool {
    if expected.is_empty() {
        return false;
    }
    let Some(got) = presented_token(header_value) else {
        return false;
    };
    let left = expected.as_bytes();
    let right = got.as_bytes();
    let mut diff = left.len() ^ right.len();
    let width = left.len().max(right.len());
    for index in 0..width {
        let x = left.get(index).copied().unwrap_or(0);
        let y = right.get(index).copied().unwrap_or(0);
        diff |= usize::from(x ^ y);
    }
    diff == 0
}

fn presented_token(value: &str) -> Option<&str> {
    let value = value.trim();
    let mut parts = value.splitn(2, char::is_whitespace);
    let scheme = parts.next()?;
    let rest = parts.next()?.trim();
    if scheme.eq_ignore_ascii_case("bearer") && !rest.is_empty() {
        Some(rest)
    } else {
        None
    }
}

fn expired(zh: bool) -> Reply {
    Reply {
        status: 404,
        reason: "Not Found",
        content_type: "text/html; charset=utf-8",
        encoding: None,
        body: crate::share::expired_page(zh).into_bytes(),
    }
}

fn plain(status: u16, reason: &'static str, message: &str) -> Reply {
    Reply {
        status,
        reason,
        content_type: "text/plain; charset=utf-8",
        encoding: None,
        body: message.as_bytes().to_vec(),
    }
}

fn encode(reply: &Reply) -> Vec<u8> {
    let encoding = match reply.encoding {
        Some(value) => format!("Content-Encoding: {value}\r\n"),
        None => String::new(),
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\n{encoding}Content-Length: {len}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        status = reply.status,
        reason = reply.reason,
        content_type = reply.content_type,
        len = reply.body.len(),
    );
    let mut out = head.into_bytes();
    out.extend_from_slice(&reply.body);
    out
}

fn header<'a>(head: &'a Head, name: &str) -> Option<&'a str> {
    head.headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn parse_head(bytes: &[u8]) -> Result<Head, ()> {
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

async fn read_header(socket: &mut TcpStream) -> std::io::Result<(Vec<u8>, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut tmp = [0_u8; 2048];
    loop {
        if let Some(end) = header_end(&buf) {
            let rest = buf.split_off(end);
            return Ok((buf, rest));
        }
        if buf.len() > 64 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "header too large",
            ));
        }
        let count = tokio::time::timeout(IO_TIMEOUT, socket.read(&mut tmp))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "header timeout"))??;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
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

async fn wait_shutdown(flag: &AtomicBool) {
    while !flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|err| err.into_inner())
}

async fn put(
    base: &RelayBase,
    token: &str,
    path: &str,
    body: &[u8],
    content_type: &str,
) -> Result<(), PushError> {
    let reply = request_with_type(base, "PUT", path, token, body, content_type).await?;
    if (200..300).contains(&reply) {
        Ok(())
    } else {
        Err(PushError {
            status: reply,
            message: String::new(),
        })
    }
}

async fn request(
    base: &RelayBase,
    method: &str,
    path: &str,
    token: &str,
    body: &[u8],
) -> Result<u16, PushError> {
    request_with_type(base, method, path, token, body, "application/octet-stream").await
}

async fn request_with_type(
    base: &RelayBase,
    method: &str,
    path: &str,
    token: &str,
    body: &[u8],
    content_type: &str,
) -> Result<u16, PushError> {
    let host = host_header(&base.host, base.port);
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Type: {content_type}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n",
        len = body.len(),
    );
    let mut message = head.into_bytes();
    message.extend_from_slice(body);
    let stream = connect(base).await?;
    exchange(stream, &message).await
}

async fn connect(base: &RelayBase) -> Result<Io, PushError> {
    let tcp = tokio::time::timeout(
        IO_TIMEOUT,
        TcpStream::connect((base.host.as_str(), base.port)),
    )
    .await
    .map_err(|_| network_error("timed out"))?
    .map_err(|err| network_error(&err.to_string()))?;
    if !base.tls {
        return Ok(Io::Plain(tcp));
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = server_name(&base.host)?;
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let tls = tokio::time::timeout(IO_TIMEOUT, connector.connect(name, tcp))
        .await
        .map_err(|_| network_error("tls timed out"))?
        .map_err(|err| network_error(&err.to_string()))?;
    Ok(Io::Tls(Box::new(tls)))
}

fn server_name(host: &str) -> Result<rustls::pki_types::ServerName<'static>, PushError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        Ok(rustls::pki_types::ServerName::IpAddress(ip.into()))
    } else {
        rustls::pki_types::ServerName::try_from(host.to_string())
            .map_err(|_| network_error("bad relay host"))
    }
}

fn network_error(message: &str) -> PushError {
    PushError {
        status: 0,
        message: message.to_string(),
    }
}

enum Io {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl AsyncRead for Io {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => std::pin::Pin::new(stream).poll_read(cx, buf),
            Self::Tls(stream) => std::pin::Pin::new(stream.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Io {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(stream) => std::pin::Pin::new(stream).poll_write(cx, buf),
            Self::Tls(stream) => std::pin::Pin::new(stream.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => std::pin::Pin::new(stream).poll_flush(cx),
            Self::Tls(stream) => std::pin::Pin::new(stream.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => std::pin::Pin::new(stream).poll_shutdown(cx),
            Self::Tls(stream) => std::pin::Pin::new(stream.as_mut()).poll_shutdown(cx),
        }
    }
}

async fn exchange(mut socket: Io, message: &[u8]) -> Result<u16, PushError> {
    tokio::time::timeout(IO_TIMEOUT, socket.write_all(message))
        .await
        .map_err(|_| network_error("timed out"))?
        .map_err(|err| network_error(&err.to_string()))?;
    let mut buf = Vec::new();
    let mut tmp = [0_u8; 2048];
    loop {
        if header_end(&buf).is_some() {
            break;
        }
        let count = tokio::time::timeout(IO_TIMEOUT, socket.read(&mut tmp))
            .await
            .map_err(|_| network_error("timed out"))?
            .map_err(|err| network_error(&err.to_string()))?;
        if count == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..count]);
        if buf.len() > 64 * 1024 {
            break;
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let start = text.lines().next().unwrap_or("");
    let status = start
        .split_whitespace()
        .nth(1)
        .and_then(|part| part.parse().ok())
        .ok_or_else(|| network_error("bad response"))?;
    Ok(status)
}

fn accepts_gzip(header: Option<&str>) -> bool {
    let Some(header) = header else {
        return false;
    };
    for part in header.split(',') {
        let mut bits = part.split(';');
        let coding = bits.next().unwrap_or("").trim();
        if !coding.eq_ignore_ascii_case("gzip") {
            continue;
        }
        let mut quality = 1.0_f32;
        for param in bits {
            let Some(value) = param.trim().strip_prefix("q=").or_else(|| {
                let trimmed = param.trim();
                trimmed
                    .get(..2)
                    .filter(|prefix| prefix.eq_ignore_ascii_case("q="))
                    .map(|_| &trimmed[2..])
            }) else {
                continue;
            };
            quality = value.trim().parse().unwrap_or(0.0);
        }
        return quality > 0.0;
    }
    false
}

fn gzip_bytes(body: &[u8]) -> Option<Vec<u8>> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(body).ok()?;
    encoder.finish().ok()
}

fn host_header(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn strip_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    if text.len() >= prefix.len()
        && text.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
    {
        Some(&text[prefix.len()..])
    } else {
        None
    }
}

fn split_host_port(authority: &str, default_port: u16) -> Option<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, port) = rest.split_once(']')?;
        if host.is_empty() {
            return None;
        }
        let port = port.trim_start_matches(':');
        let port = if port.is_empty() {
            default_port
        } else {
            port.parse().ok()?
        };
        return Some((host.to_string(), port));
    }
    if authority.matches(':').count() > 1 {
        return None;
    }
    if let Some((host, port)) = authority.rsplit_once(':')
        && !host.is_empty()
        && let Ok(port) = port.parse::<u16>()
    {
        return Some((host.to_string(), port));
    }
    if authority.is_empty() {
        return None;
    }
    Some((authority.to_string(), default_port))
}

fn parse_cli_port(text: &str) -> Result<u16, String> {
    crate::present::parse_port(text).ok_or_else(|| "端口无效".to_string())
}

fn parse_bind(text: &str) -> Result<IpAddr, String> {
    text.parse()
        .map_err(|_| "监听地址要写成 0.0.0.0 或 127.0.0.1".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_id(index: u32) -> String {
        format!("{index:08x}-1111-4111-8111-111111111111")
    }

    fn future_expires(extra: u64) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_add(extra)
    }

    fn flows_json(host: &str, expires: u64, incremental: bool) -> String {
        format!(
            r#"{{"expires":{expires},"filter":"","omitted":0,"rev":7,"incremental":{incremental},"ids":[1],"flows":[{{"id":1,"host":"{host}","request_body_spilled":false,"response_body_spilled":false}}]}}"#
        )
    }

    async fn raw_exchange(addr: SocketAddr, message: &str) -> (u16, String, Vec<u8>) {
        let mut socket = TcpStream::connect(addr).await.unwrap();
        socket.write_all(message.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), socket.read_to_end(&mut buf))
            .await
            .unwrap()
            .unwrap();
        let end = header_end(&buf).unwrap();
        let head = String::from_utf8_lossy(&buf[..end]).to_string();
        let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, head, buf[end..].to_vec())
    }

    #[test]
    fn addresses_and_cli_flags() {
        assert!(normalize_base("https://relay.example").is_some_and(|base| {
            base.tls && base.port == 443 && base.origin == "https://relay.example:443"
        }));
        let base = normalize_base("HTTP://10.1.2.3:9090/").unwrap();
        assert!(!base.tls);
        assert_eq!(base.origin, "http://10.1.2.3:9090");
        assert_eq!(
            normalize_base("http://[::1]:9").unwrap().share_url("abc"),
            "http://[::1]:9/s/abc"
        );
        assert!(normalize_base("10.1.2.3:9").is_none());
        assert!(normalize_base("http://10.1.2.3:9/s/x").is_none());
        assert!(normalize_base("http://user:secret@10.1.2.3:9").is_none());

        assert!(matches!(
            share_route("", "", "http://127.0.0.1:1", true),
            Ok(ShareRoute::Local {
                local_only: true,
                ..
            })
        ));
        assert_eq!(
            share_route("http://10.0.0.1:9", "", "http://127.0.0.1:1", false).unwrap_err(),
            ShareRouteError::EmptyToken
        );
        assert_eq!(
            share_route("not a url", "secret", "http://127.0.0.1:1", false).unwrap_err(),
            ShareRouteError::BadUrl
        );
        match share_route("http://10.0.0.1:9", "secret", "http://127.0.0.1:1", true).unwrap() {
            ShareRoute::Relay(base) => assert_eq!(base.share_url("id"), "http://10.0.0.1:9/s/id"),
            ShareRoute::Local { .. } => panic!("relay"),
        }

        let missing = cli_settings(&["--relay".into()], 8877, "").unwrap_err();
        assert!(missing.contains("--token"));
        let spaced = cli_settings(&["--relay".into()], 8877, "has space").unwrap_err();
        assert!(spaced.contains("--token"));
        let settings = cli_settings(
            &[
                "--relay".into(),
                "--token".into(),
                "s3cret".into(),
                "--bind".into(),
                "127.0.0.1".into(),
                "--port".into(),
                "9".into(),
            ],
            8877,
            "ignored",
        )
        .unwrap();
        assert_eq!(settings.port, 9);
        assert!(settings.bind.is_loopback());
        assert_eq!(settings.token, "s3cret");
        assert_eq!(settings.addr(), "127.0.0.1:9".parse().unwrap());
        let from_config = cli_settings(&["--relay".into()], 8878, "from-config").unwrap();
        assert_eq!(from_config.port, 8878);
        assert_eq!(from_config.token, "from-config");
        assert!(from_config.bind.is_unspecified());
        assert!(cli_settings(&["--relay".into(), "--port".into(), "0".into()], 1, "t").is_err());

        let now = SystemTime::now();
        let now_secs = now.duration_since(UNIX_EPOCH).unwrap().as_secs();
        assert!(slot_deadline(0, now).is_none());
        assert!(slot_deadline(now_secs.saturating_sub(5), now).is_none());
        let capped = slot_deadline(now_secs + 10 * 365 * 24 * 3600, now).unwrap();
        let left = capped.duration_since(now).unwrap();
        assert!(left <= MAX_TTL);
        assert!(left + Duration::from_secs(2) >= MAX_TTL);
    }

    #[test]
    fn spilled_ids_follow_the_flags() {
        let json = r#"{"flows":[
            {"id":3,"request_body_spilled":true,"response_body_spilled":false},
            {"id":4,"request_body_spilled":false,"response_body_spilled":true},
            {"id":5,"request_body_spilled":false,"response_body_spilled":false}
        ]}"#;
        assert_eq!(crate::proxy::spilled_flow_ids(json), vec![3, 4]);
    }

    #[tokio::test]
    async fn relay_serves_pushed_shares_and_refuses_proxying() {
        let relay = Relay::start("127.0.0.1:0".parse().unwrap(), "test-token").unwrap();
        let addr = relay.addr();
        let id = sample_id(1);
        let expires = future_expires(120);
        let document = flows_json("relay.example", expires + 10 * 365 * 24 * 3600, true);
        let put = format!(
            "PUT /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer test-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{document}",
            document.len()
        );
        let (status, _, _) = raw_exchange(addr, &put).await;
        assert_eq!(status, 204);

        let open = format!(
            "PUT /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{document}",
            document.len()
        );
        let (status, _, body) = raw_exchange(addr, &open).await;
        assert_eq!(status, 401);
        assert!(
            !body
                .windows(b"test-token".len())
                .any(|window| window == b"test-token")
        );

        let wrong = format!(
            "PUT /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer other\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
        );
        assert_eq!(raw_exchange(addr, &wrong).await.0, 401);

        let get = format!(
            "GET /s/{id}/flows.json?since=3 HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
        );
        let (status, head, body) = raw_exchange(addr, &get).await;
        assert_eq!(status, 200, "{head}");
        assert!(!head.to_ascii_lowercase().contains("content-encoding"));
        assert!(
            head.to_ascii_lowercase()
                .contains("cache-control: no-store")
        );
        assert!(head.to_ascii_lowercase().contains("connection: close"));
        let length = content_length(&head);
        assert_eq!(length, body.len());
        let text = String::from_utf8(body).unwrap();
        assert!(text.contains("relay.example"), "{text}");
        assert!(text.contains("\"incremental\":false"), "{text}");
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let stored = value["expires"].as_u64().unwrap();
        let now = future_expires(0);
        assert!(stored <= now + MAX_TTL.as_secs());
        assert!(stored + 5 >= now + MAX_TTL.as_secs());

        let gzip_get = format!(
            "GET /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAccept-Encoding: deflate, gzip\r\nConnection: close\r\n\r\n"
        );
        let (status, head, packed) = raw_exchange(addr, &gzip_get).await;
        assert_eq!(status, 200);
        assert!(head.to_ascii_lowercase().contains("content-encoding: gzip"));
        assert_eq!(content_length(&head), packed.len());
        assert_eq!(&packed[..2], &[0x1f, 0x8b]);
        let mut decoder = flate2::read::GzDecoder::new(packed.as_slice());
        let mut inflated = String::new();
        std::io::Read::read_to_string(&mut decoder, &mut inflated).unwrap();
        assert_eq!(inflated, text);

        let identity = format!(
            "GET /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAccept-Encoding: gzip;q=0, identity\r\nConnection: close\r\n\r\n"
        );
        let (_, identity_head, identity_body) = raw_exchange(addr, &identity).await;
        assert!(
            !identity_head
                .to_ascii_lowercase()
                .contains("content-encoding")
        );
        assert_eq!(identity_body, text.as_bytes());

        let page = format!(
            "GET /s/{id} HTTP/1.1\r\nHost: {addr}\r\nAccept-Encoding: gzip\r\nAccept-Language: en\r\nConnection: close\r\n\r\n"
        );
        let (status, head, body) = raw_exchange(addr, &page).await;
        assert_eq!(status, 200);
        assert!(!head.to_ascii_lowercase().contains("content-encoding"));
        let html = String::from_utf8(body).unwrap();
        assert!(html.starts_with("<!DOCTYPE html") || html.contains("<html"));
        assert!(html.contains("zniff share"));
        assert!(html.contains(&format!("/s/{id}/flows.json")));
        assert!(html.contains("relay.example"));

        let zh_page = format!(
            "GET /s/{id} HTTP/1.1\r\nHost: {addr}\r\nAccept-Language: zh-CN\r\nConnection: close\r\n\r\n"
        );
        let html = String::from_utf8(raw_exchange(addr, &zh_page).await.2).unwrap();
        assert!(html.contains("zniff 分享"));

        let flow_body = r#"{"id":1,"response_body":"n"}"#;
        let early = sample_id(2);
        let early_put = format!(
            "PUT /s/{early}/flows/1.json HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer test-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{flow_body}",
            flow_body.len()
        );
        assert_eq!(raw_exchange(addr, &early_put).await.0, 409);

        let body_put = format!(
            "PUT /s/{id}/flows/1.json HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer test-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{flow_body}",
            flow_body.len()
        );
        assert_eq!(raw_exchange(addr, &body_put).await.0, 204);
        let body_get = format!(
            "GET /s/{id}/flows/1.json HTTP/1.1\r\nHost: {addr}\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n"
        );
        let (status, head, body) = raw_exchange(addr, &body_get).await;
        assert_eq!(status, 200);
        assert!(!head.to_ascii_lowercase().contains("content-encoding"));
        assert_eq!(body, flow_body.as_bytes());

        let missing_body = format!(
            "GET /s/{id}/flows/9.json HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
        );
        let (status, _, body) = raw_exchange(addr, &missing_body).await;
        assert_eq!(status, 404);
        assert_eq!(body, b"{}");

        let install = format!("GET /zniff HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
        let (status, _, body) = raw_exchange(addr, &install).await;
        assert_eq!(status, 404);
        assert!(
            !body
                .windows(b"zniff CA".len())
                .any(|window| window == b"zniff CA")
        );

        assert_eq!(
            raw_exchange(
                addr,
                "GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n"
            )
            .await
            .0,
            403
        );
        assert_eq!(
            raw_exchange(
                addr,
                "CONNECT example.com:443 HTTP/1.1\r\nHost: example.com:443\r\nConnection: close\r\n\r\n"
            )
            .await
            .0,
            403
        );

        let huge = format!(
            "PUT /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer test-token\r\nContent-Length: 99999999\r\nConnection: close\r\n\r\n"
        );
        assert_eq!(raw_exchange(addr, &huge).await.0, 413);

        let stale = flows_json("old.example", 1, false);
        let stale_put = format!(
            "PUT /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer test-token\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{stale}",
            stale.len()
        );
        assert_eq!(raw_exchange(addr, &stale_put).await.0, 400);

        let removed =
            format!("DELETE /s/{id} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
        assert_eq!(raw_exchange(addr, &removed).await.0, 401);
        let removed = format!(
            "DELETE /s/{id} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer test-token\r\nConnection: close\r\n\r\n"
        );
        assert_eq!(raw_exchange(addr, &removed).await.0, 204);
        let gone = format!("GET /s/{id} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
        let (status, _, body) = raw_exchange(addr, &gone).await;
        assert_eq!(status, 404);
        let page = String::from_utf8_lossy(&body);
        assert!(page.contains("分享已结束") || page.contains("Share ended"));
        let again = format!(
            "DELETE /s/{id} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer test-token\r\nConnection: close\r\n\r\n"
        );
        assert_eq!(raw_exchange(addr, &again).await.0, 204);

        for index in 0..SLOT_CAP {
            let slot = sample_id(100 + index as u32);
            let document = flows_json("cap.example", future_expires(30), false);
            let message = format!(
                "PUT /s/{slot}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer test-token\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{document}",
                document.len()
            );
            assert_eq!(raw_exchange(addr, &message).await.0, 204, "{index}");
        }
        let overflow = sample_id(1000);
        let document = flows_json("cap.example", future_expires(30), false);
        let message = format!(
            "PUT /s/{overflow}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer test-token\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{document}",
            document.len()
        );
        assert_eq!(raw_exchange(addr, &message).await.0, 429);
        drop(relay);
    }

    #[tokio::test]
    async fn capture_push_follows_the_session_and_deletes_on_stop() {
        let relay = Relay::start("127.0.0.1:0".parse().unwrap(), "push-token").unwrap();
        let addr = relay.addr();
        let dir = std::env::temp_dir().join(format!("zniff-relay-push-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(crate::ca::Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        hub.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "GET".into(),
            host: "push.example".into(),
            path: "/one".into(),
            url: "http://push.example/one".into(),
            request_headers: vec![("Accept".into(), "text/plain".into())],
        });
        let (id, _) = hub.begin_share(Duration::from_secs(60), "");
        let base = normalize_base(&format!("http://{addr}")).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&events);
        let handle = spawn_push(
            Arc::clone(&hub),
            id.clone(),
            base,
            "push-token".into(),
            move |event| {
                let label = match event {
                    PushEvent::Connected => "up".into(),
                    PushEvent::Failed(err) => format!("down {err}"),
                };
                seen.lock().unwrap().push(label);
            },
        );
        let get =
            format!("GET /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
        let mut saw = false;
        for _ in 0..30 {
            let (status, _, body) = raw_exchange(addr, &get).await;
            if status == 200
                && body
                    .windows(b"push.example".len())
                    .any(|window| window == b"push.example")
            {
                saw = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(saw, "first snapshot was not pushed");

        hub.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "POST".into(),
            host: "second.example".into(),
            path: "/two".into(),
            url: "http://second.example/two".into(),
            request_headers: Vec::new(),
        });
        let mut saw_second = false;
        for _ in 0..40 {
            let (status, _, body) = raw_exchange(addr, &get).await;
            if status == 200
                && body
                    .windows(b"second.example".len())
                    .any(|window| window == b"second.example")
            {
                saw_second = true;
                let text = String::from_utf8_lossy(&body);
                assert!(text.contains("\"incremental\":false"), "{text}");
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(saw_second, "later flow was not pushed");
        assert!(
            events.lock().unwrap().is_empty(),
            "a healthy push should not change the status: {:?}",
            events.lock().unwrap()
        );

        handle.stop();
        hub.end_share();
        let mut gone = false;
        for _ in 0..40 {
            if raw_exchange(addr, &get).await.0 == 404 {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(gone, "stop did not delete the relay share");
        let _ = std::fs::remove_dir_all(&dir);
        drop(relay);
    }

    fn content_length(head: &str) -> usize {
        head.lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().ok())
                    .flatten()
            })
            .unwrap()
    }
}
