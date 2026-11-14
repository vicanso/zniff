//! HTTP proxy: CONNECT with protocol following, and plain HTTP.
//!
//! The origin is contacted directly, never through the system proxy. A side
//! copy of each body is decompressed and capped; forwarding does not wait
//! for the UI.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use bytes::Bytes;
use http::{HeaderMap, HeaderName, Method, Request, Response, StatusCode, Uri, Version};
use http_body::Body;
use http_body::Frame;
use http_body_util::BodyExt;
#[cfg(test)]
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::upgrade::Upgraded;
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::RootCertStore;
use rustls::client::ClientConfig;
#[cfg(test)]
use rustls::pki_types::CertificateDer;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use crate::bypass::BypassRules;
use crate::ca::Ca;
use crate::client_hello::{ReadHello, read_client_hello};
use crate::decompress::Decoder;
use crate::rules::{self, Action, Rule};
use crate::session::{Direction, FlowDraft, Message, Protocol, RelayDraft, Session};
use crate::ws::{self, FrameFeed};

const IO_TIMEOUT: Duration = Duration::from_secs(20);
/// Drop a cached origin HTTP/2 connection before a typical server idle timeout.
const H2_IDLE: Duration = Duration::from_secs(45);
/// List refreshes follow `generation`. Streaming body chunks bump it at most this often.
const BODY_UI_TOUCH: Duration = Duration::from_millis(100);

fn body_clock_ms() -> u64 {
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    START
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}
/// Maximum number of concurrent flows waiting in the breakpoint FIFO queue.
pub const BREAK_QUEUE_CAP: usize = 8;

tokio::task_local! {
    static CLIENT_CTX: Arc<ClientConnCtx>;
}

struct ClientConnCtx {
    label: Mutex<String>,
    opened_ids: Mutex<Vec<u64>>,
}

fn current_client_ctx() -> Option<Arc<ClientConnCtx>> {
    CLIENT_CTX.try_with(Arc::clone).ok()
}

fn local_app_cache() -> &'static Mutex<HashMap<u16, (String, Instant)>> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Mutex<HashMap<u16, (String, Instant)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_local_app(port: u16) -> Option<String> {
    let mut guard = local_app_cache()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    if let Some((app, at)) = guard.get(&port)
        && at.elapsed() < Duration::from_secs(15)
    {
        return (!app.is_empty()).then(|| app.clone());
    }
    if guard.len() > 256 {
        guard.retain(|_, (_, at)| at.elapsed() < Duration::from_secs(15));
    }
    None
}

fn store_cached_local_app(port: u16, app: String) {
    if app.is_empty() {
        return;
    }
    let mut guard = local_app_cache()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    guard.insert(port, (app, Instant::now()));
}

struct LocalPorts {
    at: Instant,
    map: HashMap<u16, String>,
    ok: bool,
}

/// One process-name snapshot for every local TCP port. Callers share a scan
/// instead of spawning `lsof` once per loopback connection.
fn port_snapshot() -> HashMap<u16, String> {
    use std::sync::OnceLock;
    static SNAP: OnceLock<Mutex<LocalPorts>> = OnceLock::new();
    let snap = SNAP.get_or_init(|| {
        Mutex::new(LocalPorts {
            at: Instant::now() - Duration::from_secs(60),
            map: HashMap::new(),
            ok: false,
        })
    });
    let mut guard = snap.lock().unwrap_or_else(|err| err.into_inner());
    // A failed scan keeps the previous map and waits out the same TTL, so one
    // outage does not spawn `lsof` again on every new connection.
    let stale = guard.at.elapsed() >= Duration::from_millis(1500);
    if stale {
        match scan_local_ports() {
            Some(map) => {
                guard.map = map;
                guard.ok = true;
                guard.at = Instant::now();
            }
            None => {
                // A failed scan is not "no process". Keep the previous map, but
                // don't retry every connection for the next 1.5s.
                guard.ok = false;
                guard.at = Instant::now();
            }
        }
    }
    guard.map.clone()
}

#[cfg(any(test, target_os = "macos", target_os = "windows"))]
fn local_endpoint_port(name: &str) -> Option<u16> {
    let local = name.split("->").next()?.trim();
    let port = local.rsplit(':').next()?.trim();
    if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    port.parse().ok()
}

#[cfg(test)]
fn scan_local_ports() -> Option<HashMap<u16, String>> {
    Some(HashMap::new())
}

#[cfg(all(not(test), target_os = "macos"))]
fn scan_local_ports() -> Option<HashMap<u16, String>> {
    let out = std::process::Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:ESTABLISHED", "-Fpcn"])
        .output()
        .ok()?;
    if !out.status.success() && out.stdout.is_empty() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut map = HashMap::new();
    let mut current = String::new();
    for line in stdout.lines() {
        if let Some(cmd) = line.strip_prefix('c') {
            current = clean_process_name(cmd.trim());
        } else if let Some(net) = line.strip_prefix('n')
            && !current.is_empty()
            && let Some(port) = local_endpoint_port(net)
        {
            map.entry(port).or_insert_with(|| current.clone());
        }
    }
    Some(map)
}

#[cfg(all(not(test), target_os = "linux"))]
fn scan_local_ports() -> Option<HashMap<u16, String>> {
    let mut inode_port: HashMap<String, u16> = HashMap::new();
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in content.lines().skip(1) {
            let mut cols = line.split_whitespace();
            let _sl = cols.next();
            let Some(local_addr) = cols.next() else {
                continue;
            };
            let Some((_, port_hex)) = local_addr.rsplit_once(':') else {
                continue;
            };
            let Ok(port) = u16::from_str_radix(port_hex, 16) else {
                continue;
            };
            let Some(inode) = cols.nth(7) else {
                continue;
            };
            if inode != "0" {
                inode_port.entry(inode.to_string()).or_insert(port);
            }
        }
    }
    if inode_port.is_empty() {
        return Some(HashMap::new());
    }
    let mut map = HashMap::new();
    let entries = std::fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        let fname = entry.file_name();
        let Some(pid_str) = fname.to_str() else {
            continue;
        };
        if !pid_str.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let fd_dir = entry.path().join("fd");
        let Ok(fds) = std::fs::read_dir(fd_dir) else {
            continue;
        };
        let mut name = None;
        for fd in fds.flatten() {
            let Ok(link) = std::fs::read_link(fd.path()) else {
                continue;
            };
            let link = link.to_string_lossy();
            let Some(inode) = link
                .strip_prefix("socket:[")
                .and_then(|rest| rest.strip_suffix(']'))
            else {
                continue;
            };
            let Some(port) = inode_port.get(inode).copied() else {
                continue;
            };
            if name.is_none() {
                let comm = std::fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
                name = Some(clean_process_name(comm.trim()));
            }
            let Some(name) = name.as_ref() else {
                continue;
            };
            if !name.is_empty() {
                map.entry(port).or_insert_with(|| name.clone());
            }
        }
    }
    Some(map)
}

#[cfg(all(not(test), target_os = "windows"))]
fn scan_local_ports() -> Option<HashMap<u16, String>> {
    let out = std::process::Command::new("netstat")
        .args(["-ano", "-p", "TCP"])
        .output()
        .ok()?;
    if !out.status.success() && out.stdout.is_empty() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut port_pid: HashMap<u16, u32> = HashMap::new();
    for line in stdout.lines() {
        let mut cols = line.split_whitespace();
        let Some(proto) = cols.next() else {
            continue;
        };
        if !proto.eq_ignore_ascii_case("TCP") {
            continue;
        }
        let Some(local) = cols.next() else {
            continue;
        };
        let Some(_remote) = cols.next() else {
            continue;
        };
        let Some(state) = cols.next() else {
            continue;
        };
        let Some(pid) = cols.next().and_then(|text| text.parse::<u32>().ok()) else {
            continue;
        };
        if !state.eq_ignore_ascii_case("ESTABLISHED") || pid == 0 {
            continue;
        }
        if let Some(port) = local_endpoint_port(local) {
            port_pid.entry(port).or_insert(pid);
        }
    }
    let tlist = std::process::Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .output()
        .ok()?;
    let t_out = String::from_utf8_lossy(&tlist.stdout);
    let mut pid_name: HashMap<u32, String> = HashMap::new();
    for line in t_out.lines() {
        let mut cols = line.split(',');
        let Some(name) = cols.next() else {
            continue;
        };
        let name = name.trim().trim_matches('"');
        let Some(pid) = cols
            .next()
            .map(|text| text.trim().trim_matches('"'))
            .and_then(|text| text.parse::<u32>().ok())
        else {
            continue;
        };
        if !name.is_empty() && pid > 0 {
            pid_name
                .entry(pid)
                .or_insert_with(|| clean_process_name(name));
        }
    }
    let mut map = HashMap::new();
    for (port, pid) in port_pid {
        if let Some(name) = pid_name.get(&pid)
            && !name.is_empty()
        {
            map.insert(port, name.clone());
        }
    }
    Some(map)
}

#[cfg(all(
    not(test),
    not(any(target_os = "macos", target_os = "linux", target_os = "windows"))
))]
fn scan_local_ports() -> Option<HashMap<u16, String>> {
    None
}

/// Identifies the local process name owning `peer_port` on loopback connections.
pub fn resolve_local_process_name(peer_port: u16) -> Option<String> {
    if peer_port == 0 {
        return None;
    }
    if let Some(name) = cached_local_app(peer_port) {
        return Some(name);
    }
    let map = port_snapshot();
    let name = map.get(&peer_port)?.clone();
    if name.is_empty() {
        return None;
    }
    store_cached_local_app(peer_port, name.clone());
    Some(name)
}

#[cfg_attr(test, allow(dead_code))]
fn clean_process_name(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches(".exe");
    trimmed.replace("\\x20", " ")
}

type H2Sender = hyper::client::conn::http2::SendRequest<RecordingBody<Incoming>>;

struct PooledH2 {
    sender: H2Sender,
    used: Instant,
    generation: u64,
    conn_info: crate::session::ConnInfo,
    first_used: Arc<AtomicBool>,
}

type PlainPoolMap = HashMap<(String, u16), Vec<(u64, TcpStream)>>;

pub struct Hub {
    session: Mutex<Session>,
    generation: AtomicU64,
    /// Milliseconds since [`body_clock`] of the last streaming UI bump.
    body_touch_ms: AtomicU64,
    pub ca: Arc<Ca>,
    roots: RootCertStore,
    default_client_config: ClientConfig,
    bypass: Mutex<BypassRules>,
    bypass_text: Mutex<String>,
    rules: Mutex<Vec<Rule>>,
    rules_text: Mutex<String>,
    rules_rev: AtomicU64,
    /// Session-only. Off skips every rule without rewriting the text.
    apply_rules: AtomicBool,
    /// Session-only. Off drops `break` and `break-res` and keeps the other rules.
    apply_breaks: AtomicBool,
    /// Off by default. On records an origin TLS probe that fell back to a tunnel.
    show_tunnel_failures: AtomicBool,
    upstream: Mutex<Option<Upstream>>,
    upstream_text: Mutex<String>,
    proxy_auth: Mutex<String>,
    /// Idle plaintext origin sockets, used only on the proxy runtime.
    pool: tokio::sync::Mutex<PlainPoolMap>,
    /// Origin HTTP/2 senders. Each browser connection keeps its own lane.
    h2_pool: Mutex<HashMap<(String, u16), PooledH2>>,
    h2_generation: AtomicU64,
    origin_epoch: AtomicU64,
    /// Test-only. The next send on a live browser connection dials again.
    #[cfg(test)]
    refresh_h2: AtomicBool,
    break_queue: Mutex<std::collections::VecDeque<BreakSlot>>,
    /// One live HTML share. A new call replaces the id.
    share: Mutex<Option<crate::share::LiveShare>>,
    /// Off until the user turns on 「接收中继」. Uploads stay a 404 until then.
    relay_accept: AtomicBool,
    relay_token: Mutex<String>,
    /// Documents pushed by other zniff processes, keyed by share id.
    relay_in: Mutex<HashMap<String, InboundShare>>,
}

struct InboundShare {
    document: Vec<u8>,
    bodies: HashMap<u64, Vec<u8>>,
    expires: std::time::SystemTime,
}

/// One snapshot the capture process pushes to a relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShareExport {
    pub rev: u64,
    pub document: String,
    pub bodies: Vec<(u64, String)>,
}

pub(crate) fn spilled_flow_ids(document: &str) -> Vec<u64> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(document) else {
        return Vec::new();
    };
    let Some(flows) = value.get("flows").and_then(|item| item.as_array()) else {
        return Vec::new();
    };
    flows
        .iter()
        .filter_map(|flow| {
            let request = flow
                .get("request_body_spilled")
                .and_then(|item| item.as_bool())
                .unwrap_or(false);
            let response = flow
                .get("response_body_spilled")
                .and_then(|item| item.as_bool())
                .unwrap_or(false);
            if request || response {
                flow.get("id").and_then(|item| item.as_u64())
            } else {
                None
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    pub host: String,
    pub port: u16,
    pub auth_header: Option<String>,
}

struct BreakSlot {
    id: u64,
    side: BreakSide,
    touched: Instant,
    reply: Option<tokio::sync::oneshot::Sender<BreakDecision>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakSide {
    Request,
    Response,
}

#[derive(Debug, Clone)]
pub struct BreakDecision {
    pub drop: bool,
    pub headers: Option<Vec<(String, String)>>,
    pub body: Option<Vec<u8>>,
    /// Replaces the response status. Ignored for a request break.
    pub status: Option<u16>,
}

/// Optional edits for one replay. Empty method or URL keeps the stored request.
#[derive(Debug, Default, Clone)]
pub struct ReplayOverride {
    pub method: Option<String>,
    pub url: Option<String>,
    pub headers: Option<Vec<(String, String)>>,
    pub body: Option<Vec<u8>>,
}

impl Hub {
    pub fn new(ca: Arc<Ca>) -> Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let default_client_config = ClientConfig::builder()
            .with_root_certificates(roots.clone())
            .with_no_client_auth();
        Ok(Self {
            session: Mutex::new(Session::default()),
            generation: AtomicU64::new(1),
            body_touch_ms: AtomicU64::new(0),
            ca,
            roots,
            default_client_config,
            bypass: Mutex::new(BypassRules::default()),
            bypass_text: Mutex::new(String::new()),
            rules: Mutex::new(Vec::new()),
            rules_text: Mutex::new(String::new()),
            rules_rev: AtomicU64::new(0),
            apply_rules: AtomicBool::new(true),
            apply_breaks: AtomicBool::new(true),
            show_tunnel_failures: AtomicBool::new(false),
            upstream: Mutex::new(None),
            upstream_text: Mutex::new(String::new()),
            proxy_auth: Mutex::new(String::new()),
            pool: tokio::sync::Mutex::new(HashMap::new()),
            h2_pool: Mutex::new(HashMap::new()),
            h2_generation: AtomicU64::new(1),
            origin_epoch: AtomicU64::new(1),
            #[cfg(test)]
            refresh_h2: AtomicBool::new(false),
            break_queue: Mutex::new(std::collections::VecDeque::new()),
            share: Mutex::new(None),
            relay_accept: AtomicBool::new(false),
            relay_token: Mutex::new(String::new()),
            relay_in: Mutex::new(HashMap::new()),
        })
    }

    pub fn set_relay_ingress(&self, accept: bool, token: &str) {
        self.set_relay_token(token);
        self.relay_accept.store(accept, Ordering::Relaxed);
    }

    pub fn set_relay_token(&self, token: &str) {
        *self
            .relay_token
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = crate::relay::usable_token(token);
    }

    pub fn set_relay_accept(&self, accept: bool) {
        self.relay_accept.store(accept, Ordering::Relaxed);
    }

    pub fn relay_accepts(&self) -> bool {
        self.relay_accept.load(Ordering::Relaxed)
    }

    pub fn relay_token_ready(&self) -> bool {
        !self
            .relay_token
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .is_empty()
    }

    fn relay_authorized(&self, authorization: Option<&str>) -> bool {
        let expected = self
            .relay_token
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        crate::relay::bearer_ok(&expected, authorization)
    }

    /// Replaces any previous link. The id is unguessable and dies at `ttl`.
    #[cfg(test)]
    pub fn begin_share(
        &self,
        ttl: std::time::Duration,
        filter: &str,
    ) -> (String, std::time::SystemTime) {
        self.begin_share_with_redact(ttl, filter, crate::share::RedactOptions::default())
    }

    pub fn begin_share_with_redact(
        &self,
        ttl: std::time::Duration,
        filter: &str,
        redact: crate::share::RedactOptions,
    ) -> (String, std::time::SystemTime) {
        let id = crate::share::new_id();
        let expires = std::time::SystemTime::now() + ttl;
        *self.share.lock().unwrap_or_else(|err| err.into_inner()) = Some(crate::share::LiveShare {
            id: id.clone(),
            expires,
            filter: filter.to_string(),
            redact,
        });
        (id, expires)
    }

    pub fn end_share(&self) {
        *self.share.lock().unwrap_or_else(|err| err.into_inner()) = None;
    }

    /// The current id while it is still before its deadline.
    pub fn share_id(&self) -> Option<String> {
        let mut slot = self.share.lock().unwrap_or_else(|err| err.into_inner());
        let share = slot.as_ref()?;
        if std::time::SystemTime::now() < share.expires {
            return Some(share.id.clone());
        }
        *slot = None;
        None
    }

    #[cfg(test)]
    fn share_document(&self, id: &str) -> Option<String> {
        self.share_document_since(id, None)
    }

    fn share_document_since(&self, id: &str, since: Option<u64>) -> Option<String> {
        let mut slot = self.share.lock().unwrap_or_else(|err| err.into_inner());
        let share = slot.as_ref()?;
        let now = std::time::SystemTime::now();
        if !crate::share::grant_ok(share, id, now) {
            if share.id == id {
                *slot = None;
            }
            return None;
        }
        let expires = share.expires;
        let filter = share.filter.clone();
        let redact = share.redact.clone();
        drop(slot);
        let (plan, rev) =
            self.with_session(|session| (session.plan_filter(&filter), session.rev()));
        let matched = plan.finish();
        let flows = self.with_session(|session| {
            matched
                .iter()
                .filter_map(|(id, _)| session.get(*id).cloned())
                .collect::<Vec<_>>()
        });
        Some(crate::share::document_since_with_redact(
            &flows, expires, &filter, rev, since, &redact,
        ))
    }

    fn share_flow_body(&self, id: &str, flow_id: u64) -> Option<String> {
        let mut slot = self.share.lock().unwrap_or_else(|err| err.into_inner());
        let share = slot.as_ref()?;
        let now = std::time::SystemTime::now();
        if !crate::share::grant_ok(share, id, now) {
            if share.id == id {
                *slot = None;
            }
            return None;
        }
        let filter = share.filter.clone();
        let redact = share.redact.clone();
        drop(slot);
        let flow = self.with_session(|session| session.hydrated_matching(flow_id, &filter))?;
        Some(crate::share::flow_body_document_with_redact(&flow, &redact))
    }

    /// Full share document plus any spilled bodies the page will ask for.
    /// `rev` is the document revision. A later call with the same `rev` has
    /// the same flow list.
    pub(crate) fn share_export(&self, id: &str) -> Option<ShareExport> {
        let document = self.share_document_since(id, None)?;
        let rev = serde_json::from_str::<serde_json::Value>(&document)
            .ok()
            .and_then(|value| value.get("rev")?.as_u64())
            .unwrap_or(0);
        let mut bodies = Vec::new();
        for flow_id in spilled_flow_ids(&document) {
            if let Some(body) = self.share_flow_body(id, flow_id) {
                bodies.push((flow_id, body));
            }
        }
        Some(ShareExport {
            rev,
            document,
            bodies,
        })
    }

    fn put_inbound_flows(&self, id: &str, body: &[u8]) -> Result<(), crate::relay::RelayStatus> {
        let now = std::time::SystemTime::now();
        let (bytes, expires) = crate::relay::prepare_flows(body, now)?;
        let parsed =
            crate::share::parse_inbound_share(&bytes).ok_or(crate::relay::RelayStatus {
                status: 400,
                reason: "Bad Request",
                message: "flows.json must be a JSON object",
            })?;
        self.gc_inbound(now);
        {
            let mut slots = self.relay_in.lock().unwrap_or_else(|err| err.into_inner());
            if !slots.contains_key(id) && slots.len() >= crate::relay::SLOT_CAP {
                return Err(crate::relay::RelayStatus {
                    status: 429,
                    reason: "Too Many Requests",
                    message: "too many shares",
                });
            }
            match slots.get_mut(id) {
                Some(slot) => {
                    slot.document = bytes;
                    slot.expires = expires;
                }
                None => {
                    slots.insert(
                        id.to_string(),
                        InboundShare {
                            document: bytes,
                            bodies: HashMap::new(),
                            expires,
                        },
                    );
                }
            }
        }
        let drafts = relay_drafts(&parsed);
        self.with_session(|session| session.sync_relay_share(id, drafts));
        self.touch();
        Ok(())
    }

    fn put_inbound_body(
        &self,
        id: &str,
        flow_id: u64,
        body: &[u8],
    ) -> Result<(), crate::relay::RelayStatus> {
        if body.is_empty() || body.len() > crate::relay::MAX_BODY {
            return Err(crate::relay::RelayStatus {
                status: 413,
                reason: "Content Too Large",
                message: "body is too large",
            });
        }
        if serde_json::from_slice::<serde_json::Value>(body).is_err() {
            return Err(crate::relay::RelayStatus {
                status: 400,
                reason: "Bad Request",
                message: "flow body must be JSON",
            });
        }
        let now = std::time::SystemTime::now();
        self.gc_inbound(now);
        {
            let mut slots = self.relay_in.lock().unwrap_or_else(|err| err.into_inner());
            let Some(slot) = slots.get_mut(id) else {
                return Err(crate::relay::RelayStatus {
                    status: 409,
                    reason: "Conflict",
                    message: "upload flows.json first",
                });
            };
            slot.bodies.insert(flow_id, body.to_vec());
        }
        if let Some(parsed) = crate::share::parse_inbound_body(body) {
            let messages = shared_messages(&parsed.messages);
            self.with_session(|session| {
                session.apply_relay_body(
                    id,
                    flow_id,
                    parsed.request_body.into_bytes(),
                    parsed.response_body.into_bytes(),
                    messages,
                );
            });
            self.touch();
        }
        Ok(())
    }

    fn delete_inbound(&self, id: &str) {
        self.relay_in
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(id);
        self.with_session(|session| {
            session.drop_relay_source(id);
        });
        self.touch();
    }

    fn gc_inbound(&self, now: std::time::SystemTime) {
        let expired: Vec<String> = {
            let mut slots = self.relay_in.lock().unwrap_or_else(|err| err.into_inner());
            let expired: Vec<String> = slots
                .iter()
                .filter(|(_, slot)| now >= slot.expires)
                .map(|(id, _)| id.clone())
                .collect();
            for id in &expired {
                slots.remove(id);
            }
            expired
        };
        if expired.is_empty() {
            return;
        }
        self.with_session(|session| {
            for id in &expired {
                session.drop_relay_source(id);
            }
        });
        self.touch();
    }

    fn inbound_document(&self, id: &str) -> Option<String> {
        let now = std::time::SystemTime::now();
        self.gc_inbound(now);
        let slots = self.relay_in.lock().unwrap_or_else(|err| err.into_inner());
        let slot = slots.get(id)?;
        if now >= slot.expires {
            return None;
        }
        Some(String::from_utf8_lossy(&slot.document).into_owned())
    }

    fn inbound_body(&self, id: &str, flow_id: u64) -> Option<Vec<u8>> {
        let now = std::time::SystemTime::now();
        self.gc_inbound(now);
        let slots = self.relay_in.lock().unwrap_or_else(|err| err.into_inner());
        let slot = slots.get(id)?;
        if now >= slot.expires {
            return None;
        }
        slot.bodies.get(&flow_id).cloned()
    }

    pub fn set_bypass(&self, text: &str) {
        *self
            .bypass_text
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = text.to_string();
        *self.bypass.lock().unwrap_or_else(|err| err.into_inner()) = BypassRules::parse(text);
    }

    pub fn bypass_text(&self) -> String {
        self.bypass_text
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub fn set_rules(&self, text: &str) {
        *self
            .rules_text
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = text.to_string();
        let next_rules = rules::parse_rules(text);
        let client_certs_changed = {
            let mut guard = self.rules.lock().unwrap_or_else(|err| err.into_inner());
            let changed = client_cert_rules(&guard) != client_cert_rules(&next_rules);
            *guard = next_rules;
            changed
        };
        self.rules_rev.fetch_add(1, Ordering::Relaxed);
        if client_certs_changed {
            self.invalidate_origin_pools();
        }
    }

    pub fn rules_revision(&self) -> u64 {
        self.rules_rev.load(Ordering::Relaxed)
    }

    pub fn rules_text(&self) -> String {
        self.rules_text
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub fn set_rules_enabled(&self, enabled: bool) {
        let prev = self.apply_rules.swap(enabled, Ordering::Relaxed);
        if prev != enabled {
            let has_client_certs = self
                .rules
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .iter()
                .any(|r| matches!(r, rules::Rule::ClientCert { .. }));
            if has_client_certs {
                self.invalidate_origin_pools();
            }
        }
    }

    pub fn rules_enabled(&self) -> bool {
        self.apply_rules.load(Ordering::Relaxed)
    }

    pub fn set_breaks_enabled(&self, enabled: bool) {
        self.apply_breaks.store(enabled, Ordering::Relaxed);
    }

    pub fn breaks_enabled(&self) -> bool {
        self.apply_breaks.load(Ordering::Relaxed)
    }

    pub fn set_show_tunnel_failures(&self, show: bool) {
        self.show_tunnel_failures.store(show, Ordering::Relaxed);
    }

    pub fn tunnel_failures_shown(&self) -> bool {
        self.show_tunnel_failures.load(Ordering::Relaxed)
    }

    pub fn set_upstream(&self, text: &str) {
        *self
            .upstream_text
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = text.to_string();
        let next = parse_upstream(text);
        let changed = {
            let mut guard = self.upstream.lock().unwrap_or_else(|err| err.into_inner());
            let changed = *guard != next;
            *guard = next;
            changed
        };
        if changed {
            self.invalidate_origin_pools();
        }
    }

    fn invalidate_origin_pools(&self) {
        self.origin_epoch.fetch_add(1, Ordering::AcqRel);
        self.h2_pool
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clear();
        if let Ok(mut pool) = self.pool.try_lock() {
            pool.clear();
        }
    }

    pub fn upstream_text(&self) -> String {
        self.upstream_text
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    fn upstream(&self) -> Option<Upstream> {
        self.upstream
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub fn set_proxy_auth(&self, auth: &str) {
        *self
            .proxy_auth
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = auth.trim().to_string();
    }

    pub fn proxy_auth_text(&self) -> String {
        self.proxy_auth
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    fn proxy_auth_ok(&self, headers: &[(String, String)]) -> bool {
        let expected_raw = self
            .proxy_auth
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        if expected_raw.is_empty() {
            return true;
        }
        let expected = if expected_raw.contains(':') {
            expected_raw
        } else {
            format!("{expected_raw}:")
        };
        headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("proxy-authorization"))
            .any(|(_, value)| verify_basic_auth(&expected, value))
    }

    fn skips_tls(&self, host: &str) -> bool {
        self.bypass
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .skips(host)
    }

    fn has_op_rules(&self, host: &str, path: &str) -> bool {
        if !self.apply_rules.load(Ordering::Relaxed) {
            return false;
        }
        let rules = self
            .rules
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        rules::has_op_rules(&rules, host, path)
    }

    #[cfg(test)]
    fn plan(&self, host: &str, path: &str) -> rules::Effect {
        self.plan_with_ops(host, path, &[])
    }

    fn plan_with_ops(&self, host: &str, path: &str, ops: &[String]) -> rules::Effect {
        if !self.apply_rules.load(Ordering::Relaxed) {
            return rules::Effect::default();
        }
        let rules = self
            .rules
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        let effect = rules::effect_with_ops(&rules, host, path, ops);
        if self.apply_breaks.load(Ordering::Relaxed) {
            effect
        } else {
            rules::without_breaks(effect)
        }
    }

    fn server_config_for_host(&self, host: &str, alpn: &[u8]) -> Result<rustls::ServerConfig> {
        let rule_cert = if self.apply_rules.load(Ordering::Relaxed) {
            let rules = self
                .rules
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone();
            rules::custom_cert(&rules, host)
        } else {
            None
        };
        let custom_ref = rule_cert
            .as_ref()
            .map(|(cert, key)| (cert.as_path(), key.as_deref()));
        self.ca.server_config_with_custom(host, alpn, custom_ref)
    }

    fn has_client_cert_for_host(&self, host: &str) -> bool {
        if self.apply_rules.load(Ordering::Relaxed) {
            let rules = self
                .rules
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone();
            if rules::custom_client_cert(&rules, host).is_some() {
                return true;
            }
        }
        self.ca.has_client_cert_in_dir(host)
    }

    fn client_config_for_host(
        &self,
        host: &str,
        alpn: &[Vec<u8>],
    ) -> Result<(ClientConfig, Option<String>)> {
        let rule_cert = if self.apply_rules.load(Ordering::Relaxed) {
            let rules = self
                .rules
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone();
            rules::custom_client_cert(&rules, host)
        } else {
            None
        };
        let custom_ref = rule_cert
            .as_ref()
            .map(|(cert, key)| (cert.as_path(), key.as_deref()));
        let identity = self
            .ca
            .client_identity_for_host(host, custom_ref)
            .with_context(|| format!("加载源站 mTLS 客户端证书 ({host})"))?;
        let mut presented_cert = None;
        let mut config = match identity {
            Some(id) => {
                let cert_display = id.cert_path.display().to_string();
                let key_display = id.key_path.as_ref().map(|p| p.display().to_string());
                let mut cfg = ClientConfig::builder()
                    .with_root_certificates(self.roots.clone())
                    .with_client_auth_cert(id.certs, id.key)
                    .with_context(|| match &key_display {
                        Some(kp) => format!(
                            "配置源站 mTLS 客户端证书 ({host}, cert={cert_display}, key={kp})"
                        ),
                        None => {
                            format!("配置源站 mTLS 客户端证书 ({host}, pem={cert_display})")
                        }
                    })?;
                cfg.resumption = self.default_client_config.resumption.clone();
                presented_cert = Some(cert_display);
                cfg
            }
            None => self.default_client_config.clone(),
        };
        config.alpn_protocols = alpn.to_vec();
        Ok((config, presented_cert))
    }

    #[cfg(test)]
    fn note_rules(&self, id: u64, host: &str, path: &str) {
        self.note_rules_with_ops(id, host, path, &[]);
    }

    fn note_rules_with_ops(&self, id: u64, host: &str, path: &str, ops: &[String]) {
        if !self.apply_rules.load(Ordering::Relaxed) {
            return;
        }
        let rules = self
            .rules
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        let mut hit = rules::hit_summary_with_ops(&rules, host, path, ops);
        if !self.apply_breaks.load(Ordering::Relaxed) {
            hit = rules::without_break_labels(&hit);
        }
        if !hit.is_empty() {
            self.with_session(|session| session.set_rule_hit(id, hit));
            self.touch();
        }
    }

    pub fn paused(&self) -> Option<(u64, BreakSide)> {
        self.break_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .front()
            .map(|slot| (slot.id, slot.side))
    }

    pub fn paused_queue_len(&self) -> usize {
        self.break_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .len()
    }

    /// Ids and sides from front to back. The front is the flow `resolve_break` edits.
    pub fn paused_queue(&self) -> Vec<(u64, BreakSide)> {
        self.break_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .iter()
            .map(|slot| (slot.id, slot.side))
            .collect()
    }

    /// Moves `id` to the front and restarts its 60s timer. Other slots stay paused.
    pub fn promote_break(&self, id: u64) -> bool {
        let mut queue = self
            .break_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let Some(index) = queue.iter().position(|slot| slot.id == id) else {
            return false;
        };
        if index > 0
            && let Some(slot) = queue.remove(index)
        {
            queue.push_front(slot);
        }
        if let Some(front) = queue.front_mut() {
            front.touched = Instant::now();
        }
        drop(queue);
        self.touch();
        true
    }

    #[cfg(test)]
    pub fn paused_id(&self) -> Option<u64> {
        self.paused().map(|(id, _)| id)
    }

    pub fn set_pinned(&self, id: u64, pinned: bool) -> bool {
        let ok = self.with_session(|session| session.set_pinned(id, pinned));
        if ok {
            self.touch();
        }
        ok
    }

    pub fn set_comment(&self, id: u64, comment: impl Into<String>) -> bool {
        let ok = self.with_session(|session| session.set_comment(id, comment));
        if ok {
            self.touch();
        }
        ok
    }

    pub fn append_comment(&self, id: u64, note: &str) {
        let changed = self.with_session(|session| {
            let Some(flow) = session.get(id) else {
                return false;
            };
            if flow.comment.contains(note) {
                return false;
            }
            let next = if flow.comment.is_empty() {
                note.to_string()
            } else {
                format!("{} {note}", flow.comment)
            };
            session.set_comment(id, next)
        });
        if changed {
            self.touch();
        }
    }

    pub fn resolve_break(&self, decision: BreakDecision) -> bool {
        let mut queue = self
            .break_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let Some(slot) = queue.pop_front() else {
            return false;
        };
        if let Some(next) = queue.front_mut() {
            next.touched = Instant::now();
        }
        drop(queue);
        let ok = slot
            .reply
            .map(|reply| reply.send(decision).is_ok())
            .unwrap_or(false);
        self.touch();
        ok
    }

    /// Releases every paused flow in the breakpoint FIFO queue without dropping them.
    pub fn resolve_all_breaks(&self) -> usize {
        let slots: Vec<BreakSlot> = self
            .break_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .drain(..)
            .collect();
        let count = slots.len();
        for slot in slots {
            if let Some(reply) = slot.reply {
                let _ = reply.send(BreakDecision {
                    drop: false,
                    headers: None,
                    body: None,
                    status: None,
                });
            }
        }
        if count > 0 {
            self.touch();
        }
        count
    }

    pub fn touch_break(&self) {
        if let Some(slot) = self
            .break_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .front_mut()
        {
            slot.touched = Instant::now();
        }
    }

    async fn wait_break(&self, id: u64, side: BreakSide) -> BreakDecision {
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        {
            let mut queue = self
                .break_queue
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            if queue.len() >= BREAK_QUEUE_CAP {
                // Evict oldest paused flow so its oneshot sender drops and auto-continues.
                let _ = queue.pop_front();
                if let Some(next) = queue.front_mut() {
                    next.touched = Instant::now();
                }
            }
            queue.push_back(BreakSlot {
                id,
                side,
                touched: Instant::now(),
                reply: Some(tx),
            });
        }
        self.touch();
        let decision = loop {
            let remaining = {
                let mut queue = self
                    .break_queue
                    .lock()
                    .unwrap_or_else(|err| err.into_inner());
                match queue.iter_mut().enumerate().find(|(_, slot)| slot.id == id) {
                    Some((0, slot)) => {
                        Duration::from_secs(60).saturating_sub(slot.touched.elapsed())
                    }
                    Some((_, slot)) => {
                        slot.touched = Instant::now();
                        Duration::from_millis(200)
                    }
                    None => Duration::ZERO,
                }
            };
            if remaining.is_zero() {
                break BreakDecision {
                    drop: false,
                    headers: None,
                    body: None,
                    status: None,
                };
            }
            match timeout(remaining, &mut rx).await {
                Ok(Ok(decision)) => break decision,
                Ok(Err(_)) => {
                    break BreakDecision {
                        drop: false,
                        headers: None,
                        body: None,
                        status: None,
                    };
                }
                Err(_) => continue,
            }
        };
        {
            let mut queue = self
                .break_queue
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            let was_front = queue.front().map(|slot| slot.id) == Some(id);
            queue.retain(|slot| slot.id != id);
            if was_front && let Some(next) = queue.front_mut() {
                next.touched = Instant::now();
            }
        }
        self.touch();
        decision
    }

    fn cached_h2(
        &self,
        key: &(String, u16),
    ) -> Option<(H2Sender, u64, crate::session::ConnInfo, Arc<AtomicBool>)> {
        let mut pool = self.h2_pool.lock().unwrap_or_else(|err| err.into_inner());
        let expired = pool
            .get(key)
            .is_some_and(|entry| entry.used.elapsed() > H2_IDLE);
        if expired {
            pool.remove(key);
            return None;
        }
        let entry = pool.get_mut(key)?;
        entry.used = Instant::now();
        Some((
            entry.sender.clone(),
            entry.generation,
            entry.conn_info.clone(),
            Arc::clone(&entry.first_used),
        ))
    }

    /// Returns the generation stored with this sender. Replacing a key does
    /// not wipe the other origins. A full pool drops every entry only when
    /// the key is new.
    fn store_h2(
        &self,
        key: (String, u16),
        sender: H2Sender,
        conn_info: crate::session::ConnInfo,
    ) -> (u64, Arc<AtomicBool>) {
        let generation = self.h2_generation.fetch_add(1, Ordering::Relaxed);
        let first_used = Arc::new(AtomicBool::new(false));
        let mut pool = self.h2_pool.lock().unwrap_or_else(|err| err.into_inner());
        let replacing = pool.contains_key(&key);
        if !replacing && pool.len() >= 16 {
            pool.clear();
        }
        pool.insert(
            key,
            PooledH2 {
                sender,
                used: Instant::now(),
                generation,
                conn_info,
                first_used: Arc::clone(&first_used),
            },
        );
        (generation, first_used)
    }

    /// Removes the entry only when it is still this generation, so an old
    /// connection task cannot delete the replacement.
    fn drop_h2_generation(&self, key: &(String, u16), generation: u64) {
        let mut pool = self.h2_pool.lock().unwrap_or_else(|err| err.into_inner());
        let matches = pool
            .get(key)
            .is_some_and(|entry| entry.generation == generation);
        if matches {
            pool.remove(key);
        }
    }

    fn touch_h2(&self, key: &(String, u16), generation: u64) {
        let mut pool = self.h2_pool.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(entry) = pool.get_mut(key)
            && entry.generation == generation
        {
            entry.used = Instant::now();
        }
    }

    #[cfg(test)]
    fn expire_h2_cache(&self) {
        let stale = Instant::now()
            .checked_sub(H2_IDLE + Duration::from_secs(1))
            .expect("idle stamp");
        let mut pool = self.h2_pool.lock().unwrap_or_else(|err| err.into_inner());
        for entry in pool.values_mut() {
            entry.used = stale;
        }
    }

    #[cfg(test)]
    fn take_h2_refresh(&self) -> bool {
        self.refresh_h2.swap(false, Ordering::AcqRel)
    }

    async fn take_idle(&self, key: (String, u16)) -> Option<TcpStream> {
        let current_epoch = self.origin_epoch.load(Ordering::Acquire);
        let mut pool = self.pool.lock().await;
        let slots = pool.get_mut(&key)?;
        while let Some((epoch, stream)) = slots.pop() {
            if epoch != current_epoch {
                continue;
            }
            let mut probe = [0u8; 1];
            match stream.try_read(&mut probe) {
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => return Some(stream),
                _ => continue,
            }
        }
        None
    }

    async fn park_idle(&self, key: (String, u16), stream: TcpStream) {
        let mut probe = [0u8; 1];
        if !matches!(
            stream.try_read(&mut probe),
            Err(err) if err.kind() == io::ErrorKind::WouldBlock
        ) {
            return;
        }
        let epoch = self.origin_epoch.load(Ordering::Acquire);
        let mut pool = self.pool.lock().await;
        if pool.len() >= 32 && !pool.contains_key(&key) {
            pool.clear();
        }
        let slots = pool.entry(key).or_default();
        if slots.len() >= 4 {
            slots.remove(0);
        }
        slots.push((epoch, stream));
    }

    #[cfg(test)]
    pub fn add_trust_anchor(&mut self, der: CertificateDer<'static>) {
        let _ = self.roots.add(der);
        self.default_client_config = ClientConfig::builder()
            .with_root_certificates(self.roots.clone())
            .with_no_client_auth();
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    pub fn with_session<R>(&self, f: impl FnOnce(&mut Session) -> R) -> R {
        let mut session = self.session.lock().unwrap_or_else(|err| err.into_inner());
        f(&mut session)
    }

    fn touch(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// At most one list refresh per [`BODY_UI_TOUCH`] while a body is streaming.
    fn touch_body(&self) {
        let now = body_clock_ms();
        let prev = self.body_touch_ms.load(Ordering::Relaxed);
        if now.saturating_sub(prev) < BODY_UI_TOUCH.as_millis() as u64 {
            return;
        }
        if self
            .body_touch_ms
            .compare_exchange(prev, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            self.touch();
        }
    }

    fn record_chunk(
        &self,
        id: u64,
        direction: Direction,
        wire: u64,
        decoded: &[u8],
        encoding_error: Option<&str>,
    ) {
        if wire == 0 && decoded.is_empty() && encoding_error.is_none() {
            return;
        }
        self.with_session(|session| {
            if wire > 0 {
                session.add_transfer(id, direction, wire);
            }
            if !decoded.is_empty() {
                session.append_body(id, direction, decoded);
            }
            if let Some(error) = encoding_error {
                session.set_encoding_error(id, direction, error);
            }
        });
        if encoding_error.is_some() {
            self.touch();
        } else {
            self.touch_body();
        }
    }

    /// Ids that match `filter`, oldest first, with the session revision taken
    /// in the same lock as the plan. Spill reads happen in `finish`, outside the lock.
    pub fn matching_ids(&self, filter: &str) -> (Vec<(u64, Option<std::time::SystemTime>)>, u64) {
        let (plan, rev) = self.with_session(|session| (session.plan_filter(filter), session.rev()));
        (plan.finish(), rev)
    }

    pub(crate) fn open(&self, draft: FlowDraft) -> u64 {
        let ctx = current_client_ctx();
        let client_ip = ctx
            .as_ref()
            .map(|c| c.label.lock().unwrap_or_else(|e| e.into_inner()).clone())
            .unwrap_or_default();
        let id = self.with_session(|session| {
            let id = session.open(draft);
            if !client_ip.is_empty() {
                session.set_client_ip(id, client_ip);
            }
            id
        });
        if let Some(c) = ctx {
            c.opened_ids
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(id);
        }
        self.touch();
        id
    }

    fn set_response(
        &self,
        id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        protocol: Option<Protocol>,
    ) {
        self.with_session(|session| session.set_response(id, status, headers, protocol));
        self.touch();
    }

    fn clear_response(&self, id: u64) {
        self.with_session(|session| session.clear_response(id));
        self.touch();
    }

    fn set_conn_info(&self, id: u64, conn_info: crate::session::ConnInfo) {
        self.with_session(|session| session.set_conn_info(id, conn_info));
        self.touch();
    }

    fn set_error(&self, id: u64, error: impl Into<String>) {
        self.with_session(|session| session.set_error(id, error));
        self.touch();
    }

    fn append_body(&self, id: u64, direction: Direction, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        self.with_session(|session| session.append_body(id, direction, data));
        self.touch_body();
    }

    fn add_transfer(&self, id: u64, direction: Direction, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.with_session(|session| session.add_transfer(id, direction, bytes));
        self.touch_body();
    }

    pub fn mark_finished(&self, id: u64) {
        let flushed = self
            .with_session(|session| session.begin_finish(id, std::time::SystemTime::now()))
            .flush();
        if !flushed.is_empty() {
            self.with_session(|session| session.commit_finish(id, flushed));
        }
        self.touch();
    }

    pub fn clear_session(&self) {
        self.with_session(|session| session.clear());
        self.touch();
    }

    pub fn clear_unpinned(&self) -> usize {
        let removed = self.with_session(|session| session.clear_unpinned());
        if removed > 0 {
            self.touch();
        }
        removed
    }

    /// Exports the flows the list is showing. An empty filter exports the session.
    pub fn export_har_matching_with_redact(
        &self,
        filter: &str,
        redact: &crate::share::RedactOptions,
    ) -> serde_json::Value {
        let (matched, _) = self.matching_ids(filter);
        let flows = self.with_session(|session| {
            matched
                .into_iter()
                .filter_map(|(id, _)| session.hydrated(id))
                .collect::<Vec<_>>()
        });
        crate::har::export_har_with_redact(&flows, redact)
    }

    pub fn import_har(&self, value: &serde_json::Value) -> usize {
        let items = crate::har::import_har(value);
        let count = items.len();
        for item in items {
            let id = self.open(FlowDraft {
                protocol: item.protocol,
                method: item.method,
                host: item.host,
                path: item.path,
                url: item.url,
                request_headers: item.request_headers,
            });
            self.with_session(|session| session.set_started(id, item.started));
            if let Some(status) = item.status {
                self.set_response(id, status, item.response_headers, Some(item.protocol));
            }
            self.append_body(id, Direction::Request, &item.request_body);
            self.append_body(id, Direction::Response, &item.response_body);
            if let Some(error) = item.error {
                self.set_error(id, error);
            }
            self.mark_finished(id);
            self.with_session(|session| {
                session.restore_imported(
                    id,
                    item.response_at,
                    item.finished_at,
                    item.request_bytes,
                    item.response_bytes,
                );
                if let Some(comment) = item.comment {
                    session.set_comment(id, comment);
                }
                if !item.conn_info.is_empty() {
                    session.set_conn_info(id, item.conn_info);
                }
                for msg in item.messages {
                    session.add_message(id, msg.direction, msg.kind, msg.text);
                }
                if item.messages_truncated {
                    session.mark_messages_truncated(id);
                }
            });
            self.touch();
        }
        count
    }

    /// Replays one stored request on a fresh runtime. Map and break rules are
    /// not applied. The callback runs on the worker thread and may only touch
    /// mutexes, not nsh signals.
    pub fn replay_later<F>(self: &Arc<Self>, id: u64, edit: ReplayOverride, on_done: F)
    where
        F: FnOnce(Result<u64>) + Send + 'static,
    {
        let flow = self.with_session(|session| session.hydrated(id));
        let Some(flow) = flow else {
            on_done(Err(anyhow!("没有这条记录")));
            return;
        };
        if flow.protocol == Protocol::Tunnel || flow.method.eq_ignore_ascii_case("CONNECT") {
            on_done(Err(anyhow!("隧道不能重放")));
            return;
        }
        if flow.relay_source.is_some() {
            on_done(Err(anyhow!("中继流量只能查看")));
            return;
        }
        let hub = Arc::clone(self);
        std::thread::spawn(move || {
            let result = (|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("重放运行时")?;
                runtime.block_on(replay_once(hub, Some(flow), edit))
            })();
            on_done(result);
        });
    }

    /// Replays one stored request `count` times (with up to `concurrency` parallel
    /// workers) and `interval_ms` between launches on a fresh runtime. Map and break
    /// rules are not applied.
    pub fn replay_batch_later<F>(
        self: &Arc<Self>,
        id: u64,
        count: u32,
        concurrency: u32,
        interval_ms: u64,
        edit: ReplayOverride,
        on_done: F,
    ) where
        F: FnOnce(Result<Vec<u64>>) + Send + 'static,
    {
        let flow = self.with_session(|session| session.hydrated(id));
        let Some(flow) = flow else {
            on_done(Err(anyhow!("没有这条记录")));
            return;
        };
        if flow.protocol == Protocol::Tunnel || flow.method.eq_ignore_ascii_case("CONNECT") {
            on_done(Err(anyhow!("隧道不能重放")));
            return;
        }
        if flow.relay_source.is_some() {
            on_done(Err(anyhow!("中继流量只能查看")));
            return;
        }
        let hub = Arc::clone(self);
        std::thread::spawn(move || {
            let result = (|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("重放运行时")?;
                runtime.block_on(replay_batch(
                    hub,
                    flow,
                    count,
                    concurrency,
                    interval_ms,
                    edit,
                ))
            })();
            on_done(result);
        });
    }

    /// Sends a standalone composed request on a fresh runtime without a source
    /// flow. Map and break rules are not applied.
    pub fn send_composed_later<F>(self: &Arc<Self>, edit: ReplayOverride, on_done: F)
    where
        F: FnOnce(Result<u64>) + Send + 'static,
    {
        let hub = Arc::clone(self);
        std::thread::spawn(move || {
            let result = (|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("重放运行时")?;
                runtime.block_on(replay_once(hub, None, edit))
            })();
            on_done(result);
        });
    }

    fn add_message(&self, id: u64, direction: Direction, kind: &str, text: String) {
        self.with_session(|session| session.add_message(id, direction, kind, text));
        self.touch();
    }

    fn retarget(&self, id: u64, host: String, path: String, url: String) {
        self.with_session(|session| session.retarget(id, host, path, url));
        self.touch();
    }

    fn replace_request(&self, id: u64, headers: Vec<(String, String)>, body: Vec<u8>) {
        self.with_session(|session| session.replace_request(id, headers, body));
        self.touch();
    }

    fn replace_response(
        &self,
        id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) {
        self.with_session(|session| session.replace_response(id, status, headers, body));
        self.touch();
    }

    fn replace_response_head(&self, id: u64, status: u16, headers: Vec<(String, String)>) {
        self.with_session(|session| {
            let body = session
                .get(id)
                .map(|flow| flow.response_body.clone())
                .unwrap_or_default();
            session.replace_response(id, status, headers, body);
        });
        self.touch();
    }

    fn set_encoding_error(&self, id: u64, direction: Direction, error: impl Into<String>) {
        self.with_session(|session| session.set_encoding_error(id, direction, error));
        self.touch();
    }

    fn mark_protocol(&self, id: u64, protocol: Protocol) {
        self.with_session(|session| session.set_protocol(id, protocol));
        self.touch();
    }
}

pub async fn serve(listener: TcpListener, hub: Arc<Hub>, mut shutdown: watch::Receiver<bool>) {
    log::info!(
        "代理监听 {}",
        listener
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_default()
    );
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                let Ok((socket, _)) = accepted else { continue };
                let hub = hub.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_conn(socket, hub).await {
                        log::debug!("连接结束: {err:#}");
                    }
                });
            }
        }
    }
}

pub struct Capture {
    pub addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Capture {
    pub fn spawn(hub: Arc<Hub>, addr: SocketAddr) -> Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel();
        let (shutdown, shutdown_rx) = watch::channel(false);
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            runtime.block_on(async move {
                let listener = match TcpListener::bind(addr).await {
                    Ok(listener) => listener,
                    Err(err) => {
                        let _ = tx.send(Err(err));
                        return;
                    }
                };
                let addr = match listener.local_addr() {
                    Ok(addr) => addr,
                    Err(err) => {
                        let _ = tx.send(Err(err));
                        return;
                    }
                };
                let _ = tx.send(Ok(addr));
                serve(listener, hub, shutdown_rx).await;
            });
        });
        let addr = rx.recv().context("代理线程没有回报端口")??;
        Ok(Self {
            addr,
            shutdown,
            thread: Some(thread),
        })
    }

    pub fn stop(mut self) {
        let _ = self.shutdown.send(true);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Binds on the caller's runtime and returns the listener. Tests and the
/// capture thread use this directly.
#[cfg(test)]
pub async fn listen() -> Result<TcpListener> {
    Ok(TcpListener::bind("127.0.0.1:0").await?)
}

async fn handle_conn(socket: TcpStream, hub: Arc<Hub>) -> Result<()> {
    let peer = socket.peer_addr().ok();
    let base_ip = peer
        .map(|addr| addr.ip().to_canonical().to_string())
        .unwrap_or_default();
    let mut initial_label = base_ip.clone();
    let mut lookup_port = None;
    if let Some(addr) = peer
        && addr.ip().is_loopback()
    {
        if let Some(app) = cached_local_app(addr.port()) {
            initial_label = format!("{base_ip} ({app})");
        } else {
            lookup_port = Some(addr.port());
        }
    }
    let ctx = Arc::new(ClientConnCtx {
        label: Mutex::new(initial_label),
        opened_ids: Mutex::new(Vec::new()),
    });
    #[cfg(not(test))]
    if let Some(port) = lookup_port {
        let ctx_bg = Arc::clone(&ctx);
        let hub_bg = Arc::clone(&hub);
        let ip_bg = base_ip;
        tokio::task::spawn_blocking(move || {
            if let Some(app) = resolve_local_process_name(port) {
                store_cached_local_app(port, app.clone());
                let full = format!("{ip_bg} ({app})");
                *ctx_bg.label.lock().unwrap_or_else(|e| e.into_inner()) = full.clone();
                let ids = ctx_bg
                    .opened_ids
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                if !ids.is_empty() {
                    hub_bg.with_session(|session| {
                        for id in ids {
                            session.set_client_ip(id, full.clone());
                        }
                    });
                    hub_bg.touch();
                }
            }
        });
    }
    #[cfg(test)]
    let _ = lookup_port;
    CLIENT_CTX.scope(ctx, handle_conn_inner(socket, hub)).await
}

async fn handle_conn_inner(mut socket: TcpStream, hub: Arc<Hub>) -> Result<()> {
    socket.set_nodelay(true).ok();
    let local_addr = socket.local_addr().ok();
    let (head_bytes, rest) = read_header_block(&mut socket).await?;
    let head = parse_head(&head_bytes)?;
    let mut parts = head.start.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let self_target = targets_self(local_addr, &head, &method, &target);
    match local_route_for(&method, &target, self_target) {
        Some(LocalRoute::Cert) => return serve_ca_cert(&mut socket, &hub, &head).await,
        Some(LocalRoute::Install) => return serve_install_page(&mut socket, &hub, &head).await,
        Some(LocalRoute::Share(id, kind)) => {
            return serve_share(&mut socket, &hub, &head, &id, kind).await;
        }
        Some(LocalRoute::Relay(id, kind, verb)) => {
            return serve_relay(&mut socket, rest, &hub, &head, &id, kind, verb).await;
        }
        Some(LocalRoute::Mcp) => {
            return serve_mcp(&mut socket, rest, hub, &method, &head).await;
        }
        Some(LocalRoute::Favicon) => {
            return write_untracked(&mut socket, 204, "No Content", "image/x-icon", b"").await;
        }
        None if self_target => {
            return write_untracked_encoded_for_method(
                &mut socket,
                404,
                "Not Found",
                "text/plain; charset=utf-8",
                None,
                b"Not Found",
                method.eq_ignore_ascii_case("HEAD"),
            )
            .await;
        }
        None => {}
    }
    if !hub.proxy_auth_ok(&head.headers) {
        return write_proxy_auth_required(&mut socket, method.eq_ignore_ascii_case("HEAD")).await;
    }
    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = split_host_port(&target, 443)?;
        socket
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
        let reader = Prefixed {
            prefix: rest,
            pos: 0,
            inner: socket,
        };
        mitm_connect(reader, &host, port, hub).await
    } else {
        let mut reader = Prefixed {
            prefix: rest,
            pos: 0,
            inner: socket,
        };
        plain_http(&mut reader, &head, hub).await
    }
}

async fn mitm_connect<S>(mut client: S, host: &str, port: u16, hub: Arc<Hub>) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let hello = match timeout(IO_TIMEOUT, read_client_hello(&mut client)).await {
        Ok(Ok(hello)) => hello,
        Ok(Err(err)) => {
            record_tunnel(&hub, host, port, format!("读取 ClientHello 失败: {err}"));
            return Err(err.into());
        }
        Err(_) => {
            record_tunnel(&hub, host, port, "读取 ClientHello 超时".into());
            return Err(anyhow!("读取 ClientHello 超时"));
        }
    };
    let ReadHello::Hello(hello) = hello else {
        let ReadHello::NotTls { raw } = hello else {
            unreachable!();
        };
        record_tunnel(&hub, host, port, "不是 TLS，已改为隧道".into());
        return tunnel_after_hello(&mut client, raw, host, port, &hub)
            .await
            .map(|_| ());
    };
    let sni_host = hello.sni.clone().unwrap_or_else(|| host.to_string());
    if hub.skips_tls(&sni_host) {
        record_tunnel(&hub, &sni_host, port, "直通名单，未解密".into());
        return tunnel_after_hello(&mut client, hello.raw, host, port, &hub)
            .await
            .map(|_| ());
    }
    let offers_h2 = hello.alpn.iter().any(|proto| proto == b"h2");
    if offers_h2
        && let Some((mut sender, generation, conn_info, first_used)) =
            hub.cached_h2(&(sni_host.clone(), port))
    {
        let ready = timeout(Duration::from_secs(2), sender.ready()).await;
        if matches!(ready, Ok(Ok(()))) {
            return intercept_h2_pooled(
                client, hello.raw, sender, generation, conn_info, first_used, &sni_host, port, hub,
            )
            .await;
        }
        hub.drop_h2_generation(&(sni_host.clone(), port), generation);
    }
    let alpn = if hello.alpn.is_empty() {
        vec![b"http/1.1".to_vec()]
    } else {
        hello.alpn.clone()
    };
    if let Err(err) = hub.client_config_for_host(&sni_host, &alpn) {
        return fail_client_cert_connect(client, hello.raw, &sni_host, port, &alpn, err, hub).await;
    }
    match timeout(IO_TIMEOUT, connect_origin(&sni_host, port, &alpn, &hub)).await {
        Ok(Ok((origin, selected, mut conn_info))) => {
            conn_info.dialed_before_start = true;
            intercept(
                client, hello.raw, origin, &sni_host, port, selected, conn_info, hub,
            )
            .await
        }
        Ok(Err(err)) => {
            // The bytes still go through as a tunnel, so the site keeps
            // working and there is no decrypted request. The row is optional
            // only when the fallback tunnel actually carries traffic.
            let reason = origin_tls_reason(&err);
            log::debug!("源站 TLS 失败，改隧道 {sni_host}:{port}: {reason} ({err:#})");
            let is_client_cert = is_client_cert_error(&err);
            let recorded = hub.show_tunnel_failures.load(Ordering::Relaxed)
                || hub.has_client_cert_for_host(&sni_host)
                || is_client_cert;
            if recorded {
                record_tunnel(&hub, &sni_host, port, reason.clone());
            }
            let result = tunnel_after_hello(&mut client, hello.raw, host, port, &hub).await;
            if !recorded
                && !matches!(
                    result,
                    Ok((from_client, from_origin)) if from_client > 16 && from_origin > 0
                )
            {
                record_tunnel(&hub, &sni_host, port, reason);
            }
            result.map(|_| ())
        }
        Err(_) => {
            record_tunnel(&hub, &sni_host, port, "连接源站超时，已改为隧道".into());
            tunnel_after_hello(&mut client, hello.raw, host, port, &hub)
                .await
                .map(|_| ())
        }
    }
}

async fn connect_origin(
    host: &str,
    port: u16,
    alpn: &[Vec<u8>],
    hub: &Hub,
) -> Result<(
    tokio_rustls::client::TlsStream<TcpStream>,
    Vec<u8>,
    crate::session::ConnInfo,
)> {
    let (config, presented_cert) = hub.client_config_for_host(host, alpn)?;
    let (tcp, mut info) = timeout(IO_TIMEOUT, dial_timed(host, port, hub, true))
        .await
        .context("连接源站超时")??;
    tcp.set_nodelay(true).ok();
    if let Some(cert_path) = presented_cert {
        info.client_cert = cert_path;
    }
    let connector = TlsConnector::from(Arc::new(config));
    let name = server_name(host)?;
    let tls_start = Instant::now();
    let tls = connector.connect(name, tcp).await.context("源站 TLS")?;
    info.tls_ms = Some(tls_start.elapsed().as_millis() as u64);
    let conn = tls.get_ref().1;
    info.tls_version = match conn.protocol_version() {
        Some(rustls::ProtocolVersion::TLSv1_3) => "TLS 1.3".to_string(),
        Some(rustls::ProtocolVersion::TLSv1_2) => "TLS 1.2".to_string(),
        Some(other) => format!("{other:?}"),
        None => String::new(),
    };
    let selected = conn
        .alpn_protocol()
        .map(|proto| proto.to_vec())
        .unwrap_or_else(|| b"http/1.1".to_vec());
    Ok((tls, selected, info))
}

async fn fail_client_cert_connect<S>(
    client: S,
    client_hello: Vec<u8>,
    host: &str,
    port: u16,
    alpn: &[Vec<u8>],
    err: anyhow::Error,
    hub: Arc<Hub>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let reason = client_cert_load_reason(&err);
    let selected = if alpn.iter().any(|proto| proto == b"h2") {
        b"h2".to_vec()
    } else {
        b"http/1.1".to_vec()
    };
    let protocol = if selected == b"h2" {
        Protocol::Http2
    } else {
        Protocol::Http11
    };
    let config = match hub.server_config_for_host(host, &selected) {
        Ok(cfg) => cfg,
        Err(_) => {
            record_note(&hub, host, port, protocol, reason.clone());
            return Err(anyhow!(reason));
        }
    };
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let rewound = Prefixed {
        prefix: client_hello,
        pos: 0,
        inner: client,
    };
    let client_tls = match timeout(IO_TIMEOUT, acceptor.accept(rewound)).await {
        Ok(Ok(tls)) => tls,
        Ok(Err(_)) | Err(_) => {
            record_note(&hub, host, port, protocol, reason.clone());
            return Err(anyhow!(reason));
        }
    };
    let served = Arc::new(AtomicBool::new(false));
    let served_svc = served.clone();
    let host_owned = host.to_string();
    let reason_svc = reason.clone();
    let hub_svc = hub.clone();
    let client_ctx = current_client_ctx();
    let service = hyper::service::service_fn(move |req: Request<Incoming>| {
        let served = served_svc.clone();
        let hub = hub_svc.clone();
        let host = host_owned.clone();
        let reason = reason_svc.clone();
        let client_ctx = client_ctx.clone();
        async move {
            served.store(true, Ordering::Relaxed);
            let fut = async move {
                let encoding = req
                    .headers()
                    .get(http::header::CONTENT_ENCODING)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                let (parts, body) = req.into_parts();
                let path = parts
                    .uri
                    .path_and_query()
                    .map(|p| p.as_str().to_string())
                    .unwrap_or_else(|| "/".into());
                let authority = display_authority(&host, port, "https");
                let id = hub.open(FlowDraft {
                    protocol,
                    method: parts.method.to_string(),
                    host: host.clone(),
                    path: path.clone(),
                    url: format!("https://{authority}{path}"),
                    request_headers: header_pairs(&parts.headers),
                });
                let _ = record_request_bytes(&hub, id, body, encoding.as_deref()).await;
                hub.note_rules_with_ops(id, &host, &path, &[]);
                if hub.ca.has_client_cert_in_dir(&host) {
                    hub.with_session(|session| {
                        if let Some(flow) = session.get(id)
                            && !flow.rule_hit.split_whitespace().any(|t| t == "client-cert")
                        {
                            let next = if flow.rule_hit.is_empty() {
                                "client-cert".to_string()
                            } else {
                                format!("{} client-cert", flow.rule_hit)
                            };
                            session.set_rule_hit(id, next);
                        }
                    });
                }
                hub.set_error(id, reason.clone());
                let body_msg = format!("zniff: {reason}\n");
                synthetic_response(
                    &hub,
                    id,
                    StatusCode::BAD_GATEWAY,
                    "text/plain; charset=utf-8",
                    body_msg.into_bytes(),
                )
            };
            let res = if let Some(ctx) = client_ctx {
                CLIENT_CTX.scope(ctx, fut).await
            } else {
                fut.await
            };
            res.map_err(|e| io::Error::other(e.to_string()))
        }
    });
    let serve_res = if selected == b"h2" {
        hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(client_tls), service)
            .await
            .map_err(|e| anyhow!(e))
    } else {
        hyper::server::conn::http1::Builder::new()
            .preserve_header_case(true)
            .serve_connection(TokioIo::new(client_tls), service)
            .await
            .map_err(|e| anyhow!(e))
    };
    if !served.load(Ordering::Relaxed) {
        record_note(&hub, host, port, protocol, reason);
    }
    serve_res
}

#[allow(clippy::too_many_arguments)]
async fn intercept<S>(
    client: S,
    client_hello: Vec<u8>,
    origin: tokio_rustls::client::TlsStream<TcpStream>,
    host: &str,
    port: u16,
    selected: Vec<u8>,
    conn_info: crate::session::ConnInfo,
    hub: Arc<Hub>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let config = hub
        .server_config_for_host(host, &selected)
        .context("叶子证书")?;
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let rewound = Prefixed {
        prefix: client_hello,
        pos: 0,
        inner: client,
    };
    let client_tls = match timeout(IO_TIMEOUT, acceptor.accept(rewound)).await {
        Ok(Ok(tls)) => tls,
        Ok(Err(err)) => {
            let reason = client_tls_reason(&err, host);
            record_note(&hub, host, port, Protocol::Http11, reason.clone());
            return Err(anyhow!(reason));
        }
        Err(_) => {
            record_note(&hub, host, port, Protocol::Http11, "客户端 TLS 超时".into());
            return Err(anyhow!("客户端 TLS 超时"));
        }
    };
    let host = host.to_string();
    if selected == b"h2" {
        serve_h2(client_tls, origin, host, port, conn_info, hub).await
    } else {
        serve_h1(client_tls, origin, host, port, conn_info, hub).await
    }
}

fn client_tls_reason(err: &io::Error, host: &str) -> String {
    if let Some(alert) = client_tls_alert(err) {
        return match alert {
            rustls::AlertDescription::UnknownCA
            | rustls::AlertDescription::BadCertificate
            | rustls::AlertDescription::CertificateUnknown
            | rustls::AlertDescription::CertificateExpired
            | rustls::AlertDescription::CertificateRevoked
            | rustls::AlertDescription::UnsupportedCertificate => {
                "证书被拒绝。猜测：客户端不信任 zniff CA，请安装并信任根证书。".into()
            }
            rustls::AlertDescription::HandshakeFailure
            | rustls::AlertDescription::InsufficientSecurity
            | rustls::AlertDescription::AccessDenied => pinned_guess(),
            _ => reset_guess(host),
        };
    }
    match err.kind() {
        io::ErrorKind::UnexpectedEof
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::BrokenPipe => reset_guess(host),
        _ => {
            let text = err.to_string();
            let lower = text.to_ascii_lowercase();
            if lower.contains("unknown_ca") || lower.contains("bad_certificate") {
                "证书被拒绝。猜测：客户端不信任 zniff CA，请安装并信任根证书。".into()
            } else if lower.contains("certificate")
                || lower.contains("alert")
                || lower.contains("close_notify")
            {
                reset_guess(host)
            } else {
                format!("客户端 TLS 失败: {text}")
            }
        }
    }
}

fn client_tls_alert(err: &io::Error) -> Option<rustls::AlertDescription> {
    let tls = err.get_ref()?.downcast_ref::<rustls::Error>()?;
    match tls {
        rustls::Error::AlertReceived(alert) => Some(*alert),
        _ => None,
    }
}

/// Connection closed with no TLS alert. A pinned host is the usual case;
/// otherwise the CA is the first thing to check.
fn reset_guess(host: &str) -> String {
    if likely_pinned(host) {
        pinned_guess()
    } else {
        "证书被拒绝。猜测：先确认系统已信任 zniff CA。若已信任，就是这个客户端自带证书库或做了钉扎，把主机加入直通名单。".into()
    }
}

fn pinned_guess() -> String {
    "证书被拒绝。猜测：该客户端多半做了证书钉扎，不接受代理签发的证书。把主机加入直通名单。若还没安装 zniff CA，先安装。".into()
}

fn likely_pinned(host: &str) -> bool {
    let host = host.trim().trim_matches(|ch| ch == '[' || ch == ']');
    let host = host.split('%').next().unwrap_or(host);
    const SUFFIXES: &[&str] = &[
        "google.com",
        "googleapis.com",
        "gstatic.com",
        "googleusercontent.com",
        "gvt1.com",
        "gvt2.com",
        "youtube.com",
        "ytimg.com",
        "ggpht.com",
        "apple.com",
        "icloud.com",
        "mzstatic.com",
        "cdn-apple.com",
        "microsoft.com",
        "live.com",
        "office.com",
        "office365.com",
        "windows.com",
        "windows.net",
        "facebook.com",
        "fbcdn.net",
        "instagram.com",
        "whatsapp.net",
        "whatsapp.com",
    ];
    SUFFIXES
        .iter()
        .any(|suffix| host.eq_ignore_ascii_case(suffix) || host_ends_with(host, suffix))
}

fn host_ends_with(host: &str, suffix: &str) -> bool {
    host.len() > suffix.len()
        && host.as_bytes().get(host.len() - suffix.len() - 1) == Some(&b'.')
        && host[host.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

type H1Sender = hyper::client::conn::http1::SendRequest<RecordingBody<Incoming>>;

async fn handshake_h1<O>(origin: O) -> Result<H1Sender>
where
    O: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (sender, connection) = hyper::client::conn::http1::Builder::new()
        .preserve_header_case(true)
        .handshake::<_, RecordingBody<Incoming>>(TokioIo::new(origin))
        .await
        .context("上游 HTTP/1.1")?;
    tokio::spawn(async move {
        if let Err(err) = connection.with_upgrades().await {
            log::debug!("上游 HTTP/1.1 连接结束: {err}");
        }
    });
    Ok(sender)
}

async fn dial_h1(host: &str, port: u16, hub: &Hub) -> Result<(H1Sender, crate::session::ConnInfo)> {
    let (origin, _, conn_info) = timeout(
        IO_TIMEOUT,
        connect_origin(host, port, &[b"http/1.1".to_vec()], hub),
    )
    .await
    .context("连接源站超时")??;
    let sender = handshake_h1(origin).await?;
    Ok((sender, conn_info))
}

async fn serve_h1<C, O>(
    client: C,
    origin: O,
    host: String,
    port: u16,
    conn_info: crate::session::ConnInfo,
    hub: Arc<Hub>,
) -> Result<()>
where
    C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    O: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let sender = handshake_h1(origin).await?;
    // hyper 1.12's HTTP/1 SendRequest is not Clone. One task owns it and
    // redials the origin if an idle keep-alive connection was closed upstream.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<H1Job>();
    let host_for_worker = host.clone();
    let hub_for_worker = hub.clone();
    let initial_epoch = hub.origin_epoch.load(Ordering::Acquire);
    tokio::spawn(async move {
        let mut sender = sender;
        let mut conn_info = conn_info;
        let mut first = true;
        let mut broken = false;
        let mut lane_epoch = initial_epoch;
        while let Some(job) = rx.recv().await {
            let result = async {
                let mut pending = Some(job.req);
                for attempt in 0..2 {
                    let Some(req) = pending.take() else {
                        break;
                    };
                    let epoch_now = hub_for_worker.origin_epoch.load(Ordering::Acquire);
                    if broken
                        || lane_epoch != epoch_now
                        || sender.is_closed()
                        || sender.ready().await.is_err()
                    {
                        let (next_sender, next_info) =
                            dial_h1(&host_for_worker, port, &hub_for_worker).await?;
                        sender = next_sender;
                        conn_info = next_info;
                        first = true;
                        broken = false;
                        lane_epoch = hub_for_worker.origin_epoch.load(Ordering::Acquire);
                        sender.ready().await.context("上游 HTTP/1.1 未就绪")?;
                    }
                    let info = if first {
                        first = false;
                        conn_info.clone()
                    } else {
                        conn_info.as_reused()
                    };
                    match sender.try_send_request(req).await {
                        Ok(resp) => return Ok((resp, info)),
                        Err(mut err) => {
                            broken = true;
                            pending = err.take_message();
                            if attempt == 0 && pending.is_some() {
                                continue;
                            }
                            return Err(
                                anyhow::Error::from(err.into_error()).context("上游 HTTP/1.1")
                            );
                        }
                    }
                }
                Err(anyhow!("上游 HTTP/1.1 未就绪"))
            }
            .await;
            let _ = job.reply.send(result);
        }
    });
    let shared = H1Client { tx };
    let host_for_service = host.clone();
    let client_ctx = current_client_ctx();
    let service = hyper::service::service_fn(move |req| {
        let mut sender = shared.clone();
        let hub = hub.clone();
        let host = host_for_service.clone();
        let client_ctx = client_ctx.clone();
        async move {
            let fut = forward_request(
                &mut sender,
                req,
                hub,
                &host,
                port,
                "https",
                Protocol::Http11,
            );
            let res = if let Some(ctx) = client_ctx {
                CLIENT_CTX.scope(ctx, fut).await
            } else {
                fut.await
            };
            res.map_err(|err| io::Error::other(err.to_string()))
        }
    });
    hyper::server::conn::http1::Builder::new()
        .preserve_header_case(true)
        .serve_connection(TokioIo::new(client), service)
        .with_upgrades()
        .await
        .context("客户端 HTTP/1.1")?;
    Ok(())
}

async fn handshake_h2<O>(
    origin: O,
    host: &str,
    port: u16,
    conn_info: crate::session::ConnInfo,
    hub: &Arc<Hub>,
) -> Result<(H2Sender, u64, crate::session::ConnInfo, Arc<AtomicBool>)>
where
    O: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (sender, connection) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
        .handshake::<_, RecordingBody<Incoming>>(TokioIo::new(origin))
        .await
        .context("上游 HTTP/2")?;
    let (generation, first_used) =
        hub.store_h2((host.to_string(), port), sender.clone(), conn_info.clone());
    let hub = hub.clone();
    let key = (host.to_string(), port);
    tokio::spawn(async move {
        if let Err(err) = connection.await {
            log::debug!("上游 HTTP/2 连接结束: {err}");
        }
        hub.drop_h2_generation(&key, generation);
    });
    Ok((sender, generation, conn_info, first_used))
}

/// Dials the origin offering only `h2`. A server that picks another protocol
/// is an error: this path does not fall back to HTTP/1.1.
async fn dial_h2(
    host: &str,
    port: u16,
    hub: &Arc<Hub>,
) -> Result<(H2Sender, u64, crate::session::ConnInfo, Arc<AtomicBool>)> {
    let (origin, selected, conn_info) = timeout(
        IO_TIMEOUT,
        connect_origin(host, port, &[b"h2".to_vec()], hub),
    )
    .await
    .context("连接源站超时")??;
    if selected != b"h2" {
        anyhow::bail!("源站没有选择 HTTP/2");
    }
    handshake_h2(origin, host, port, conn_info, hub).await
}

async fn serve_h2<C, O>(
    client: C,
    origin: O,
    host: String,
    port: u16,
    conn_info: crate::session::ConnInfo,
    hub: Arc<Hub>,
) -> Result<()>
where
    C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    O: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (sender, generation, conn_info, first_used) =
        handshake_h2(origin, &host, port, conn_info, &hub).await?;
    serve_h2_sender(
        client,
        sender,
        generation,
        conn_info,
        first_used,
        host,
        port,
        hub,
        TokioExecutor::new(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn intercept_h2_pooled<S>(
    client: S,
    client_hello: Vec<u8>,
    sender: H2Sender,
    generation: u64,
    conn_info: crate::session::ConnInfo,
    first_used: Arc<AtomicBool>,
    host: &str,
    port: u16,
    hub: Arc<Hub>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let config = hub
        .server_config_for_host(host, b"h2")
        .context("叶子证书")?;
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let rewound = Prefixed {
        prefix: client_hello,
        pos: 0,
        inner: client,
    };
    let client_tls = match timeout(IO_TIMEOUT, acceptor.accept(rewound)).await {
        Ok(Ok(tls)) => tls,
        Ok(Err(err)) => {
            let reason = client_tls_reason(&err, host);
            record_note(&hub, host, port, Protocol::Http2, reason.clone());
            return Err(anyhow!(reason));
        }
        Err(_) => {
            record_note(&hub, host, port, Protocol::Http2, "客户端 TLS 超时".into());
            return Err(anyhow!("客户端 TLS 超时"));
        }
    };
    serve_h2_sender(
        client_tls,
        sender,
        generation,
        conn_info,
        first_used,
        host.to_string(),
        port,
        hub,
        TokioExecutor::new(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn serve_h2_sender<C>(
    client: C,
    sender: H2Sender,
    generation: u64,
    conn_info: crate::session::ConnInfo,
    first_used: Arc<AtomicBool>,
    host: String,
    port: u16,
    hub: Arc<Hub>,
    executor: TokioExecutor,
) -> Result<()>
where
    C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let shared = H2Lane {
        lane: Arc::new(tokio::sync::Mutex::new(LaneInner {
            sender,
            last_ok: Instant::now(),
            broken: false,
            generation,
            origin_epoch: hub.origin_epoch.load(Ordering::Acquire),
            conn_info,
            first_used,
        })),
        hub: hub.clone(),
        host: host.clone(),
        port,
    };
    let client_ctx = current_client_ctx();
    let service = hyper::service::service_fn(move |req| {
        let mut lane = shared.clone();
        let hub = lane.hub.clone();
        let host = lane.host.clone();
        let client_ctx = client_ctx.clone();
        async move {
            let fut = forward_request(&mut lane, req, hub, &host, port, "https", Protocol::Http2);
            let res = if let Some(ctx) = client_ctx {
                CLIENT_CTX.scope(ctx, fut).await
            } else {
                fut.await
            };
            res.map_err(|err| io::Error::other(err.to_string()))
        }
    });
    hyper::server::conn::http2::Builder::new(executor)
        .serve_connection(TokioIo::new(client), service)
        .await
        .context("客户端 HTTP/2")?;
    Ok(())
}

fn server_name(host: &str) -> Result<ServerName<'static>> {
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        Ok(ServerName::IpAddress(ip.into()))
    } else {
        ServerName::try_from(host.to_string()).context("SNI")
    }
}

/// Helper to distinguish local client certificate configuration / loading errors
/// from upstream TLS handshake rejections.
fn is_client_cert_error(err: &anyhow::Error) -> bool {
    err.chain().any(|c| {
        let s = c.to_string();
        s.contains("客户端证书")
            || s.contains("mTLS")
            || s.contains("client certificate")
            || s.contains("client-cert")
    })
}

fn client_cert_load_reason(err: &anyhow::Error) -> String {
    format!("客户端证书加载失败: {err:#}")
}

/// Short reason for a probe that never reached ServerHello. `{err}` alone is
/// only the `context("源站 TLS")` label, so the list would repeat that label.
fn origin_tls_reason(err: &anyhow::Error) -> String {
    if is_client_cert_error(err) {
        return client_cert_load_reason(err);
    }
    if let Some(tls) = rustls_error(err) {
        return match tls {
            rustls::Error::InvalidCertificate(cert) => match cert {
                rustls::CertificateError::UnknownIssuer => "源站证书不在内置根证书里，已改为隧道",
                rustls::CertificateError::Expired
                | rustls::CertificateError::ExpiredContext { .. } => "源站证书已过期，已改为隧道",
                rustls::CertificateError::NotValidYet
                | rustls::CertificateError::NotValidYetContext { .. } => {
                    "源站证书还没到生效时间，已改为隧道"
                }
                rustls::CertificateError::NotValidForName
                | rustls::CertificateError::NotValidForNameContext { .. } => {
                    "源站证书域名对不上，已改为隧道"
                }
                rustls::CertificateError::Revoked => "源站证书已吊销，已改为隧道",
                _ => "源站证书无效，已改为隧道",
            }
            .to_string(),
            rustls::Error::AlertReceived(alert) => {
                format!("源站中止握手（{alert:?}），已改为隧道")
            }
            rustls::Error::NoApplicationProtocol => {
                "源站不接受客户端的协议，已改为隧道".to_string()
            }
            rustls::Error::NoCertificatesPresented => "源站没有给出证书，已改为隧道".to_string(),
            rustls::Error::PeerIncompatible(_) => {
                "源站 TLS 版本或算法不兼容，已改为隧道".to_string()
            }
            other => format!("源站 TLS 失败，已改为隧道: {other}"),
        };
    }
    for cause in err.chain().rev() {
        let text = cause.to_string();
        if text != "源站 TLS" && !text.is_empty() {
            return format!("源站 TLS 失败，已改为隧道: {text}");
        }
    }
    "源站 TLS 失败，已改为隧道".to_string()
}

/// The pink error box only showed the step name (`HTTP/2 上游`), because
/// `anyhow` prints the outer context. Append one inferred cause.
fn explain_flow_error(err: &anyhow::Error) -> String {
    let label = err.to_string();
    if label.is_empty() || label.contains('。') || label.contains("猜测") {
        return label;
    }
    match flow_error_guess(err, &label) {
        Some(guess) => format!("{label}。猜测：{guess}"),
        None => label,
    }
}

fn flow_error_guess(err: &anyhow::Error, label: &str) -> Option<&'static str> {
    let text = err
        .chain()
        .map(|cause| cause.to_string().to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    let hyper = err
        .chain()
        .find_map(|cause| cause.downcast_ref::<hyper::Error>());
    let io = err
        .chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>());
    if is_flow_timeout(label, &text, hyper, io) {
        return Some(timeout_guess(label));
    }
    if hyper.is_some_and(|err| err.is_closed())
        || text.contains("channel closed")
        || text.contains("dispatch task is gone")
    {
        return Some("到源站的发送端已经结束，多半是上一条把连接关掉了。");
    }
    if text.contains("requires http/1.1") || text.contains("http_1_1_required") {
        return Some("源站要求改用 HTTP/1.1，不接受这条 HTTP/2。");
    }
    if text.contains("refused stream")
        || text.contains("connection error received")
        || text.contains("connection error detected")
        || text.contains("goaway")
    {
        return Some("源站拒绝继续在这条 HTTP/2 连接上接收请求。");
    }
    if is_flow_reset(&text, io) {
        return Some("源站或网络把连接重置了。");
    }
    if io.is_some_and(|err| err.kind() == std::io::ErrorKind::ConnectionRefused)
        || text.contains("connection refused")
    {
        return Some("源站端口没有服务在听。");
    }
    if hyper.is_some_and(|err| err.is_incomplete_message())
        || text.contains("connection closed before message completed")
    {
        return Some("对端在消息没收完时把连接关了。");
    }
    if text.contains("nodename nor servname")
        || text.contains("name or service not known")
        || text.contains("failed to lookup")
        || text.contains("no such host")
        || text.contains("temporary failure in name resolution")
    {
        return Some("主机名没有解析到地址。");
    }
    if is_flow_canceled(label, &text, hyper) {
        return Some(canceled_guess(label));
    }
    bare_step_guess(label)
}

fn canceled_guess(label: &str) -> &'static str {
    if label.contains("HTTP/2") {
        "复用的 HTTP/2 连接已经关掉，请求还没送到源站。下一次会重新连接。"
    } else if label.contains("HTTP/1.1") {
        "到源站的 HTTP/1.1 连接已经结束，这条请求没送出去。"
    } else {
        "连接已经关掉，请求没送出去。"
    }
}

fn is_flow_timeout(
    label: &str,
    text: &str,
    hyper: Option<&hyper::Error>,
    io: Option<&std::io::Error>,
) -> bool {
    label.contains("超时")
        || hyper.is_some_and(|err| err.is_timeout())
        || io.is_some_and(|err| err.kind() == std::io::ErrorKind::TimedOut)
        || text.contains("timed out")
        || text.contains("deadline has elapsed")
}

fn timeout_guess(label: &str) -> &'static str {
    if label.contains("请求头") {
        "客户端迟迟没有把请求头发完。"
    } else if label.contains("源站") || label.contains("隧道") {
        "源站没有在时限内接受连接。可能是网络不通、地址解析慢，或源站没有响应。"
    } else {
        "这一步在时限内没有完成。"
    }
}

fn is_flow_canceled(label: &str, text: &str, hyper: Option<&hyper::Error>) -> bool {
    hyper.is_some_and(|err| err.is_canceled())
        || text.contains("connection was not ready")
        || text.contains("operation was canceled")
        || label == "上游 HTTP/1.1 未就绪"
        || label == "上游 HTTP/1.1 已经关闭"
}

fn is_flow_reset(text: &str, io: Option<&std::io::Error>) -> bool {
    io.is_some_and(|err| {
        matches!(
            err.kind(),
            std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::BrokenPipe
        )
    }) || text.contains("connection reset")
        || text.contains("broken pipe")
        || text.contains("connection aborted")
}

fn bare_step_guess(label: &str) -> Option<&'static str> {
    Some(match label {
        "HTTP/2 上游" => {
            "发往源站的 HTTP/2 请求失败，还没有状态码。常见是这条连接已经失效，或源站把流关掉了。"
        }
        "上游 HTTP/2" => "和源站建立 HTTP/2 失败。源站可能不接受这个协议，或刚连上就把连接关了。",
        "上游 HTTP/1.1" => "和源站的 HTTP/1.1 对话中断。",
        "上游 HTTP/1.1 没有返回" => "请求送出后，源站没有把响应交回来。",
        "客户端 HTTP/2" | "客户端 HTTP/1.1" => {
            "和客户端的连接中断。源站那边可能已经开始处理。"
        }
        "连接上游代理" => "配置里的上游代理没有接通。",
        "组装上游 URI" | "组装断点响应" | "组装映射请求" => {
            "改写后的地址或响应不是合法的 HTTP。"
        }
        "映射连接" | "映射连接未就绪" | "映射请求" => "按转发规则去连目标时失败。",
        "SNI" => "主机名不能当作 TLS 的服务器名。",
        _ => return None,
    })
}

fn rustls_error(err: &anyhow::Error) -> Option<&rustls::Error> {
    for cause in err.chain() {
        if let Some(tls) = cause.downcast_ref::<rustls::Error>() {
            return Some(tls);
        }
        let Some(io_err) = cause.downcast_ref::<io::Error>() else {
            continue;
        };
        if let Some(tls) = io_err
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        {
            return Some(tls);
        }
    }
    None
}

fn record_tunnel(hub: &Hub, host: &str, port: u16, reason: String) {
    let id = hub.open(FlowDraft {
        protocol: Protocol::Tunnel,
        method: "CONNECT".into(),
        host: host.to_string(),
        path: format!(":{port}"),
        url: format!("{host}:{port}"),
        request_headers: Vec::new(),
    });
    hub.set_error(id, reason);
}

fn record_note(hub: &Hub, host: &str, port: u16, protocol: Protocol, reason: String) {
    let id = hub.open(FlowDraft {
        protocol,
        method: "CONNECT".into(),
        host: host.to_string(),
        path: "/".into(),
        url: format!("https://{}:{port}", display_host(host)),
        request_headers: Vec::new(),
    });
    hub.set_error(id, reason);
}

fn display_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

fn display_authority(host: &str, port: u16, scheme: &str) -> String {
    let host = display_host(host);
    let default_port = if scheme == "https" { 443 } else { 80 };
    if port == default_port {
        host
    } else {
        format!("{host}:{port}")
    }
}

async fn tunnel_after_hello<S>(
    client: &mut S,
    hello: Vec<u8>,
    host: &str,
    port: u16,
    hub: &Hub,
) -> Result<(u64, u64)>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut origin = timeout(IO_TIMEOUT, dial(host, port, hub, true))
        .await
        .context("隧道连接源站超时")??;
    origin.set_nodelay(true).ok();
    origin.write_all(&hello).await?;
    copy_bidirectional(client, &mut origin).await
}

async fn plain_http<S>(client: &mut S, head: &Head, hub: Arc<Hub>) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut client = Prefixed {
        prefix: Vec::new(),
        pos: 0,
        inner: client,
    };
    let (method, host, port, path) = request_target(head)?;
    let sanitized_headers: Vec<(String, String)> = head
        .headers
        .iter()
        .filter(|(name, _)| !name.eq_ignore_ascii_case("proxy-authorization"))
        .cloned()
        .collect();
    let id = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: method.clone(),
        host: host.clone(),
        path: path.clone(),
        url: format!("http://{}{path}", display_authority(&host, port, "http")),
        request_headers: sanitized_headers.clone(),
    });
    let mut host = host;
    let mut port = port;
    let mut path = path;
    let mut headers = sanitized_headers;
    let mut body_override: Option<Vec<u8>> = None;
    let mut request_taken = false;
    let mut request_recorded = false;
    let mut request_overflowed = false;
    let mut break_overrode_body = false;
    let mut strip_request_encoding = false;
    let has_expect = header_has_expect_100_continue(&head.headers);
    let websocket = header_is_websocket(&headers);
    let plan = if websocket {
        rules::Effect::default()
    } else {
        let mut ops = crate::present::graphql_operations(&path, &[]);
        if !has_expect && hub.has_op_rules(&host, &path) {
            if let Some(len) = content_length(&head.headers) {
                if len <= crate::session::BODY_CAP && len > 0 {
                    let raw = read_exact(&mut client, len).await?;
                    request_taken = true;
                    let req_encoding = header_value(&head.headers, "content-encoding");
                    note_decoded(&hub, id, Direction::Request, &raw, req_encoding.as_deref());
                    request_recorded = true;
                    let decoded = hub.with_session(|s| {
                        s.get(id)
                            .map(|f| f.request_body.clone())
                            .unwrap_or_default()
                    });
                    ops = crate::present::graphql_operations(&path, &decoded);
                    body_override = Some(raw);
                } else if len > crate::session::BODY_CAP {
                    request_overflowed = true;
                }
            } else if is_chunked(&head.headers) {
                match read_chunked_limited(&mut client, crate::session::BODY_CAP).await? {
                    LimitedRead::Whole(raw) => {
                        request_taken = true;
                        let req_encoding = header_value(&head.headers, "content-encoding");
                        note_decoded(&hub, id, Direction::Request, &raw, req_encoding.as_deref());
                        request_recorded = true;
                        let decoded = hub.with_session(|s| {
                            s.get(id)
                                .map(|f| f.request_body.clone())
                                .unwrap_or_default()
                        });
                        ops = crate::present::graphql_operations(&path, &decoded);
                        body_override = Some(raw);
                    }
                    LimitedRead::Overflow { wire_prefix } => {
                        client.rewind(wire_prefix);
                        request_overflowed = true;
                    }
                }
            }
        }
        hub.note_rules_with_ops(id, &host, &path, &ops);
        hub.plan_with_ops(&host, &path, &ops)
    };
    if !plan.delay.is_zero() {
        tokio::time::sleep(plan.delay).await;
    }
    if !plan.request_headers.is_empty() {
        rules::apply_header_ops(&mut headers, &plan.request_headers);
        let existing = hub.with_session(|s| {
            s.get(id)
                .map(|f| f.request_body.clone())
                .unwrap_or_default()
        });
        hub.replace_request(id, headers.clone(), existing);
    }
    let response_ops = plan.response_headers.clone();
    let cors = plan.cors;
    let throttle_up_bps = plan.throttle_up_bps;
    let throttle_down_bps = plan.throttle_down_bps;
    let break_response = matches!(plan.action, Some(Action::BreakResponse));
    if cors && method.eq_ignore_ascii_case("OPTIONS") && plan.action.is_none() {
        if !request_taken {
            let _ = drain_request(&mut client, &headers, &hub, id).await;
        }
        let mut resp_headers = Vec::new();
        rules::ensure_cors(&mut resp_headers);
        rules::apply_header_ops(&mut resp_headers, &response_ops);
        write_static(&mut client, &hub, id, 204, "No Content", &resp_headers, &[]).await?;
        return Ok(());
    }
    let mut scheme = "http";
    match &plan.action {
        Some(Action::MapLocal {
            path_prefix,
            file,
            status,
            fallback,
        }) => match rules::resolve_map_local_target(file.as_deref(), path_prefix, &path) {
            Ok(resolved) => {
                if !request_taken {
                    let _ = drain_request(&mut client, &headers, &hub, id).await;
                }
                return write_mapped_file(
                    &mut client,
                    &hub,
                    id,
                    resolved.as_deref(),
                    *status,
                    &response_ops,
                    cors,
                    throttle_down_bps,
                )
                .await;
            }
            Err(rules::MapLocalError::NotFound(_)) if *fallback => {
                hub.append_comment(id, "map-local(fallback)");
            }
            Err(err) => {
                if !request_taken {
                    let _ = drain_request(&mut client, &headers, &hub, id).await;
                }
                let body = err.message();
                hub.set_error(id, body.clone());
                write_static(
                    &mut client,
                    &hub,
                    id,
                    502,
                    "Bad Gateway",
                    &[("Content-Type".into(), "text/plain".into())],
                    body.as_bytes(),
                )
                .await?;
                return Ok(());
            }
        },
        Some(Action::BreakRequest) => {
            let decision = hub.wait_break(id, BreakSide::Request).await;
            if decision.drop {
                if !request_taken && !has_expect {
                    let _ = drain_request(&mut client, &headers, &hub, id).await;
                }
                hub.set_error(id, "断点丢弃");
                hub.mark_finished(id);
                let _ = client.shutdown().await;
                return Ok(());
            }
            if let Some(next) = decision.headers {
                let existing = hub.with_session(|s| {
                    s.get(id)
                        .map(|f| f.request_body.clone())
                        .unwrap_or_default()
                });
                hub.replace_request(id, next.clone(), existing);
                headers = next;
            }
            if let Some(b) = decision.body {
                body_override = Some(b);
                break_overrode_body = true;
                strip_request_encoding = true;
            }
        }
        Some(Action::MapRemote { url, path_prefix }) => {
            if let Some((next_scheme, next_host, next_port, next_path)) = parse_http_url(url) {
                scheme = if next_scheme == "https" {
                    "https"
                } else {
                    "http"
                };
                host = next_host;
                port = next_port;
                path = rules::map_remote_path(&next_path, path_prefix, &path);
                let authority = display_authority(&host, port, scheme);
                hub.retarget(
                    id,
                    host.clone(),
                    path.clone(),
                    format!("{scheme}://{authority}{path}"),
                );
                if let Some(pos) = headers
                    .iter()
                    .position(|(k, _)| k.eq_ignore_ascii_case("host"))
                {
                    headers[pos].1 = authority.clone();
                } else {
                    headers.push(("Host".to_string(), authority.clone()));
                }
                let existing = hub.with_session(|s| {
                    s.get(id)
                        .map(|f| f.request_body.clone())
                        .unwrap_or_default()
                });
                hub.replace_request(id, headers.clone(), existing);
            }
        }
        Some(Action::BreakResponse) | None => {}
        Some(Action::Block { drop_conn, status }) => {
            if !request_taken {
                let _ = drain_request(&mut client, &headers, &hub, id).await;
            }
            if *drop_conn {
                hub.set_error(id, "规则阻断 (连接已断开)");
                hub.mark_finished(id);
                let _ = client.shutdown().await;
                return Ok(());
            }
            let mut resp_headers = vec![("Content-Length".to_string(), "0".to_string())];
            if cors {
                rules::ensure_cors(&mut resp_headers);
            }
            rules::apply_header_ops(&mut resp_headers, &response_ops);
            write_static(
                &mut client,
                &hub,
                id,
                *status,
                reason_phrase(*status),
                &resp_headers,
                &[],
            )
            .await?;
            return Ok(());
        }
    }
    if let Some(next) = rules::apply_query_ops(&path, &plan.query_ops) {
        path = next;
        hub.retarget(
            id,
            host.clone(),
            path.clone(),
            format!(
                "{scheme}://{}{path}",
                display_authority(&host, port, scheme)
            ),
        );
    }
    if let Some(next) = rules::replace_target(&path, &plan.request_replaces) {
        path = next;
        hub.retarget(
            id,
            host.clone(),
            path.clone(),
            format!(
                "{scheme}://{}{path}",
                display_authority(&host, port, scheme)
            ),
        );
    }
    if !break_overrode_body
        && (!plan.request_replaces.is_empty() || !plan.request_json_ops.is_empty())
    {
        if has_expect && body_override.is_none() {
            if !plan.request_json_ops.is_empty() {
                hub.append_comment(
                    id,
                    "[json rule skipped: request body withheld by Expect: 100-continue]",
                );
            }
        } else if request_overflowed {
            hub.append_comment(id, "[rule skipped: body > 1MB]");
        } else if let Some(raw) = body_override.take() {
            if !raw.is_empty() {
                let encoding = header_value(&head.headers, "content-encoding");
                if let Some(next) = rewrite_body(
                    &hub,
                    id,
                    &raw,
                    encoding.as_deref(),
                    &plan.request_replaces,
                    &plan.request_json_ops,
                ) {
                    strip_request_encoding = true;
                    body_override = Some(next);
                } else {
                    body_override = Some(raw);
                }
            } else {
                body_override = Some(raw);
            }
        } else if let Some(len) = content_length(&head.headers) {
            if len > crate::session::BODY_CAP {
                hub.append_comment(id, "[rule skipped: body > 1MB]");
            } else if len > 0 {
                request_taken = true;
                let raw = read_exact(&mut client, len).await?;
                let encoding = header_value(&head.headers, "content-encoding");
                if let Some(next) = rewrite_body(
                    &hub,
                    id,
                    &raw,
                    encoding.as_deref(),
                    &plan.request_replaces,
                    &plan.request_json_ops,
                ) {
                    strip_request_encoding = true;
                    body_override = Some(next);
                } else {
                    note_decoded(&hub, id, Direction::Request, &raw, encoding.as_deref());
                    request_recorded = true;
                    body_override = Some(raw);
                }
            }
        } else if is_chunked(&head.headers) {
            match read_chunked_limited(&mut client, crate::session::BODY_CAP).await? {
                LimitedRead::Whole(raw) => {
                    request_taken = true;
                    let encoding = header_value(&head.headers, "content-encoding");
                    if !raw.is_empty()
                        && let Some(next) = rewrite_body(
                            &hub,
                            id,
                            &raw,
                            encoding.as_deref(),
                            &plan.request_replaces,
                            &plan.request_json_ops,
                        )
                    {
                        strip_request_encoding = true;
                        body_override = Some(next);
                    } else {
                        if !raw.is_empty() {
                            note_decoded(&hub, id, Direction::Request, &raw, encoding.as_deref());
                        }
                        request_recorded = true;
                        body_override = Some(raw);
                    }
                }
                LimitedRead::Overflow { wire_prefix } => {
                    client.rewind(wire_prefix);
                    hub.append_comment(id, "[rule skipped: body > 1MB]");
                }
            }
        }
    }
    if scheme == "https" {
        let body = if let Some(body) = body_override {
            headers.retain(|(name, _)| {
                !name.eq_ignore_ascii_case("transfer-encoding")
                    && !(strip_request_encoding && name.eq_ignore_ascii_case("content-encoding"))
                    && !name.eq_ignore_ascii_case("content-length")
            });
            headers.push(("Content-Length".into(), body.len().to_string()));
            if strip_request_encoding || break_overrode_body || !request_recorded {
                hub.replace_request(id, headers.clone(), body.clone());
            }
            if !request_taken {
                let _ = discard_request(&mut client, &head.headers).await;
            }
            body
        } else if !request_taken {
            read_client_request_body(&mut client, &head.headers, has_expect, &hub, id).await?
        } else {
            Vec::new()
        };
        return exchange_tls(
            &mut client,
            &hub,
            id,
            &method,
            &host,
            port,
            &path,
            &headers,
            &body,
            &plan,
            break_response,
            throttle_up_bps,
            throttle_down_bps,
        )
        .await;
    }
    let pool_key = pool_key(&host, port, &hub);
    let (mut origin, mut conn_info) = match hub.take_idle(pool_key.clone()).await {
        Some(stream) => {
            let server_addr = stream
                .peer_addr()
                .map(|a| a.to_string())
                .unwrap_or_default();
            (
                stream,
                crate::session::ConnInfo {
                    reused: true,
                    server_addr,
                    ..Default::default()
                },
            )
        }
        None => timeout(IO_TIMEOUT, dial_timed(&host, port, &hub, false))
            .await
            .context("连接源站超时")
            .inspect_err(|err| {
                hub.set_error(id, explain_flow_error(err));
            })??,
    };
    origin.set_nodelay(true).ok();
    if let Some(body) = &body_override {
        headers.retain(|(name, _)| {
            !name.eq_ignore_ascii_case("transfer-encoding")
                && !(strip_request_encoding && name.eq_ignore_ascii_case("content-encoding"))
                && !name.eq_ignore_ascii_case("content-length")
        });
        headers.push(("Content-Length".into(), body.len().to_string()));
    }
    let upstream = hub.upstream();
    let target = if upstream.is_some() {
        format!("http://{}{path}", display_authority(&host, port, "http"))
    } else {
        path.clone()
    };
    let mut outbound = format!("{method} {target} HTTP/1.1\r\n");
    let mut has_host = false;
    let mut has_connection = false;
    let mut connection_close = false;
    for (name, value) in &headers {
        if name.eq_ignore_ascii_case("proxy-connection")
            || name.eq_ignore_ascii_case("proxy-authorization")
            || name.eq_ignore_ascii_case("proxy-authenticate")
            || (body_override.is_some()
                && (name.eq_ignore_ascii_case("content-length")
                    || name.eq_ignore_ascii_case("transfer-encoding")))
            || (strip_request_encoding && name.eq_ignore_ascii_case("content-encoding"))
            || (websocket && name.eq_ignore_ascii_case("sec-websocket-extensions"))
        {
            continue;
        }
        if name.eq_ignore_ascii_case("host") {
            has_host = true;
        }
        if name.eq_ignore_ascii_case("connection") {
            has_connection = true;
            connection_close = value.eq_ignore_ascii_case("close");
        }
        outbound.push_str(name);
        outbound.push_str(": ");
        outbound.push_str(value);
        outbound.push_str("\r\n");
    }
    if !has_host {
        outbound.push_str(&format!(
            "Host: {}\r\n",
            display_authority(&host, port, "http")
        ));
    }
    if let Some(auth) = upstream.as_ref().and_then(|u| u.auth_header.as_deref()) {
        outbound.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    if !has_connection {
        outbound.push_str("Connection: keep-alive\r\n");
    }
    if let Some(body) = &body_override {
        outbound.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    outbound.push_str("\r\n");
    if origin.write_all(outbound.as_bytes()).await.is_err() {
        let (next_origin, next_info) = timeout(IO_TIMEOUT, dial_timed(&host, port, &hub, false))
            .await
            .context("连接源站超时")??;
        origin = next_origin;
        conn_info = next_info;
        origin.set_nodelay(true).ok();
        origin.write_all(outbound.as_bytes()).await?;
    }
    hub.set_conn_info(id, conn_info);
    let encoding = header_value(&headers, "content-encoding");
    let mut request_body_copied = false;
    if let Some(body) = &body_override {
        write_all_throttled(&mut origin, body, throttle_up_bps).await?;
        if strip_request_encoding || break_overrode_body || !request_recorded {
            hub.replace_request(id, headers.clone(), body.clone());
        }
        if !request_taken {
            let _ = discard_request(&mut client, &head.headers).await;
        }
    } else if !has_expect {
        copy_message_body(
            &mut client,
            &mut origin,
            &head.headers,
            &hub,
            id,
            Direction::Request,
            encoding.clone(),
            throttle_up_bps,
            false,
        )
        .await?;
        request_body_copied = true;
    }
    let (mut resp_bytes, mut resp_rest) = read_header_block(&mut origin).await?;
    let mut resp = parse_head(&resp_bytes)?;
    let (mut status, mut reason) = status_and_reason(&resp.start);
    let mut interim_count = 0;
    while (100..=199).contains(&status) && status != 101 {
        interim_count += 1;
        if interim_count > 16 {
            anyhow::bail!("过多 1xx 中间响应");
        }
        client.write_all(&resp_bytes).await?;
        if has_expect && status == 100 && !request_body_copied && body_override.is_none() {
            copy_message_body(
                &mut client,
                &mut origin,
                &head.headers,
                &hub,
                id,
                Direction::Request,
                encoding.clone(),
                throttle_up_bps,
                false,
            )
            .await?;
            request_body_copied = true;
        }
        let (next_bytes, next_rest) = read_header_block_prefixed(&mut origin, resp_rest).await?;
        resp_bytes = next_bytes;
        resp_rest = next_rest;
        resp = parse_head(&resp_bytes)?;
        let (s, r) = status_and_reason(&resp.start);
        status = s;
        reason = r;
    }
    if websocket {
        resp_bytes = strip_ws_extension(&resp_bytes);
        resp = parse_head(&resp_bytes)?;
        let (s, r) = status_and_reason(&resp.start);
        status = s;
        reason = r;
    }
    let mut resp_headers = resp.headers.clone();
    if !websocket {
        rules::apply_header_ops(&mut resp_headers, &response_ops);
        if cors {
            rules::ensure_cors(&mut resp_headers);
        }
    }
    if websocket {
        client.write_all(&resp_bytes).await?;
    }
    hub.set_response(id, status, resp_headers.clone(), None);
    if status == 101 && header_is_websocket(&headers) {
        hub.mark_protocol(id, Protocol::WebSocket);
        if !resp_rest.is_empty() {
            client.write_all(&resp_rest).await?;
        }
        let copied = copy_with_messages(&mut client, &mut origin, &hub, id, &resp_rest).await;
        hub.mark_finished(id);
        copied?;
        return Ok(());
    }
    let no_resp_body = response_has_no_body(&method, status);
    let mut origin = Prefixed {
        prefix: resp_rest,
        pos: 0,
        inner: origin,
    };
    if !websocket
        && !break_response
        && !no_resp_body
        && (!plan.response_replaces.is_empty() || !plan.response_json_ops.is_empty())
    {
        let buffered_resp = if let Some(len) = content_length(&resp_headers) {
            if len > crate::session::BODY_CAP {
                hub.append_comment(id, "[rule skipped: body > 1MB]");
                None
            } else {
                Some(read_exact(&mut origin, len).await?)
            }
        } else if is_chunked(&resp_headers) {
            match read_chunked_limited(&mut origin, crate::session::BODY_CAP).await? {
                LimitedRead::Whole(raw) => Some(raw),
                LimitedRead::Overflow { wire_prefix } => {
                    origin.rewind(wire_prefix);
                    hub.append_comment(id, "[rule skipped: body > 1MB]");
                    None
                }
            }
        } else {
            match read_eof_limited(&mut origin, crate::session::BODY_CAP).await? {
                LimitedRead::Whole(raw) => Some(raw),
                LimitedRead::Overflow { wire_prefix } => {
                    origin.rewind(wire_prefix);
                    hub.append_comment(id, "[rule skipped: body > 1MB]");
                    None
                }
            }
        };
        if let Some(raw) = buffered_resp {
            let encoding = header_value(&resp_headers, "content-encoding");
            if !raw.is_empty()
                && let Some(next) = rewrite_body(
                    &hub,
                    id,
                    &raw,
                    encoding.as_deref(),
                    &plan.response_replaces,
                    &plan.response_json_ops,
                )
            {
                resp_headers.retain(|(name, _)| {
                    !name.eq_ignore_ascii_case("content-encoding")
                        && !name.eq_ignore_ascii_case("transfer-encoding")
                        && !name.eq_ignore_ascii_case("content-length")
                });
                resp_headers.push(("Content-Length".into(), next.len().to_string()));
                hub.replace_response(id, status, resp_headers.clone(), next.clone());
                write_bytes(
                    &mut client,
                    status,
                    &reason,
                    &resp_headers,
                    &next,
                    throttle_down_bps,
                )
                .await?;
                hub.mark_finished(id);
                return Ok(());
            }
            resp_headers.retain(|(name, _)| {
                !name.eq_ignore_ascii_case("transfer-encoding")
                    && !name.eq_ignore_ascii_case("content-length")
            });
            resp_headers.push(("Content-Length".into(), raw.len().to_string()));
            hub.replace_response_head(id, status, resp_headers.clone());
            write_bytes(
                &mut client,
                status,
                &reason,
                &resp_headers,
                &raw,
                throttle_down_bps,
            )
            .await?;
            note_decoded(&hub, id, Direction::Response, &raw, encoding.as_deref());
            hub.mark_finished(id);
            return Ok(());
        }
    }
    if break_response {
        match read_capped_body(
            &mut origin,
            &resp_headers,
            crate::session::BODY_CAP,
            no_resp_body,
        )
        .await
        {
            Ok(Some(body)) => {
                let encoding = header_value(&resp_headers, "content-encoding");
                if !body.is_empty() {
                    note_decoded(&hub, id, Direction::Response, &body, encoding.as_deref());
                }
                let decision = hub.wait_break(id, BreakSide::Response).await;
                if decision.drop {
                    hub.clear_response(id);
                    hub.set_error(id, "断点丢弃");
                    hub.mark_finished(id);
                    let _ = client.shutdown().await;
                    return Ok(());
                }
                let mut next_headers = decision.headers.unwrap_or(resp_headers);
                let replaced_body = decision.body.is_some();
                let next_body = decision.body.unwrap_or(body);
                if replaced_body {
                    next_headers.retain(|(name, _)| !name.eq_ignore_ascii_case("content-encoding"));
                }
                if let Some(code) = decision.status {
                    status = code;
                }
                let reason = reason_phrase(status);
                if replaced_body {
                    hub.replace_response(id, status, next_headers.clone(), next_body.clone());
                } else {
                    hub.replace_response_head(id, status, next_headers.clone());
                }
                write_bytes(
                    &mut client,
                    status,
                    reason,
                    &next_headers,
                    &next_body,
                    throttle_down_bps,
                )
                .await?;
                hub.mark_finished(id);
                return Ok(());
            }
            Ok(None) => {
                hub.set_error(id, "响应断点跳过，body 超过 1 MB");
            }
            Err(err) => {
                let text = format!("响应断点读 body 失败: {err}");
                hub.set_error(id, text.clone());
                write_static(
                    &mut client,
                    &hub,
                    id,
                    502,
                    "Bad Gateway",
                    &[("Content-Type".into(), "text/plain".into())],
                    text.as_bytes(),
                )
                .await?;
                return Ok(());
            }
        }
    }
    if !websocket {
        let head = render_response_head(status, &reason, &resp_headers);
        client.write_all(head.as_bytes()).await?;
    }
    let encoding = header_value(&resp_headers, "content-encoding");
    let reusable = copy_message_body(
        &mut origin,
        &mut client,
        &resp_headers,
        &hub,
        id,
        Direction::Response,
        encoding,
        throttle_down_bps,
        no_resp_body,
    )
    .await?;
    hub.mark_finished(id);
    let response_close = header_value(&resp.headers, "connection")
        .is_some_and(|value| value.eq_ignore_ascii_case("close"));
    let req_has_body =
        content_length(&head.headers).is_some_and(|l| l > 0) || is_chunked(&head.headers);
    let request_body_settled =
        !has_expect || !req_has_body || request_body_copied || body_override.is_some();
    if reusable
        && request_body_settled
        && !connection_close
        && !response_close
        && status != 101
        && let Some(stream) = origin.into_idle()
    {
        hub.park_idle(pool_key, stream).await;
    }
    Ok(())
}

fn pool_key(host: &str, port: u16, hub: &Hub) -> (String, u16) {
    match hub.upstream() {
        Some(upstream) => (upstream.host, upstream.port),
        None => (host.to_string(), port),
    }
}

async fn dial(host: &str, port: u16, hub: &Hub, tunnel: bool) -> Result<TcpStream> {
    Ok(dial_timed(host, port, hub, tunnel).await?.0)
}

async fn dial_timed(
    host: &str,
    port: u16,
    hub: &Hub,
    tunnel: bool,
) -> Result<(TcpStream, crate::session::ConnInfo)> {
    let mut info = crate::session::ConnInfo::default();
    if let Some(upstream) = hub.upstream() {
        let tcp_start = Instant::now();
        let mut stream = TcpStream::connect((upstream.host.as_str(), upstream.port))
            .await
            .context("连接上游代理")?;
        stream.set_nodelay(true).ok();
        if let Ok(peer) = stream.peer_addr() {
            info.server_addr = peer.to_string();
        }
        if tunnel {
            let connect_authority = format!("{}:{port}", display_host(host));
            let auth_line = upstream
                .auth_header
                .as_deref()
                .map(|auth| format!("Proxy-Authorization: {auth}\r\n"))
                .unwrap_or_default();
            let req = format!(
                "CONNECT {connect_authority} HTTP/1.1\r\nHost: {connect_authority}\r\n{auth_line}\r\n"
            );
            stream.write_all(req.as_bytes()).await?;
            let (head, rest) = read_header_block(&mut stream).await?;
            let text = String::from_utf8_lossy(&head);
            let ok = text.starts_with("HTTP/1.1 200") || text.starts_with("HTTP/1.0 200");
            if !ok {
                anyhow::bail!(
                    "上游代理拒绝 CONNECT: {}",
                    text.lines().next().unwrap_or("").trim()
                );
            }
            if !rest.is_empty() {
                anyhow::bail!("上游 CONNECT 响应带了多余字节");
            }
        }
        info.tcp_ms = Some(tcp_start.elapsed().as_millis() as u64);
        return Ok((stream, info));
    }
    let clean_host = host.trim().trim_matches(|ch| ch == '[' || ch == ']');
    let stream = if let Ok(ip) = clean_host.parse::<std::net::IpAddr>() {
        let tcp_start = Instant::now();
        let s = TcpStream::connect((ip, port)).await?;
        info.tcp_ms = Some(tcp_start.elapsed().as_millis() as u64);
        s
    } else {
        let dns_start = Instant::now();
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port)).await?.collect();
        info.dns_ms = Some(dns_start.elapsed().as_millis() as u64);
        if addrs.is_empty() {
            anyhow::bail!("主机名没有解析到地址");
        }
        let tcp_start = Instant::now();
        let s = TcpStream::connect(&addrs[..]).await?;
        info.tcp_ms = Some(tcp_start.elapsed().as_millis() as u64);
        s
    };
    if let Ok(peer) = stream.peer_addr() {
        info.server_addr = peer.to_string();
    }
    stream.set_nodelay(true).ok();
    Ok((stream, info))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    let max_len = left.len().max(right.len());
    for idx in 0..max_len {
        let a = left.get(idx).copied().unwrap_or(0);
        let b = right.get(idx).copied().unwrap_or(0);
        diff |= (a ^ b) as usize;
    }
    diff == 0
}

pub(crate) fn verify_basic_auth(expected_user_pass: &str, header_value: &str) -> bool {
    let trimmed = header_value.trim();
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let Some(scheme) = parts.next() else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("basic") {
        return false;
    }
    let token = parts.next().unwrap_or("").trim();
    if token.is_empty() || token.len() > 2048 {
        return false;
    }
    let Some(decoded) = crate::session::base64_decode(token) else {
        return false;
    };
    constant_time_eq(&decoded, expected_user_pass.as_bytes())
}

fn decode_percent_userinfo(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0;
    while idx < bytes.len() {
        if bytes[idx] == b'%'
            && idx + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (
                (bytes[idx + 1] as char).to_digit(16),
                (bytes[idx + 2] as char).to_digit(16),
            )
        {
            out.push(((hi << 4) | lo) as u8);
            idx += 3;
            continue;
        }
        out.push(bytes[idx]);
        idx += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_upstream(text: &str) -> Option<Upstream> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        return None;
    }
    let rest = if text.len() >= 7 && text[..7].eq_ignore_ascii_case("http://") {
        &text[7..]
    } else if text.contains("://") {
        return None;
    } else {
        text
    };
    let authority = rest.split('/').next().unwrap_or(rest);
    if authority.is_empty() || authority.contains('?') || authority.contains('#') {
        return None;
    }
    let (auth_header, host_port) = match authority.rsplit_once('@') {
        Some((userinfo, hp)) => {
            if userinfo.is_empty() {
                (None, hp)
            } else {
                let (u, p) = userinfo.split_once(':').unwrap_or((userinfo, ""));
                let creds = format!(
                    "{}:{}",
                    decode_percent_userinfo(u),
                    decode_percent_userinfo(p)
                );
                let encoded = crate::session::base64_encode(creds.as_bytes());
                (Some(format!("Basic {encoded}")), hp)
            }
        }
        None => (None, authority),
    };
    if !host_port.starts_with('[') && host_port.matches(':').count() > 1 {
        return None;
    }
    let (host, port) = split_host_port(host_port, 80).ok()?;
    if host.is_empty() || port == 0 {
        return None;
    }
    Some(Upstream {
        host,
        port,
        auth_header,
    })
}

fn client_cert_rules(rules: &[rules::Rule]) -> Vec<&rules::Rule> {
    rules
        .iter()
        .filter(|r| matches!(r, rules::Rule::ClientCert { .. }))
        .collect()
}

async fn read_client_request_body<S>(
    client: &mut S,
    headers: &[(String, String)],
    has_expect: bool,
    hub: &Hub,
    id: u64,
) -> Result<Vec<u8>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let req_encoding = header_value(headers, "content-encoding");
    if let Some(len) = content_length(headers) {
        if len == 0 {
            return Ok(Vec::new());
        }
        if has_expect {
            client.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await?;
        }
        let raw = read_exact(client, len).await?;
        note_decoded(hub, id, Direction::Request, &raw, req_encoding.as_deref());
        return Ok(raw);
    }
    if is_chunked(headers) {
        if has_expect {
            client.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await?;
        }
        let mut out = Vec::new();
        loop {
            let size_line = read_line(client).await?;
            let text = String::from_utf8_lossy(&size_line);
            let size =
                usize::from_str_radix(text.trim().split(';').next().unwrap_or("0").trim(), 16)
                    .unwrap_or(0);
            if size == 0 {
                let _ = read_line(client).await?;
                break;
            }
            if out.len().saturating_add(size) > 64 * 1024 * 1024 {
                anyhow::bail!("映射请求分块 body 超过 64 MB");
            }
            out.extend(read_exact(client, size).await?);
            let _ = read_exact(client, 2).await?;
        }
        note_decoded(hub, id, Direction::Request, &out, req_encoding.as_deref());
        return Ok(out);
    }
    Ok(Vec::new())
}

async fn drain_request<S>(
    client: &mut S,
    headers: &[(String, String)],
    hub: &Hub,
    id: u64,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if header_has_expect_100_continue(headers) {
        return Ok(());
    }
    let mut sink = tokio::io::sink();
    let encoding = header_value(headers, "content-encoding");
    let _ = copy_message_body(
        client,
        &mut sink,
        headers,
        hub,
        id,
        Direction::Request,
        encoding,
        None,
        false,
    )
    .await?;
    Ok(())
}

async fn discard_request<S>(client: &mut S, headers: &[(String, String)]) -> Result<()>
where
    S: AsyncRead + Unpin,
{
    if header_has_expect_100_continue(headers) {
        return Ok(());
    }
    if let Some(length) = content_length(headers) {
        let mut left = length;
        let mut buf = vec![0u8; 16 * 1024];
        while left > 0 {
            let want = left.min(buf.len());
            let n = client.read(&mut buf[..want]).await?;
            if n == 0 {
                break;
            }
            left -= n;
        }
    } else if is_chunked(headers) {
        loop {
            let size_line = read_line(client).await?;
            let text = String::from_utf8_lossy(&size_line);
            let size =
                usize::from_str_radix(text.trim().split(';').next().unwrap_or("0").trim(), 16)
                    .unwrap_or(0);
            if size == 0 {
                let _ = read_line(client).await?;
                break;
            }
            let mut left = size;
            let mut buf = vec![0u8; 16 * 1024];
            while left > 0 {
                let want = left.min(buf.len());
                let n = client.read(&mut buf[..want]).await?;
                if n == 0 {
                    return Ok(());
                }
                left -= n;
            }
            let _ = read_exact(client, 2).await?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn write_mapped_file<S>(
    client: &mut S,
    hub: &Hub,
    id: u64,
    file: Option<&std::path::Path>,
    status: u16,
    response_ops: &[rules::HeaderOp],
    cors: bool,
    throttle_bps: Option<u64>,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let Some(file) = file else {
        let mut headers = vec![("Content-Length".to_string(), "0".to_string())];
        if cors {
            rules::ensure_cors(&mut headers);
        }
        rules::apply_header_ops(&mut headers, response_ops);
        return write_static(
            client,
            hub,
            id,
            status,
            reason_phrase(status),
            &headers,
            &[],
        )
        .await;
    };
    match std::fs::read(file) {
        Ok(body) => {
            let kind = file_type(file);
            let mut headers = vec![
                ("Content-Type".to_string(), kind.to_string()),
                ("Content-Length".to_string(), body.len().to_string()),
            ];
            if cors {
                rules::ensure_cors(&mut headers);
            }
            rules::apply_header_ops(&mut headers, response_ops);
            write_static_throttled(
                client,
                hub,
                id,
                status,
                reason_phrase(status),
                &headers,
                &body,
                throttle_bps,
            )
            .await
        }
        Err(err) => {
            let body = format!("map-local 读不到文件: {err}");
            hub.set_error(id, body.clone());
            write_static(
                client,
                hub,
                id,
                502,
                "Bad Gateway",
                &[("Content-Type".into(), "text/plain".into())],
                body.as_bytes(),
            )
            .await
        }
    }
}

async fn write_static<S>(
    client: &mut S,
    hub: &Hub,
    id: u64,
    status: u16,
    reason: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    write_static_throttled(client, hub, id, status, reason, headers, body, None).await
}

#[allow(clippy::too_many_arguments)]
async fn write_static_throttled<S>(
    client: &mut S,
    hub: &Hub,
    id: u64,
    status: u16,
    reason: &str,
    headers: &[(String, String)],
    body: &[u8],
    throttle_bps: Option<u64>,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    write_static_for_method(
        client,
        hub,
        id,
        status,
        reason,
        headers,
        body,
        throttle_bps,
        false,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn write_static_for_method<S>(
    client: &mut S,
    hub: &Hub,
    id: u64,
    status: u16,
    reason: &str,
    headers: &[(String, String)],
    body: &[u8],
    throttle_bps: Option<u64>,
    omit_body: bool,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    client.write_all(head.as_bytes()).await?;
    if !omit_body {
        write_all_throttled(client, body, throttle_bps).await?;
    }
    let mut recorded = headers.to_vec();
    if !recorded
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-length"))
    {
        recorded.push(("Content-Length".into(), body.len().to_string()));
    }
    hub.set_response(id, status, recorded, None);
    if !omit_body && !body.is_empty() {
        hub.append_body(id, Direction::Response, body);
    }
    hub.mark_finished(id);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn exchange_tls<S>(
    client: &mut S,
    hub: &Hub,
    id: u64,
    method: &str,
    host: &str,
    port: u16,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
    plan: &rules::Effect,
    break_response: bool,
    throttle_up_bps: Option<u64>,
    throttle_down_bps: Option<u64>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (mut origin, _, conn_info) =
        connect_origin(host, port, &[b"http/1.1".to_vec()], hub).await?;
    hub.set_conn_info(id, conn_info);
    let mut outbound = format!("{method} {path} HTTP/1.1\r\n");
    let mut has_host = false;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("proxy-connection")
            || name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("transfer-encoding")
        {
            continue;
        }
        if name.eq_ignore_ascii_case("host") {
            has_host = true;
        }
        outbound.push_str(name);
        outbound.push_str(": ");
        outbound.push_str(value);
        outbound.push_str("\r\n");
    }
    if !has_host {
        outbound.push_str(&format!(
            "Host: {}\r\n",
            display_authority(host, port, "https")
        ));
    }
    outbound.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    origin.write_all(outbound.as_bytes()).await?;
    write_all_throttled(&mut origin, body, throttle_up_bps).await?;
    let (mut resp_bytes, mut resp_rest) = read_header_block(&mut origin).await?;
    let mut resp = parse_head(&resp_bytes)?;
    let (mut status, mut reason) = status_and_reason(&resp.start);
    let mut interim_count = 0;
    while (100..=199).contains(&status) && status != 101 {
        interim_count += 1;
        if interim_count > 16 {
            anyhow::bail!("过多 1xx 中间响应");
        }
        client.write_all(&resp_bytes).await?;
        let (next_bytes, next_rest) = read_header_block_prefixed(&mut origin, resp_rest).await?;
        resp_bytes = next_bytes;
        resp_rest = next_rest;
        resp = parse_head(&resp_bytes)?;
        let (s, r) = status_and_reason(&resp.start);
        status = s;
        reason = r;
    }
    let mut resp_headers = resp.headers.clone();
    rules::apply_header_ops(&mut resp_headers, &plan.response_headers);
    if plan.cors {
        rules::ensure_cors(&mut resp_headers);
    }
    hub.set_response(id, status, resp_headers.clone(), None);
    let no_resp_body = response_has_no_body(method, status);
    let mut origin = Prefixed {
        prefix: resp_rest,
        pos: 0,
        inner: origin,
    };
    if !break_response
        && !no_resp_body
        && (!plan.response_replaces.is_empty() || !plan.response_json_ops.is_empty())
    {
        let buffered_resp = if let Some(len) = content_length(&resp_headers) {
            if len > crate::session::BODY_CAP {
                hub.append_comment(id, "[rule skipped: body > 1MB]");
                None
            } else {
                Some(read_exact(&mut origin, len).await?)
            }
        } else if is_chunked(&resp_headers) {
            match read_chunked_limited(&mut origin, crate::session::BODY_CAP).await? {
                LimitedRead::Whole(raw) => Some(raw),
                LimitedRead::Overflow { wire_prefix } => {
                    origin.rewind(wire_prefix);
                    hub.append_comment(id, "[rule skipped: body > 1MB]");
                    None
                }
            }
        } else {
            match read_eof_limited(&mut origin, crate::session::BODY_CAP).await? {
                LimitedRead::Whole(raw) => Some(raw),
                LimitedRead::Overflow { wire_prefix } => {
                    origin.rewind(wire_prefix);
                    hub.append_comment(id, "[rule skipped: body > 1MB]");
                    None
                }
            }
        };
        if let Some(raw) = buffered_resp {
            let encoding = header_value(&resp_headers, "content-encoding");
            if !raw.is_empty()
                && let Some(next) = rewrite_body(
                    hub,
                    id,
                    &raw,
                    encoding.as_deref(),
                    &plan.response_replaces,
                    &plan.response_json_ops,
                )
            {
                resp_headers.retain(|(name, _)| {
                    !name.eq_ignore_ascii_case("content-encoding")
                        && !name.eq_ignore_ascii_case("transfer-encoding")
                        && !name.eq_ignore_ascii_case("content-length")
                });
                resp_headers.push(("Content-Length".into(), next.len().to_string()));
                hub.replace_response(id, status, resp_headers.clone(), next.clone());
                write_bytes(
                    client,
                    status,
                    &reason,
                    &resp_headers,
                    &next,
                    throttle_down_bps,
                )
                .await?;
                hub.mark_finished(id);
                return Ok(());
            }
            resp_headers.retain(|(name, _)| {
                !name.eq_ignore_ascii_case("transfer-encoding")
                    && !name.eq_ignore_ascii_case("content-length")
            });
            resp_headers.push(("Content-Length".into(), raw.len().to_string()));
            hub.replace_response_head(id, status, resp_headers.clone());
            write_bytes(
                client,
                status,
                &reason,
                &resp_headers,
                &raw,
                throttle_down_bps,
            )
            .await?;
            note_decoded(hub, id, Direction::Response, &raw, encoding.as_deref());
            hub.mark_finished(id);
            return Ok(());
        }
    }
    if break_response {
        match read_capped_body(
            &mut origin,
            &resp_headers,
            crate::session::BODY_CAP,
            no_resp_body,
        )
        .await
        {
            Ok(Some(resp_body)) => {
                let encoding = header_value(&resp_headers, "content-encoding");
                if !resp_body.is_empty() {
                    note_decoded(
                        hub,
                        id,
                        Direction::Response,
                        &resp_body,
                        encoding.as_deref(),
                    );
                }
                let decision = hub.wait_break(id, BreakSide::Response).await;
                if decision.drop {
                    hub.clear_response(id);
                    hub.set_error(id, "断点丢弃");
                    hub.mark_finished(id);
                    let _ = client.shutdown().await;
                    return Ok(());
                }
                let mut next_headers = decision.headers.unwrap_or(resp_headers);
                let replaced_body = decision.body.is_some();
                let next_body = decision.body.unwrap_or(resp_body);
                if replaced_body {
                    next_headers.retain(|(name, _)| !name.eq_ignore_ascii_case("content-encoding"));
                }
                if let Some(code) = decision.status {
                    status = code;
                }
                let reason = reason_phrase(status);
                if replaced_body {
                    hub.replace_response(id, status, next_headers.clone(), next_body.clone());
                } else {
                    hub.replace_response_head(id, status, next_headers.clone());
                }
                write_bytes(
                    client,
                    status,
                    reason,
                    &next_headers,
                    &next_body,
                    throttle_down_bps,
                )
                .await?;
                hub.mark_finished(id);
                return Ok(());
            }
            Ok(None) => {
                hub.set_error(id, "响应断点跳过，body 超过 1 MB");
            }
            Err(err) => {
                let text = format!("响应断点读 body 失败: {err}");
                hub.set_error(id, text.clone());
                write_static(
                    client,
                    hub,
                    id,
                    502,
                    "Bad Gateway",
                    &[("Content-Type".into(), "text/plain".into())],
                    text.as_bytes(),
                )
                .await?;
                return Ok(());
            }
        }
    }
    let head = render_response_head(status, &reason, &resp_headers);
    client.write_all(head.as_bytes()).await?;
    let _ = copy_message_body(
        &mut origin,
        client,
        &resp_headers,
        hub,
        id,
        Direction::Response,
        header_value(&resp_headers, "content-encoding"),
        throttle_down_bps,
        no_resp_body,
    )
    .await?;
    hub.mark_finished(id);
    Ok(())
}

fn parse_http_url(url: &str) -> Option<(&str, String, u16, String)> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let path = if path.is_empty() {
        "/".to_string()
    } else {
        format!("/{path}")
    };
    let default_port = if scheme == "https" { 443 } else { 80 };
    let (host, port) = split_host_port(authority, default_port).ok()?;
    Some((scheme, host, port, path))
}

enum LocalRoute {
    Cert,
    Install,
    Share(String, crate::share::SharePath),
    Relay(String, crate::share::SharePath, RelayVerb),
    Favicon,
    Mcp,
}

enum RelayVerb {
    Put,
    Delete,
}

fn targets_self(local_addr: Option<SocketAddr>, head: &Head, method: &str, target: &str) -> bool {
    let Some(local_addr) = local_addr else {
        return false;
    };
    let parsed = if method.eq_ignore_ascii_case("CONNECT") {
        split_host_port(target, 443).ok()
    } else {
        request_target(head)
            .ok()
            .map(|(_, host, port, _)| (host, port))
    };
    let Some((host, port)) = parsed else {
        return false;
    };
    if port != local_addr.port() {
        return false;
    }
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified() || ip == local_addr.ip())
}

/// A request the proxy answers itself. Origin-form `/` is the install page.
/// Absolute-form `http://其他主机/` stays a proxied request. `/zniff` is the
/// page in both forms. `/zniff-ca.crt` is the certificate file.
#[cfg(test)]
fn local_route(method: &str, target: &str) -> Option<LocalRoute> {
    local_route_for(method, target, false)
}

fn local_route_for(method: &str, target: &str, self_target: bool) -> Option<LocalRoute> {
    let no_frag = target.split('#').next().unwrap_or(target);
    let (origin_form, path_and_query) = if let Some(rest) = no_frag
        .strip_prefix("http://")
        .or_else(|| no_frag.strip_prefix("https://"))
    {
        let pq = match rest.find('/') {
            Some(index) => &rest[index..],
            None => "/",
        };
        (false, pq)
    } else if no_frag.starts_with('/') {
        (true, no_frag)
    } else {
        return None;
    };
    let path = path_and_query.split('?').next().unwrap_or(path_and_query);
    let direct = origin_form || self_target;
    if direct && (path == "/mcp" || path == "/mcp/") {
        return Some(LocalRoute::Mcp);
    }
    if direct
        && (method.eq_ignore_ascii_case("PUT") || method.eq_ignore_ascii_case("DELETE"))
        && let Some((id, kind)) = crate::share::parse_share_path(path_and_query)
    {
        let verb = if method.eq_ignore_ascii_case("DELETE") {
            RelayVerb::Delete
        } else {
            RelayVerb::Put
        };
        return Some(LocalRoute::Relay(id, kind, verb));
    }
    if !method.eq_ignore_ascii_case("GET") && !method.eq_ignore_ascii_case("HEAD") {
        return None;
    }
    if path == "/zniff-ca.crt" || path.ends_with("/zniff-ca.crt") {
        return Some(LocalRoute::Cert);
    }
    if path == "/zniff" || path == "/zniff/" || (direct && path == "/") {
        return Some(LocalRoute::Install);
    }
    if direct && path == "/favicon.ico" {
        return Some(LocalRoute::Favicon);
    }
    // Only a browser hitting this proxy directly (or via proxy targeting this
    // listener port). An absolute-form request for some other host must still be forwarded.
    if direct && let Some((id, kind)) = crate::share::parse_share_path(path_and_query) {
        return Some(LocalRoute::Share(id, kind));
    }
    None
}

fn is_loopback_host(authority: &str) -> bool {
    let authority = authority.trim();
    if authority.is_empty() {
        return false;
    }
    if let Ok(ip) = authority.parse::<std::net::IpAddr>() {
        return ip.is_loopback();
    }
    let (host, _port) = match split_host_port(authority, 80) {
        Ok(res) => res,
        Err(_) => return false,
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return ip.is_loopback();
    }
    false
}

async fn serve_mcp(
    socket: &mut TcpStream,
    rest: Vec<u8>,
    hub: Arc<Hub>,
    method: &str,
    head: &Head,
) -> Result<()> {
    let is_loopback = socket.peer_addr().is_ok_and(|peer| peer.ip().is_loopback());
    if !is_loopback {
        return write_untracked(
            socket,
            403,
            "Forbidden",
            "text/plain; charset=utf-8",
            b"MCP endpoint is restricted to loopback (127.0.0.1 / ::1).",
        )
        .await;
    }
    if method.eq_ignore_ascii_case("OPTIONS") {
        return write_untracked(
            socket,
            403,
            "Forbidden",
            "text/plain; charset=utf-8",
            b"CORS preflight requests are forbidden.",
        )
        .await;
    }
    if head
        .headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("origin"))
    {
        return write_untracked(
            socket,
            403,
            "Forbidden",
            "text/plain; charset=utf-8",
            b"Cross-origin requests are forbidden on MCP endpoint.",
        )
        .await;
    }
    let host_valid = header_value(&head.headers, "host")
        .as_deref()
        .map(is_loopback_host)
        .unwrap_or(false);
    if !host_valid {
        return write_untracked(
            socket,
            403,
            "Forbidden",
            "text/plain; charset=utf-8",
            b"MCP endpoint requires loopback Host header (127.0.0.1, localhost, [::1]).",
        )
        .await;
    }
    if !method.eq_ignore_ascii_case("POST") {
        return write_untracked(
            socket,
            405,
            "Method Not Allowed",
            "text/plain; charset=utf-8",
            b"Use POST /mcp with JSON-RPC 2.0 payload.",
        )
        .await;
    }
    let is_json = header_value(&head.headers, "content-type")
        .as_deref()
        .map(|ct| {
            ct.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/json")
        })
        .unwrap_or(false);
    if !is_json {
        return write_untracked(
            socket,
            415,
            "Unsupported Media Type",
            "text/plain; charset=utf-8",
            b"MCP endpoint requires Content-Type: application/json.",
        )
        .await;
    }
    let mut reader = Prefixed {
        prefix: rest,
        pos: 0,
        inner: socket,
    };
    let body = match timeout(
        IO_TIMEOUT,
        read_capped_body(&mut reader, &head.headers, crate::session::BODY_CAP, false),
    )
    .await
    {
        Ok(Ok(Some(bytes))) => bytes,
        _ => {
            return write_untracked(
                reader.inner,
                400,
                "Bad Request",
                "application/json; charset=utf-8",
                br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Failed to read request body"}}"#,
            )
            .await;
        }
    };
    let socket = reader.inner;
    match crate::mcp::handle_http_jsonrpc(hub, &body).await {
        Some(reply) => {
            write_untracked(socket, 200, "OK", "application/json; charset=utf-8", &reply).await
        }
        None => write_untracked(socket, 202, "Accepted", "text/plain", b"").await,
    }
}

async fn serve_relay(
    socket: &mut TcpStream,
    rest: Vec<u8>,
    hub: &Hub,
    head: &Head,
    id: &str,
    kind: crate::share::SharePath,
    verb: RelayVerb,
) -> Result<()> {
    if !hub.relay_accepts() {
        return write_untracked(
            socket,
            404,
            "Not Found",
            "text/plain; charset=utf-8",
            b"Not Found",
        )
        .await;
    }
    let authorization = header_value(&head.headers, "authorization");
    if matches!(verb, RelayVerb::Delete) {
        if !hub.relay_authorized(authorization.as_deref()) {
            return write_untracked(
                socket,
                401,
                "Unauthorized",
                "text/plain; charset=utf-8",
                b"unauthorized",
            )
            .await;
        }
        return match kind {
            crate::share::SharePath::Page => {
                hub.delete_inbound(id);
                write_untracked(socket, 204, "No Content", "text/plain; charset=utf-8", b"").await
            }
            _ => {
                write_untracked(
                    socket,
                    405,
                    "Method Not Allowed",
                    "text/plain; charset=utf-8",
                    b"delete the share url",
                )
                .await
            }
        };
    }
    let body = match read_relay_body(socket, head, &rest).await {
        Ok(body) => body,
        Err(status) => {
            return write_untracked(
                socket,
                status.status,
                status.reason,
                "text/plain; charset=utf-8",
                status.message.as_bytes(),
            )
            .await;
        }
    };
    if !hub.relay_authorized(authorization.as_deref()) {
        return write_untracked(
            socket,
            401,
            "Unauthorized",
            "text/plain; charset=utf-8",
            b"unauthorized",
        )
        .await;
    }
    let result = match kind {
        crate::share::SharePath::Data { .. } => hub.put_inbound_flows(id, &body),
        crate::share::SharePath::FlowBody(flow_id) => hub.put_inbound_body(id, flow_id, &body),
        crate::share::SharePath::Page => Err(crate::relay::RelayStatus {
            status: 405,
            reason: "Method Not Allowed",
            message: "upload flows.json",
        }),
    };
    match result {
        Ok(()) => {
            write_untracked(socket, 204, "No Content", "text/plain; charset=utf-8", b"").await
        }
        Err(status) => {
            write_untracked(
                socket,
                status.status,
                status.reason,
                "text/plain; charset=utf-8",
                status.message.as_bytes(),
            )
            .await
        }
    }
}

async fn read_relay_body(
    socket: &mut TcpStream,
    head: &Head,
    rest: &[u8],
) -> Result<Vec<u8>, crate::relay::RelayStatus> {
    if is_chunked(&head.headers) {
        return Err(crate::relay::RelayStatus {
            status: 411,
            reason: "Length Required",
            message: "chunked bodies are not accepted",
        });
    }
    let Some(raw) = header_value(&head.headers, "content-length") else {
        return Err(crate::relay::RelayStatus {
            status: 411,
            reason: "Length Required",
            message: "content-length required",
        });
    };
    let len: usize = match raw.parse() {
        Ok(len) => len,
        Err(_) => {
            return Err(crate::relay::RelayStatus {
                status: 400,
                reason: "Bad Request",
                message: "bad content-length",
            });
        }
    };
    if len > crate::relay::MAX_BODY {
        return Err(crate::relay::RelayStatus {
            status: 413,
            reason: "Content Too Large",
            message: "body is too large",
        });
    }
    let mut body = Vec::with_capacity(len);
    let take = rest.len().min(len);
    body.extend_from_slice(&rest[..take]);
    while body.len() < len {
        let mut buf = [0_u8; 8192];
        let read = match timeout(Duration::from_secs(5), socket.read(&mut buf)).await {
            Ok(Ok(0)) => {
                return Err(crate::relay::RelayStatus {
                    status: 400,
                    reason: "Bad Request",
                    message: "body ended early",
                });
            }
            Ok(Ok(count)) => count,
            Ok(Err(_)) | Err(_) => {
                return Err(crate::relay::RelayStatus {
                    status: 400,
                    reason: "Bad Request",
                    message: "body read failed",
                });
            }
        };
        let need = len - body.len();
        body.extend_from_slice(&buf[..read.min(need)]);
    }
    Ok(body)
}

fn relay_drafts(doc: &crate::share::InboundShare) -> Vec<RelayDraft> {
    doc.flows
        .iter()
        .map(|flow| RelayDraft {
            remote_id: flow.id,
            protocol: protocol_from_label(&flow.protocol),
            method: flow.method.clone(),
            host: flow.host.clone(),
            path: flow.path.clone(),
            url: flow.url.clone(),
            status: flow.status,
            error: flow.error.clone(),
            request_headers: flow.request_headers.clone(),
            response_headers: flow.response_headers.clone(),
            request_body: flow.request_body.as_bytes().to_vec(),
            response_body: flow.response_body.as_bytes().to_vec(),
            request_bytes: flow.request_bytes,
            response_bytes: flow.response_bytes,
            ttfb: flow.ttfb_ms.map(millis),
            total: flow.total_ms.map(millis),
            comment: flow.comment.clone(),
            rule_hit: flow.rule_hit.clone(),
            messages: shared_messages(&flow.messages),
        })
        .collect()
}

fn millis(ms: u128) -> Duration {
    Duration::from_millis(u64::try_from(ms).unwrap_or(u64::MAX))
}

fn protocol_from_label(label: &str) -> Protocol {
    if label.eq_ignore_ascii_case("HTTP/2") {
        Protocol::Http2
    } else if label.eq_ignore_ascii_case("WebSocket") {
        Protocol::WebSocket
    } else if label == "隧道" || label.eq_ignore_ascii_case("Tunnel") {
        Protocol::Tunnel
    } else {
        Protocol::Http11
    }
}

fn shared_messages(lines: &[String]) -> Vec<Message> {
    lines
        .iter()
        .filter_map(|line| {
            let (direction, rest) = if let Some(rest) = line.strip_prefix("→ ") {
                (Direction::Request, rest)
            } else {
                let rest = line.strip_prefix("← ")?;
                (Direction::Response, rest)
            };
            let (kind, text) = rest.split_once(' ').unwrap_or((rest, ""));
            Some(Message {
                direction,
                kind: kind.to_string(),
                text: text.to_string(),
            })
        })
        .collect()
}

async fn serve_share<S>(
    socket: &mut S,
    hub: &Hub,
    head: &Head,
    id: &str,
    kind: crate::share::SharePath,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let is_head = head_method(head).eq_ignore_ascii_case("HEAD");
    let zh = crate::share::prefers_zh(header_value(&head.headers, "accept-language").as_deref());
    match kind {
        crate::share::SharePath::Data { since } => {
            let Some(json) = hub
                .share_document_since(id, since)
                .or_else(|| hub.inbound_document(id))
            else {
                let body = crate::share::expired_page(zh);
                return write_untracked_encoded_for_method(
                    socket,
                    404,
                    "Not Found",
                    "text/html; charset=utf-8",
                    None,
                    body.as_bytes(),
                    is_head,
                )
                .await;
            };
            let raw = json.into_bytes();
            let accept = header_value(&head.headers, "accept-encoding");
            let (encoding, body) = if accepts_gzip(accept.as_deref()) {
                match gzip_bytes(&raw) {
                    Some(compressed) => (Some("gzip"), compressed),
                    None => (None, raw),
                }
            } else {
                (None, raw)
            };
            write_untracked_encoded_for_method(
                socket,
                200,
                "OK",
                "application/json; charset=utf-8",
                encoding,
                &body,
                is_head,
            )
            .await
        }
        crate::share::SharePath::FlowBody(flow_id) => {
            let stored = hub
                .share_flow_body(id, flow_id)
                .map(String::into_bytes)
                .or_else(|| hub.inbound_body(id, flow_id));
            let Some(json) = stored else {
                return write_untracked_encoded_for_method(
                    socket,
                    404,
                    "Not Found",
                    "application/json; charset=utf-8",
                    None,
                    b"{}",
                    is_head,
                )
                .await;
            };
            write_untracked_encoded_for_method(
                socket,
                200,
                "OK",
                "application/json; charset=utf-8",
                None,
                &json,
                is_head,
            )
            .await
        }
        crate::share::SharePath::Page => {
            let Some(json) = hub
                .share_document_since(id, None)
                .or_else(|| hub.inbound_document(id))
            else {
                let body = crate::share::expired_page(zh);
                return write_untracked_encoded_for_method(
                    socket,
                    404,
                    "Not Found",
                    "text/html; charset=utf-8",
                    None,
                    body.as_bytes(),
                    is_head,
                )
                .await;
            };
            let live = format!("/s/{id}/flows.json");
            let body = crate::share::page(&json, &live, zh);
            write_untracked_encoded_for_method(
                socket,
                200,
                "OK",
                "text/html; charset=utf-8",
                None,
                body.as_bytes(),
                is_head,
            )
            .await
        }
    }
}

async fn write_proxy_auth_required<S>(socket: &mut S, omit_body: bool) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    const BODY: &[u8] = b"Proxy Authentication Required";
    let head = format!(
        "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"zniff\"\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        BODY.len()
    );
    socket.write_all(head.as_bytes()).await?;
    if !omit_body {
        socket.write_all(BODY).await?;
    }
    Ok(())
}

async fn write_untracked<S>(
    socket: &mut S,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    write_untracked_encoded_for_method(socket, status, reason, content_type, None, body, false)
        .await
}

async fn write_untracked_encoded_for_method<S>(
    socket: &mut S,
    status: u16,
    reason: &str,
    content_type: &str,
    content_encoding: Option<&str>,
    body: &[u8],
    omit_body: bool,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let encoding = match content_encoding {
        Some(value) if !value.is_empty() => format!("Content-Encoding: {value}\r\n"),
        _ => String::new(),
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\n{encoding}Content-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    );
    socket.write_all(head.as_bytes()).await?;
    if !omit_body {
        socket.write_all(body).await?;
    }
    Ok(())
}

fn head_method(head: &Head) -> String {
    head.start
        .split_whitespace()
        .next()
        .unwrap_or("GET")
        .to_string()
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
            let param = param.trim();
            let Some(value) = param
                .strip_prefix("q=")
                .or_else(|| param.strip_prefix("Q="))
            else {
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

const INSTALL_PAGE: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="icon" href="data:,">
<title>安装 zniff 根证书</title>
<style>
  body { margin: 0; background: #0d1017; color: #f1f5f9; font: 16px/1.55 -apple-system, BlinkMacSystemFont, sans-serif; }
  main { max-width: 28rem; margin: 0 auto; padding: 1.5rem 1.25rem 2.5rem; }
  h1 { font-size: 1.35rem; margin: 0 0 0.6rem; }
  h2 { font-size: 1rem; margin: 1.4rem 0 0.4rem; }
  p { margin: 0.4rem 0; }
  a.btn { display: block; margin: 1rem 0 0.4rem; padding: 0.85rem 1rem; border-radius: 10px; background: #2563eb; color: #fff; font-weight: 600; text-align: center; text-decoration: none; }
  ol { margin: 0.3rem 0; padding-left: 1.2rem; }
  code { font-family: "JetBrains Mono", "JetBrains Mono NL", ui-monospace, Menlo, monospace; }
  .note { color: #94a3b8; font-size: 0.9rem; }
</style>
</head>
<body>
<main>
  <h1>安装 zniff 根证书</h1>
  <p>手机和电脑在同一局域网。先下载证书，再按系统步骤信任它。</p>
  <a class="btn" href="/zniff-ca.crt">下载根证书</a>
  <h2>iPhone / iPad</h2>
  <ol>
    <li>点上面的下载。系统会提示安装描述文件。</li>
    <li>打开「设置」→「已下载描述文件」，安装 zniff CA。</li>
    <li>再到「设置」→「通用」→「关于本机」→「证书信任设置」，打开 zniff CA 的完全信任。</li>
  </ol>
  <h2>Android</h2>
  <ol>
    <li>点下载，保存 <code>zniff-ca.crt</code>。</li>
    <li>打开「设置」→「安全」→「加密与凭据」→「安装证书」→「CA 证书」。</li>
  </ol>
  <p class="note">装好后，把手机 Wi-Fi 的代理指到这台电脑的地址和端口，才能抓到手机的 HTTP(S)。</p>
</main>
</body>
</html>
"#;

fn install_page_for(accept_language: Option<&str>) -> &'static str {
    let primary = accept_language
        .unwrap_or("")
        .split(',')
        .next()
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if primary.is_empty() || primary.to_ascii_lowercase().starts_with("zh") {
        INSTALL_PAGE
    } else {
        INSTALL_PAGE_EN
    }
}

const INSTALL_PAGE_EN: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="icon" href="data:,">
<title>Install the zniff root certificate</title>
<style>
  body { margin: 0; background: #0d1017; color: #f1f5f9; font: 16px/1.55 -apple-system, BlinkMacSystemFont, sans-serif; }
  main { max-width: 28rem; margin: 0 auto; padding: 1.5rem 1.25rem 2.5rem; }
  h1 { font-size: 1.35rem; margin: 0 0 0.6rem; }
  h2 { font-size: 1rem; margin: 1.4rem 0 0.4rem; }
  p { margin: 0.4rem 0; }
  a.btn { display: block; margin: 1rem 0 0.4rem; padding: 0.85rem 1rem; border-radius: 10px; background: #2563eb; color: #fff; font-weight: 600; text-align: center; text-decoration: none; }
  ol { margin: 0.3rem 0; padding-left: 1.2rem; }
  code { font-family: "JetBrains Mono", "JetBrains Mono NL", ui-monospace, Menlo, monospace; }
  .note { color: #94a3b8; font-size: 0.9rem; }
</style>
</head>
<body>
<main>
  <h1>Install the zniff root certificate</h1>
  <p>Keep the phone and the computer on the same network. Download the certificate, then trust it.</p>
  <a class="btn" href="/zniff-ca.crt">Download the root certificate</a>
  <h2>iPhone / iPad</h2>
  <ol>
    <li>Tap the download. The system offers to install a profile.</li>
    <li>Open Settings → Profile Downloaded and install zniff CA.</li>
    <li>Then open Settings → General → About → Certificate Trust Settings and enable full trust for zniff CA.</li>
  </ol>
  <h2>Android</h2>
  <ol>
    <li>Tap the download and save <code>zniff-ca.crt</code>.</li>
    <li>Open Settings → Security → Encryption &amp; credentials → Install a certificate → CA certificate.</li>
  </ol>
  <p class="note">After it is installed, point the phone's Wi-Fi HTTP proxy at this computer's address and port to capture HTTP(S).</p>
</main>
</body>
</html>
"#;

async fn serve_install_page<S>(socket: &mut S, hub: &Hub, head: &Head) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let method = head_method(head);
    let is_head = method.eq_ignore_ascii_case("HEAD");
    let host = header_value(&head.headers, "host").unwrap_or_else(|| "zniff".into());
    let id = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method,
        host: host.clone(),
        path: "/zniff".into(),
        url: format!("http://{host}/zniff"),
        request_headers: head.headers.clone(),
    });
    let body =
        install_page_for(header_value(&head.headers, "accept-language").as_deref()).as_bytes();
    write_static_for_method(
        socket,
        hub,
        id,
        200,
        "OK",
        &[("Content-Type".into(), "text/html; charset=utf-8".into())],
        body,
        None,
        is_head,
    )
    .await
}

async fn serve_ca_cert<S>(socket: &mut S, hub: &Hub, head: &Head) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let method = head_method(head);
    let is_head = method.eq_ignore_ascii_case("HEAD");
    let pem = std::fs::read(hub.ca.cert_path()).unwrap_or_default();
    let host = header_value(&head.headers, "host").unwrap_or_else(|| "zniff".into());
    let id = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method,
        host: host.clone(),
        path: "/zniff-ca.crt".into(),
        url: format!("http://{host}/zniff-ca.crt"),
        request_headers: head.headers.clone(),
    });
    write_static_for_method(
        socket,
        hub,
        id,
        200,
        "OK",
        &[("Content-Type".into(), "application/x-x509-ca-cert".into())],
        &pem,
        None,
        is_head,
    )
    .await
}

pub async fn replay_batch(
    hub: Arc<Hub>,
    source: crate::session::Flow,
    count: u32,
    concurrency: u32,
    interval_ms: u64,
    edit: ReplayOverride,
) -> Result<Vec<u64>> {
    let total = count.clamp(
        crate::present::REPEAT_COUNT_MIN,
        crate::present::REPEAT_COUNT_MAX,
    );
    let workers = concurrency
        .clamp(
            crate::present::REPEAT_CONCURRENCY_MIN,
            crate::present::REPEAT_CONCURRENCY_MAX,
        )
        .min(total);
    let delay_ms = interval_ms.min(crate::present::REPEAT_INTERVAL_MAX_MS);
    let source_id = source.id;
    let base_comment = source.comment.clone();
    if workers <= 1 {
        let mut ids = Vec::with_capacity(total as usize);
        for idx in 0..total {
            if idx > 0 && delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            let (replayed_id, _sent) =
                replay_once_attempt(Arc::clone(&hub), Some(source.clone()), edit.clone()).await?;
            hub.set_comment(
                replayed_id,
                repeat_tag_comment(source_id, &base_comment, idx + 1, total),
            );
            ids.push(replayed_id);
        }
        return Ok(ids);
    }

    let sem = Arc::new(tokio::sync::Semaphore::new(workers as usize));
    let mut join_set = tokio::task::JoinSet::new();
    for idx in 0..total {
        if idx > 0 && delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        let permit = Arc::clone(&sem)
            .acquire_owned()
            .await
            .context("并发信号量")?;
        let hub_task = Arc::clone(&hub);
        let source_task = source.clone();
        let edit_task = edit.clone();
        let base_comment_task = base_comment.clone();
        join_set.spawn(async move {
            let _permit = permit;
            let (replayed_id, _sent) =
                replay_once_attempt(Arc::clone(&hub_task), Some(source_task), edit_task).await?;
            hub_task.set_comment(
                replayed_id,
                repeat_tag_comment(source_id, &base_comment_task, idx + 1, total),
            );
            Ok::<(u32, u64), anyhow::Error>((idx, replayed_id))
        });
    }
    let mut pairs = Vec::with_capacity(total as usize);
    while let Some(res) = join_set.join_next().await {
        let pair = res.context("批量重放任务")??;
        pairs.push(pair);
    }
    pairs.sort_by_key(|(idx, _)| *idx);
    Ok(pairs.into_iter().map(|(_, id)| id).collect())
}

fn repeat_tag_comment(source_id: u64, base_comment: &str, index: u32, total: u32) -> String {
    let tag = format!("repeat:{source_id} ({index}/{total})");
    let trimmed = base_comment.trim();
    if trimmed.is_empty() {
        tag
    } else {
        format!("{trimmed} · {tag}")
    }
}

pub(crate) async fn replay_once(
    hub: Arc<Hub>,
    source: Option<crate::session::Flow>,
    edit: ReplayOverride,
) -> Result<u64> {
    let (id, sent) = replay_once_attempt(hub, source, edit).await?;
    sent?;
    Ok(id)
}

async fn replay_once_attempt(
    hub: Arc<Hub>,
    source: Option<crate::session::Flow>,
    edit: ReplayOverride,
) -> Result<(u64, Result<()>)> {
    let custom_method = edit.method.unwrap_or_default();
    let custom_method = custom_method.trim();
    let method = if custom_method.is_empty() {
        source
            .as_ref()
            .map(|flow| flow.method.clone())
            .unwrap_or_else(|| "GET".to_string())
    } else {
        custom_method.to_string()
    };
    let custom_url = edit.url.unwrap_or_default();
    let custom_url = custom_url.trim().to_string();
    let url_overridden = !custom_url.is_empty();
    let url_for_parse: &str = if url_overridden {
        &custom_url
    } else if let Some(flow) = source.as_ref() {
        &flow.url
    } else {
        ""
    };
    let Some((scheme, host, port, path)) = parse_http_url(url_for_parse) else {
        anyhow::bail!(
            "{}",
            if url_overridden || source.is_none() {
                "编辑后的 URL 不能重放"
            } else {
                "这条记录没有可重放的 URL"
            }
        );
    };
    let mut headers = edit.headers.unwrap_or_else(|| {
        source
            .as_ref()
            .map(|flow| flow.request_headers.clone())
            .unwrap_or_default()
    });
    if url_overridden || source.is_none() {
        headers.retain(|(name, _)| !name.eq_ignore_ascii_case("host"));
    }
    let body = edit.body.unwrap_or_else(|| {
        source
            .as_ref()
            .map(|flow| flow.request_body.clone())
            .unwrap_or_default()
    });
    let id = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: method.clone(),
        host: host.clone(),
        path: path.clone(),
        url: format!(
            "{scheme}://{}{path}",
            display_authority(&host, port, scheme)
        ),
        request_headers: headers.clone(),
    });
    if let Some(flow) = source.as_ref()
        && !flow.client_ip.is_empty()
    {
        hub.with_session(|session| session.set_client_ip(id, flow.client_ip.clone()));
    }
    let prefer_h2 = url_overridden
        || matches!(
            source.as_ref().map(|flow| flow.protocol),
            None | Some(Protocol::Http2)
        );
    hub.replace_request(id, headers.clone(), body.clone());
    let sent = if scheme == "https" {
        replay_tls(
            &hub, id, &method, &host, port, &path, &headers, &body, prefer_h2,
        )
        .await
    } else {
        replay_http(&hub, id, &method, &host, port, &path, &headers, &body).await
    };
    if let Err(err) = &sent {
        hub.set_error(id, explain_flow_error(err));
    }
    Ok((id, sent))
}

#[allow(clippy::too_many_arguments)]
async fn replay_http(
    hub: &Hub,
    id: u64,
    method: &str,
    host: &str,
    port: u16,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<()> {
    let (mut origin, conn_info) = timeout(IO_TIMEOUT, dial_timed(host, port, hub, false))
        .await
        .context("连接源站超时")??;
    hub.set_conn_info(id, conn_info);
    let target = if hub.upstream().is_some() {
        format!("http://{}{path}", display_authority(host, port, "http"))
    } else {
        path.to_string()
    };
    write_replay(
        hub,
        id,
        &mut origin,
        method,
        host,
        port,
        "http",
        &target,
        headers,
        body,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn replay_tls(
    hub: &Hub,
    id: u64,
    method: &str,
    host: &str,
    port: u16,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
    prefer_h2: bool,
) -> Result<()> {
    let alpn: &[Vec<u8>] = if prefer_h2 {
        &[b"h2".to_vec(), b"http/1.1".to_vec()]
    } else {
        &[b"http/1.1".to_vec()]
    };
    let (origin, selected, conn_info) = connect_origin(host, port, alpn, hub).await?;
    hub.set_conn_info(id, conn_info);
    if selected == b"h2" {
        return replay_h2(hub, id, origin, method, host, port, path, headers, body).await;
    }
    write_replay(
        hub, id, origin, method, host, port, "https", path, headers, body,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn replay_h2<S>(
    hub: &Hub,
    id: u64,
    origin: S,
    method: &str,
    host: &str,
    port: u16,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    hub.mark_protocol(id, Protocol::Http2);
    let (mut sender, connection) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
        .handshake::<_, RecordingBody<Incoming>>(TokioIo::new(origin))
        .await
        .context("上游 HTTP/2")?;
    tokio::spawn(async move {
        if let Err(err) = connection.await {
            log::debug!("重放 HTTP/2 连接结束: {err}");
        }
    });
    let req_method = Method::from_bytes(method.as_bytes()).context("重放 HTTP 方法无效")?;
    let authority = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("host") || k.eq_ignore_ascii_case(":authority"))
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| display_authority(host, port, "https"));
    let uri = absolute_uri("https", &authority, path)?;
    let mut header_map = HeaderMap::new();
    let mut had_content_length = false;
    for (name, value) in headers {
        if name.starts_with(':') || name.eq_ignore_ascii_case("host") {
            continue;
        }
        if name.eq_ignore_ascii_case("content-length") {
            had_content_length = true;
            continue;
        }
        if name.eq_ignore_ascii_case("connection")
            || name.eq_ignore_ascii_case("proxy-connection")
            || name.eq_ignore_ascii_case("keep-alive")
            || name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("upgrade")
            || name.eq_ignore_ascii_case("proxy-authenticate")
            || name.eq_ignore_ascii_case("proxy-authorization")
            || (name.eq_ignore_ascii_case("te") && !value.trim().eq_ignore_ascii_case("trailers"))
        {
            continue;
        }
        let Ok(hname) = HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        let Ok(hval) = http::HeaderValue::from_bytes(value.as_bytes()) else {
            continue;
        };
        header_map.append(hname, hval);
    }
    if (!body.is_empty() || had_content_length)
        && let Ok(val) = http::HeaderValue::from_str(&body.len().to_string())
    {
        header_map.insert(http::header::CONTENT_LENGTH, val);
    }
    let mut outbound = Request::builder()
        .version(Version::HTTP_2)
        .method(req_method)
        .uri(uri)
        .body(RecordingBody::from_bytes(Bytes::copy_from_slice(body)))
        .context("组装上游 URI")?;
    *outbound.headers_mut() = header_map;
    if !body.is_empty() {
        hub.add_transfer(id, Direction::Request, body.len() as u64);
    }
    sender.ready().await.context("HTTP/2 上游")?;
    let response = timeout(IO_TIMEOUT, sender.send_request(outbound))
        .await
        .context("连接源站超时")?
        .context("HTTP/2 上游")?;
    let status = response.status().as_u16();
    let resp_headers = header_pairs(response.headers());
    let resp_encoding = header_value(&resp_headers, "content-encoding");
    hub.set_response(id, status, resp_headers, Some(Protocol::Http2));
    if !response_has_no_body(method, status) {
        let mut resp_body = response.into_body();
        let mut decoder = Decoder::from_header(resp_encoding.as_deref());
        if let Some(error) = decoder.error() {
            hub.set_encoding_error(id, Direction::Response, error.to_string());
        }
        while let Some(frame) = resp_body.frame().await {
            let frame = frame.context("读取响应体")?;
            if let Ok(data) = frame.into_data()
                && !data.is_empty()
            {
                let decoded = decoder.push(&data);
                let error = decoder.error().map(|err| err.to_string());
                hub.record_chunk(
                    id,
                    Direction::Response,
                    data.len() as u64,
                    &decoded,
                    error.as_deref(),
                );
            }
        }
        let tail = decoder.finish();
        let error = decoder.error().map(|err| err.to_string());
        hub.record_chunk(id, Direction::Response, 0, &tail, error.as_deref());
    }
    hub.mark_finished(id);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn write_replay<S>(
    hub: &Hub,
    id: u64,
    mut origin: S,
    method: &str,
    host: &str,
    port: u16,
    scheme: &str,
    target: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut outbound = format!("{method} {target} HTTP/1.1\r\n");
    let mut has_host = false;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("proxy-connection")
            || name.eq_ignore_ascii_case("proxy-authorization")
            || name.eq_ignore_ascii_case("proxy-authenticate")
            || name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("connection")
        {
            continue;
        }
        if name.eq_ignore_ascii_case("host") {
            has_host = true;
        }
        outbound.push_str(name);
        outbound.push_str(": ");
        outbound.push_str(value);
        outbound.push_str("\r\n");
    }
    if !has_host {
        outbound.push_str(&format!(
            "Host: {}\r\n",
            display_authority(host, port, scheme)
        ));
    }
    if scheme == "http"
        && let Some(auth) = hub.upstream().and_then(|u| u.auth_header)
    {
        outbound.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    outbound.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    origin.write_all(outbound.as_bytes()).await?;
    origin.write_all(body).await?;
    if !body.is_empty() {
        hub.add_transfer(id, Direction::Request, body.len() as u64);
    }
    let (mut resp_bytes, mut resp_rest) = read_header_block(&mut origin).await?;
    let mut resp = parse_head(&resp_bytes)?;
    let (mut status, _) = status_and_reason(&resp.start);
    let mut interim_count = 0;
    while (100..=199).contains(&status) && status != 101 {
        interim_count += 1;
        if interim_count > 16 {
            anyhow::bail!("过多 1xx 中间响应");
        }
        let (next_bytes, next_rest) = read_header_block_prefixed(&mut origin, resp_rest).await?;
        resp_bytes = next_bytes;
        resp_rest = next_rest;
        resp = parse_head(&resp_bytes)?;
        status = status_and_reason(&resp.start).0;
    }
    hub.set_response(id, status, resp.headers.clone(), None);
    let mut origin = Prefixed {
        prefix: resp_rest,
        pos: 0,
        inner: origin,
    };
    let mut sink = tokio::io::sink();
    let _ = copy_message_body(
        &mut origin,
        &mut sink,
        &resp.headers,
        hub,
        id,
        Direction::Response,
        header_value(&resp.headers, "content-encoding"),
        None,
        response_has_no_body(method, status),
    )
    .await?;
    hub.mark_finished(id);
    Ok(())
}

fn file_type(path: &std::path::Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "json" | "map" => "application/json",
        "js" | "mjs" | "cjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "wasm" => "application/wasm",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "pdf" => "application/pdf",
        "txt" | "log" | "csv" => "text/plain",
        _ => "application/octet-stream",
    }
}

const THROTTLE_CHUNK_BYTES: usize = 4096;

fn throttle_sleep_duration(bytes: usize, bps: Option<u64>) -> Duration {
    let Some(bps) = bps.filter(|&rate| rate > 0) else {
        return Duration::ZERO;
    };
    if bytes == 0 {
        return Duration::ZERO;
    }
    let nanos = (bytes as u128).saturating_mul(1_000_000_000) / (bps as u128);
    Duration::from_nanos(nanos.min(30_000_000_000) as u64)
}

async fn write_all_throttled<W>(to: &mut W, data: &[u8], bps: Option<u64>) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let Some(bps) = bps.filter(|&rate| rate > 0) else {
        to.write_all(data).await?;
        return Ok(());
    };
    for chunk in data.chunks(THROTTLE_CHUNK_BYTES) {
        to.write_all(chunk).await?;
        let pause = throttle_sleep_duration(chunk.len(), Some(bps));
        if !pause.is_zero() {
            tokio::time::sleep(pause).await;
        }
    }
    Ok(())
}

fn response_has_no_body(method: &str, status: u16) -> bool {
    method.eq_ignore_ascii_case("HEAD") || matches!(status, 100..=199 | 204 | 304)
}

#[allow(clippy::too_many_arguments)]
async fn copy_message_body<R, W>(
    from: &mut R,
    to: &mut W,
    headers: &[(String, String)],
    hub: &Hub,
    id: u64,
    direction: Direction,
    encoding: Option<String>,
    throttle_bps: Option<u64>,
    no_body: bool,
) -> Result<bool>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    if no_body {
        return Ok(true);
    }
    let mut decoder = Decoder::from_header(encoding.as_deref());
    if let Some(error) = decoder.error() {
        hub.set_encoding_error(id, direction, error.to_string());
    }
    let chunk_cap = if throttle_bps.is_some() {
        THROTTLE_CHUNK_BYTES
    } else {
        16 * 1024
    };
    let reusable = if let Some(length) = content_length(headers) {
        let mut left = length;
        let mut buf = vec![0u8; chunk_cap];
        while left > 0 {
            let want = left.min(buf.len());
            let n = from.read(&mut buf[..want]).await?;
            if n == 0 {
                break;
            }
            write_all_throttled(to, &buf[..n], throttle_bps).await?;
            let decoded = decoder.push(&buf[..n]);
            let error = decoder.error().map(|err| err.to_string());
            hub.record_chunk(id, direction, n as u64, &decoded, error.as_deref());
            left -= n;
        }
        left == 0
    } else if is_chunked(headers) {
        copy_chunked(from, to, hub, id, direction, &mut decoder, throttle_bps).await?;
        true
    } else if direction == Direction::Response {
        let mut buf = vec![0u8; chunk_cap];
        loop {
            let n = from.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            write_all_throttled(to, &buf[..n], throttle_bps).await?;
            let decoded = decoder.push(&buf[..n]);
            let error = decoder.error().map(|err| err.to_string());
            hub.record_chunk(id, direction, n as u64, &decoded, error.as_deref());
        }
        false
    } else {
        true
    };
    let tail = decoder.finish();
    let error = decoder.error().map(|err| err.to_string());
    hub.record_chunk(id, direction, 0, &tail, error.as_deref());
    Ok(reusable)
}

async fn copy_chunked<R, W>(
    from: &mut R,
    to: &mut W,
    hub: &Hub,
    id: u64,
    direction: Direction,
    decoder: &mut Decoder,
    throttle_bps: Option<u64>,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let chunk_cap = if throttle_bps.is_some() {
        THROTTLE_CHUNK_BYTES
    } else {
        16 * 1024
    };
    loop {
        let size_line = read_line(from).await?;
        to.write_all(&size_line).await?;
        let text = String::from_utf8_lossy(&size_line);
        let size_text = text.trim().split(';').next().unwrap_or("0");
        let size = usize::from_str_radix(size_text.trim(), 16).unwrap_or(0);
        if size == 0 {
            let trailer = read_line(from).await?;
            to.write_all(&trailer).await?;
            break;
        }
        let mut left = size;
        let mut buf = vec![0u8; chunk_cap];
        while left > 0 {
            let want = left.min(buf.len());
            let n = from.read(&mut buf[..want]).await?;
            if n == 0 {
                anyhow::bail!("分块 body 提前结束");
            }
            write_all_throttled(to, &buf[..n], throttle_bps).await?;
            let decoded = decoder.push(&buf[..n]);
            let error = decoder.error().map(|err| err.to_string());
            hub.record_chunk(id, direction, n as u64, &decoded, error.as_deref());
            left -= n;
        }
        let crlf = read_exact(from, 2).await?;
        to.write_all(&crlf).await?;
    }
    Ok(())
}

fn status_and_reason(start: &str) -> (u16, String) {
    let mut parts = start.split_whitespace();
    let _version = parts.next();
    let status = parts.next().and_then(|code| code.parse().ok()).unwrap_or(0);
    let reason: String = parts.collect::<Vec<_>>().join(" ");
    let reason = if reason.is_empty() {
        reason_phrase(status).to_string()
    } else {
        reason
    };
    (status, reason)
}

fn reason_phrase(status: u16) -> &'static str {
    StatusCode::from_u16(status)
        .ok()
        .and_then(|code| code.canonical_reason())
        .unwrap_or("OK")
}

fn render_response_head(status: u16, reason: &str, headers: &[(String, String)]) -> String {
    let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    head
}

async fn write_bytes<S>(
    client: &mut S,
    status: u16,
    reason: &str,
    headers: &[(String, String)],
    body: &[u8],
    throttle_bps: Option<u64>,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let mut headers: Vec<(String, String)> = headers
        .iter()
        .filter(|(name, _)| {
            !name.eq_ignore_ascii_case("content-length")
                && !name.eq_ignore_ascii_case("transfer-encoding")
        })
        .cloned()
        .collect();
    headers.push(("Content-Length".into(), body.len().to_string()));
    let head = render_response_head(status, reason, &headers);
    client.write_all(head.as_bytes()).await?;
    write_all_throttled(client, body, throttle_bps).await?;
    Ok(())
}

/// `Ok(None)` means the body is larger than `cap` and was not consumed.
async fn read_capped_body<R>(
    reader: &mut R,
    headers: &[(String, String)],
    cap: usize,
    no_body: bool,
) -> Result<Option<Vec<u8>>>
where
    R: AsyncRead + Unpin,
{
    if no_body {
        return Ok(Some(Vec::new()));
    }
    if let Some(length) = content_length(headers) {
        if length > cap {
            return Ok(None);
        }
        return Ok(Some(read_exact(reader, length).await?));
    }
    if is_chunked(headers) {
        let mut out = Vec::new();
        loop {
            let size_line = read_line(reader).await?;
            let text = String::from_utf8_lossy(&size_line);
            let size =
                usize::from_str_radix(text.trim().split(';').next().unwrap_or("0").trim(), 16)
                    .unwrap_or(0);
            if size == 0 {
                let _ = read_line(reader).await?;
                break;
            }
            if out.len().saturating_add(size) > cap {
                anyhow::bail!("响应断点跳过，body 超过 1 MB");
            }
            out.extend(read_exact(reader, size).await?);
            let _ = read_exact(reader, 2).await?;
        }
        return Ok(Some(out));
    }
    let mut out = Vec::new();
    let mut buf = vec![0u8; 8 * 1024];
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        if out.len() + n > cap {
            anyhow::bail!("响应断点跳过，body 超过 1 MB");
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok(Some(out))
}

fn content_length(headers: &[(String, String)]) -> Option<usize> {
    header_value(headers, "content-length")?.parse().ok()
}

fn is_chunked(headers: &[(String, String)]) -> bool {
    header_value(headers, "transfer-encoding")
        .map(|value| value.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false)
}

/// Drops `Sec-WebSocket-Extensions` so a handshake cannot negotiate
/// `permessage-deflate`. The forwarded frames stay uncompressed and the
/// side-copy parser can read them. RSV1 on a frame still stops parsing only.
fn strip_ws_extension(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for line in bytes.split_inclusive(|&byte| byte == b'\n') {
        let name = line.split(|&byte| byte == b':').next().unwrap_or(line);
        let name = trim_ascii(name);
        if name.eq_ignore_ascii_case(b"sec-websocket-extensions") {
            continue;
        }
        out.extend_from_slice(line);
    }
    out
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map(|index| index + 1)
        .unwrap_or(start);
    &bytes[start..end]
}

fn header_has_expect_100_continue(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("expect")
            && value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("100-continue"))
    })
}

fn header_is_websocket(headers: &[(String, String)]) -> bool {
    header_value(headers, "upgrade")
        .map(|value| value.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
}

fn header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

fn request_target(head: &Head) -> Result<(String, String, u16, String)> {
    let mut parts = head.start.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    if let Some(rest) = target
        .strip_prefix("http://")
        .or_else(|| target.strip_prefix("https://"))
    {
        let (authority, path) = match rest.find('/') {
            Some(index) => (&rest[..index], rest[index..].to_string()),
            None => (rest, "/".to_string()),
        };
        let default_port = if target.starts_with("https://") {
            443
        } else {
            80
        };
        let (host, port) = split_host_port(authority, default_port)?;
        let path = if path.is_empty() { "/".into() } else { path };
        return Ok((method, host, port, path));
    }
    let host_header = header_value(&head.headers, "host").unwrap_or_default();
    let (host, port) = split_host_port(&host_header, 80)?;
    Ok((method, host, port, target))
}

fn split_host_port(authority: &str, default_port: u16) -> Result<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, port) = rest.split_once(']').context("IPv6 主机")?;
        let port = port.trim_start_matches(':');
        let port = if port.is_empty() {
            default_port
        } else {
            port.parse().context("端口")?
        };
        return Ok((host.to_string(), port));
    }
    if let Some((host, port)) = authority.rsplit_once(':')
        && let Ok(port) = port.parse::<u16>()
    {
        return Ok((host.to_string(), port));
    }
    if authority.is_empty() {
        anyhow::bail!("缺少主机名");
    }
    Ok((authority.to_string(), default_port))
}

struct Head {
    start: String,
    headers: Vec<(String, String)>,
}

fn parse_head(bytes: &[u8]) -> Result<Head> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.split("\r\n");
    let start = lines.next().unwrap_or("").trim().to_string();
    if start.is_empty() {
        anyhow::bail!("空的请求头");
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
    Ok(Head { start, headers })
}

async fn read_header_block<R: AsyncRead + Unpin>(reader: &mut R) -> Result<(Vec<u8>, Vec<u8>)> {
    read_header_block_prefixed(reader, Vec::new()).await
}

async fn read_header_block_prefixed<R: AsyncRead + Unpin>(
    reader: &mut R,
    mut buf: Vec<u8>,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut tmp = [0u8; 2048];
    loop {
        if let Some(end) = header_end(&buf) {
            let rest = buf.split_off(end);
            return Ok((buf, rest));
        }
        if buf.len() > 64 * 1024 {
            anyhow::bail!("请求头超过 64KB");
        }
        let n = timeout(IO_TIMEOUT, reader.read(&mut tmp))
            .await
            .context("读请求头超时")??;
        if n == 0 {
            anyhow::bail!("连接在请求头结束前关闭");
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

fn header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

async fn read_line<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = reader.read(&mut byte).await?;
        if n == 0 {
            anyhow::bail!("分块大小行提前结束");
        }
        line.push(byte[0]);
        if line.ends_with(b"\r\n") {
            return Ok(line);
        }
        if line.len() > 4096 {
            anyhow::bail!("分块大小行过长");
        }
    }
}

async fn read_exact<R: AsyncRead + Unpin>(reader: &mut R, len: usize) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

enum LimitedRead {
    Whole(Vec<u8>),
    Overflow { wire_prefix: Vec<u8> },
}

async fn read_chunked_limited<R: AsyncRead + Unpin>(
    reader: &mut R,
    cap: usize,
) -> Result<LimitedRead> {
    let mut payload = Vec::new();
    let mut wire = Vec::new();
    loop {
        let size_line = read_line(reader).await?;
        wire.extend_from_slice(&size_line);
        let text = String::from_utf8_lossy(&size_line);
        let size_text = text.trim().split(';').next().unwrap_or("0").trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| anyhow!("非法分块长度"))?;
        if size == 0 {
            loop {
                let trailer = read_line(reader).await?;
                wire.extend_from_slice(&trailer);
                if trailer == b"\r\n" || trailer == b"\n" || trailer.is_empty() {
                    break;
                }
            }
            return Ok(LimitedRead::Whole(payload));
        }
        if payload.len().saturating_add(size) > cap {
            return Ok(LimitedRead::Overflow { wire_prefix: wire });
        }
        let chunk = read_exact(reader, size).await?;
        wire.extend_from_slice(&chunk);
        payload.extend_from_slice(&chunk);
        let crlf = read_exact(reader, 2).await?;
        wire.extend_from_slice(&crlf);
    }
}

async fn read_eof_limited<R: AsyncRead + Unpin>(reader: &mut R, cap: usize) -> Result<LimitedRead> {
    let mut buf = Vec::new();
    let mut scratch = vec![0u8; 8 * 1024];
    loop {
        let n = reader.read(&mut scratch).await?;
        if n == 0 {
            return Ok(LimitedRead::Whole(buf));
        }
        if buf.len().saturating_add(n) > cap {
            buf.extend_from_slice(&scratch[..n]);
            return Ok(LimitedRead::Overflow { wire_prefix: buf });
        }
        buf.extend_from_slice(&scratch[..n]);
    }
}

struct Prefixed<S> {
    prefix: Vec<u8>,
    pos: usize,
    inner: S,
}

impl<S> Prefixed<S> {
    fn rewind(&mut self, mut wire_prefix: Vec<u8>) {
        if self.pos < self.prefix.len() {
            wire_prefix.extend_from_slice(&self.prefix[self.pos..]);
        }
        self.prefix = wire_prefix;
        self.pos = 0;
    }
}

impl Prefixed<TcpStream> {
    fn into_idle(self) -> Option<TcpStream> {
        if self.pos >= self.prefix.len() {
            Some(self.inner)
        } else {
            None
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Prefixed<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.pos < self.prefix.len() {
            let n = (self.prefix.len() - self.pos).min(buf.remaining());
            buf.put_slice(&self.prefix[self.pos..self.pos + n]);
            self.pos += n;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Prefixed<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

async fn copy_bidirectional<A, B>(a: &mut A, b: &mut B) -> Result<(u64, u64)>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    Ok(tokio::io::copy_bidirectional(a, b).await?)
}

/// Forwards bytes immediately and parses WebSocket frames from the copy.
/// A bad frame stops the parser only. The splice keeps going.
async fn copy_with_messages<A, B>(
    client: &mut A,
    origin: &mut B,
    hub: &Hub,
    id: u64,
    already_to_client: &[u8],
) -> Result<()>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let mut to_origin = FrameFeed::default();
    let mut to_client = FrameFeed::default();
    if !already_to_client.is_empty() {
        note_frames(
            &mut to_client,
            already_to_client,
            hub,
            id,
            Direction::Response,
        );
    }
    let mut client_buf = vec![0u8; 16 * 1024];
    let mut origin_buf = vec![0u8; 16 * 1024];
    let mut client_done = false;
    let mut origin_done = false;
    loop {
        if client_done && origin_done {
            break;
        }
        tokio::select! {
            result = client.read(&mut client_buf), if !client_done => {
                let n = result?;
                if n == 0 {
                    client_done = true;
                    origin.shutdown().await.ok();
                } else {
                    note_frames(&mut to_origin, &client_buf[..n], hub, id, Direction::Request);
                    origin.write_all(&client_buf[..n]).await?;
                }
            }
            result = origin.read(&mut origin_buf), if !origin_done => {
                let n = result?;
                if n == 0 {
                    origin_done = true;
                    client.shutdown().await.ok();
                } else {
                    note_frames(&mut to_client, &origin_buf[..n], hub, id, Direction::Response);
                    client.write_all(&origin_buf[..n]).await?;
                }
            }
        }
    }
    Ok(())
}

fn note_frames(feed: &mut FrameFeed, data: &[u8], hub: &Hub, id: u64, direction: Direction) {
    for frame in feed.push(data) {
        hub.add_message(
            id,
            direction,
            ws::frame_label(frame.opcode),
            ws::frame_text(frame.opcode, &frame.payload),
        );
    }
}

/// Records a decompressed side copy while the original frames pass through.
struct RecordingBody<B> {
    inner: Option<B>,
    once: Option<Bytes>,
    record: Option<Record>,
    throttle_bps: Option<u64>,
    sleep: Option<Pin<Box<tokio::time::Sleep>>>,
    pending_frame: Option<Frame<Bytes>>,
}

struct Record {
    hub: Arc<Hub>,
    id: u64,
    direction: Direction,
    decoder: Decoder,
    finished: bool,
}

impl<B> RecordingBody<B> {
    fn new(inner: B, hub: Arc<Hub>, id: u64, direction: Direction, encoding: Option<&str>) -> Self {
        let decoder = Decoder::from_header(encoding);
        if let Some(error) = decoder.error() {
            hub.set_encoding_error(id, direction, error.to_string());
        }
        Self {
            inner: Some(inner),
            once: None,
            record: Some(Record {
                hub,
                id,
                direction,
                decoder,
                finished: false,
            }),
            throttle_bps: None,
            sleep: None,
            pending_frame: None,
        }
    }

    fn passthrough(inner: B) -> Self {
        Self {
            inner: Some(inner),
            once: None,
            record: None,
            throttle_bps: None,
            sleep: None,
            pending_frame: None,
        }
    }

    fn from_bytes(bytes: impl Into<Bytes>) -> Self {
        let bytes = bytes.into();
        Self {
            inner: None,
            once: if bytes.is_empty() { None } else { Some(bytes) },
            record: None,
            throttle_bps: None,
            sleep: None,
            pending_frame: None,
        }
    }

    fn with_throttle(mut self, bps: Option<u64>) -> Self {
        self.throttle_bps = bps.filter(|&rate| rate > 0);
        self
    }

    /// Yields `prefix` first, then `inner`. Only `inner` is recorded; the caller
    /// already stored `prefix`.
    fn prefixed(
        prefix: Bytes,
        inner: B,
        hub: Arc<Hub>,
        id: u64,
        direction: Direction,
        encoding: Option<&str>,
    ) -> Self {
        let mut body = Self::new(inner, hub, id, direction, encoding);
        if !prefix.is_empty() {
            body.once = Some(prefix);
        }
        body
    }

    /// Yields `prefix` first, then `inner`, feeding both through the same
    /// streaming `Decoder` when a rule-buffered stream exceeded `BODY_CAP`.
    fn stream_with_prefix(
        prefix: Bytes,
        inner: B,
        hub: Arc<Hub>,
        id: u64,
        direction: Direction,
        encoding: Option<&str>,
    ) -> Self {
        let mut body = Self::new(inner, hub, id, direction, encoding);
        if !prefix.is_empty() {
            body.record_bytes(&prefix);
            body.once = Some(prefix);
        }
        body
    }

    fn record_bytes(&mut self, bytes: &[u8]) {
        let Some(record) = self.record.as_mut() else {
            return;
        };
        let decoded = record.decoder.push(bytes);
        let error = record.decoder.error().map(|err| err.to_string());
        record.hub.record_chunk(
            record.id,
            record.direction,
            bytes.len() as u64,
            &decoded,
            error.as_deref(),
        );
    }

    fn finish_record(&mut self) {
        let Some(record) = self.record.as_mut() else {
            return;
        };
        if record.finished {
            return;
        }
        record.finished = true;
        let direction = record.direction;
        let id = record.id;
        let hub = record.hub.clone();
        let decoded = record.decoder.finish();
        record
            .hub
            .append_body(record.id, record.direction, &decoded);
        if direction == Direction::Response {
            hub.mark_finished(id);
        }
    }
}

impl<B> Drop for RecordingBody<B> {
    fn drop(&mut self) {
        // An empty body (304, HEAD) is dropped after `is_end_stream` without
        // `poll_frame` ever running.
        self.finish_record();
    }
}

impl<B> Body for RecordingBody<B>
where
    B: Body<Data = Bytes> + Unpin,
{
    type Data = Bytes;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        use std::future::Future as _;
        let this = self.get_mut();
        loop {
            if let Some(sleep) = this.sleep.as_mut() {
                if sleep.as_mut().poll(cx).is_pending() {
                    return Poll::Pending;
                }
                this.sleep = None;
            }
            if let Some(frame) = this.pending_frame.take() {
                if this.once.is_none()
                    && this
                        .inner
                        .as_ref()
                        .is_none_or(|inner| inner.is_end_stream())
                {
                    this.finish_record();
                }
                return Poll::Ready(Some(Ok(frame)));
            }
            if let Some(mut bytes) = this.once.take() {
                let chunk = if this.throttle_bps.is_some() && bytes.len() > THROTTLE_CHUNK_BYTES {
                    let head = bytes.split_to(THROTTLE_CHUNK_BYTES);
                    this.once = Some(bytes);
                    head
                } else {
                    bytes
                };
                let pause = throttle_sleep_duration(chunk.len(), this.throttle_bps);
                if !pause.is_zero() {
                    this.pending_frame = Some(Frame::data(chunk));
                    this.sleep = Some(Box::pin(tokio::time::sleep(pause)));
                    continue;
                }
                return Poll::Ready(Some(Ok(Frame::data(chunk))));
            }
            let Some(inner) = this.inner.as_mut() else {
                this.finish_record();
                return Poll::Ready(None);
            };
            match Pin::new(inner).poll_frame(cx) {
                Poll::Ready(Some(Ok(frame))) => {
                    let mut pause = Duration::ZERO;
                    if let Some(data) = frame.data_ref() {
                        let bytes = data.clone();
                        this.record_bytes(&bytes);
                        pause = throttle_sleep_duration(bytes.len(), this.throttle_bps);
                    }
                    if !pause.is_zero() {
                        this.pending_frame = Some(frame);
                        this.sleep = Some(Box::pin(tokio::time::sleep(pause)));
                        continue;
                    }
                    // hyper stops polling once `is_end_stream` is true, so the
                    // trailing `None` never arrives for the last DATA frame.
                    if this
                        .inner
                        .as_ref()
                        .is_some_and(|inner| inner.is_end_stream())
                    {
                        this.finish_record();
                    }
                    return Poll::Ready(Some(Ok(frame)));
                }
                Poll::Ready(None) => {
                    this.finish_record();
                    return Poll::Ready(None);
                }
                other => return other,
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        if self.once.is_some() || self.pending_frame.is_some() || self.sleep.is_some() {
            return false;
        }
        self.inner
            .as_ref()
            .map(|inner| inner.is_end_stream())
            .unwrap_or(true)
    }

    fn size_hint(&self) -> http_body::SizeHint {
        if let Some(bytes) = &self.once
            && self.pending_frame.is_none()
            && self.inner.is_none()
        {
            let mut hint = http_body::SizeHint::new();
            hint.set_exact(bytes.len() as u64);
            return hint;
        }
        self.inner
            .as_ref()
            .map(|inner| inner.size_hint())
            .unwrap_or_default()
    }
}

struct H1Job {
    req: Request<RecordingBody<Incoming>>,
    reply: tokio::sync::oneshot::Sender<Result<(Response<Incoming>, crate::session::ConnInfo)>>,
}

#[derive(Clone)]
struct H1Client {
    tx: tokio::sync::mpsc::UnboundedSender<H1Job>,
}

trait HttpSender: Send {
    fn send(
        &mut self,
        req: Request<RecordingBody<Incoming>>,
    ) -> impl std::future::Future<Output = Result<(Response<Incoming>, crate::session::ConnInfo)>> + Send;
}

impl HttpSender for H1Client {
    fn send(
        &mut self,
        req: Request<RecordingBody<Incoming>>,
    ) -> impl std::future::Future<Output = Result<(Response<Incoming>, crate::session::ConnInfo)>> + Send
    {
        let (reply, rx) = tokio::sync::oneshot::channel();
        let queued = self.tx.send(H1Job { req, reply });
        async move {
            queued.context("上游 HTTP/1.1 已经关闭")?;
            rx.await.context("上游 HTTP/1.1 没有返回")?
        }
    }
}

struct LaneInner {
    sender: H2Sender,
    last_ok: Instant,
    broken: bool,
    generation: u64,
    origin_epoch: u64,
    conn_info: crate::session::ConnInfo,
    first_used: Arc<AtomicBool>,
}

/// One browser HTTP/2 connection. The lane can drop a dead origin sender
/// and dial again without waiting for Chrome to open a new connection.
#[derive(Clone)]
struct H2Lane {
    lane: Arc<tokio::sync::Mutex<LaneInner>>,
    hub: Arc<Hub>,
    host: String,
    port: u16,
}

impl HttpSender for H2Lane {
    fn send(
        &mut self,
        req: Request<RecordingBody<Incoming>>,
    ) -> impl std::future::Future<Output = Result<(Response<Incoming>, crate::session::ConnInfo)>> + Send
    {
        let lane = self.lane.clone();
        let hub = self.hub.clone();
        let host = self.host.clone();
        let port = self.port;
        async move {
            let mut pending = Some(req);
            for attempt in 0..2 {
                let Some(req) = pending.take() else {
                    break;
                };
                let key = (host.clone(), port);
                let (mut sender, generation, conn_info, first_used) = {
                    let mut guard = lane.lock().await;
                    let epoch_now = hub.origin_epoch.load(Ordering::Acquire);
                    let stale = guard.broken
                        || guard.origin_epoch != epoch_now
                        || guard.sender.is_closed()
                        || guard.last_ok.elapsed() > H2_IDLE;
                    #[cfg(test)]
                    let stale = stale || hub.take_h2_refresh();
                    if stale {
                        let old = guard.generation;
                        hub.drop_h2_generation(&key, old);
                        match dial_h2(&host, port, &hub).await {
                            Ok((next, generation, conn_info, first_used)) => {
                                guard.sender = next;
                                guard.generation = generation;
                                guard.origin_epoch = hub.origin_epoch.load(Ordering::Acquire);
                                guard.conn_info = conn_info;
                                guard.first_used = first_used;
                                guard.broken = false;
                                guard.last_ok = Instant::now();
                            }
                            Err(err) => {
                                guard.broken = true;
                                return Err(err.context("HTTP/2 上游"));
                            }
                        }
                    }
                    (
                        guard.sender.clone(),
                        guard.generation,
                        guard.conn_info.clone(),
                        Arc::clone(&guard.first_used),
                    )
                };
                // `try_send_request` gives the request back only when it never
                // left. Retrying after the body was written would repeat a POST.
                // A later failure only retires this generation, so another
                // stream's replacement stays in the pool.
                match sender.try_send_request(req).await {
                    Ok(response) => {
                        let info = if !first_used.swap(true, Ordering::AcqRel) {
                            conn_info
                        } else {
                            conn_info.as_reused()
                        };
                        let mut guard = lane.lock().await;
                        if guard.generation == generation {
                            guard.last_ok = Instant::now();
                            guard.broken = false;
                        }
                        drop(guard);
                        hub.touch_h2(&key, generation);
                        return Ok((response, info));
                    }
                    Err(mut err) => {
                        let mut guard = lane.lock().await;
                        if guard.generation == generation {
                            guard.broken = true;
                        }
                        drop(guard);
                        hub.drop_h2_generation(&key, generation);
                        pending = err.take_message();
                        if attempt == 0 && pending.is_some() {
                            continue;
                        }
                        return Err(anyhow::Error::from(err.into_error()).context("HTTP/2 上游"));
                    }
                }
            }
            Err(anyhow!("HTTP/2 上游"))
        }
    }
}

async fn forward_request(
    sender: &mut impl HttpSender,
    mut req: Request<Incoming>,
    hub: Arc<Hub>,
    host: &str,
    port: u16,
    scheme: &str,
    protocol: Protocol,
) -> Result<Response<RecordingBody<Incoming>>> {
    let websocket = is_websocket(&req);
    let upgrade = if websocket {
        Some(hyper::upgrade::on(&mut req))
    } else {
        None
    };
    let encoding = req
        .headers()
        .get(http::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let (mut parts, body) = req.into_parts();
    let mut incoming = Some(body);
    let mut path = parts
        .uri
        .path_and_query()
        .map(|path| path.as_str().to_string())
        .unwrap_or_else(|| "/".into());
    let mut host = host.to_string();
    let mut port = port;
    let mut scheme = scheme.to_string();
    let authority = display_authority(&host, port, &scheme);
    let id = hub.open(FlowDraft {
        protocol,
        method: parts.method.to_string(),
        host: host.clone(),
        path: path.clone(),
        url: format!("{scheme}://{authority}{path}"),
        request_headers: header_pairs(&parts.headers),
    });
    let req_method = parts.method.clone();
    let mut body_override: Option<Vec<u8>> = None;
    let mut request_prefixed: Option<Bytes> = None;
    let mut request_prefix_recorded = true;
    let mut request_recorded = false;
    let mut break_overrode_body = false;
    let mut strip_request_encoding = false;
    let mut headers_replaced = false;
    let mut mapped_remote = false;
    let mut break_response = false;
    let mut response_ops = Vec::new();
    let mut response_replaces = Vec::new();
    let mut response_json_ops = Vec::new();
    let mut cors = false;
    let mut throttle_up_bps = None;
    let mut throttle_down_bps = None;
    if !websocket {
        let mut ops = crate::present::graphql_operations(&path, &[]);
        if hub.has_op_rules(&host, &path) {
            let known = parts
                .headers
                .get(http::header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|text| text.parse::<usize>().ok());
            let has_body = incoming.as_ref().is_some_and(|b| !b.is_end_stream());
            if has_body && known.is_none_or(|len| len <= crate::session::BODY_CAP) {
                match take_limited(
                    incoming.take().expect("request body"),
                    crate::session::BODY_CAP,
                )
                .await?
                {
                    Take::Whole(bytes) => {
                        note_decoded(&hub, id, Direction::Request, &bytes, encoding.as_deref());
                        request_recorded = true;
                        let decoded = hub.with_session(|s| {
                            s.get(id)
                                .map(|f| f.request_body.clone())
                                .unwrap_or_default()
                        });
                        ops = crate::present::graphql_operations(&path, &decoded);
                        body_override = Some(bytes.to_vec());
                    }
                    Take::Overflow { prefix, rest } => {
                        note_decoded(&hub, id, Direction::Request, &prefix, encoding.as_deref());
                        request_prefixed = Some(prefix);
                        incoming = Some(rest);
                    }
                }
            }
        }
        hub.note_rules_with_ops(id, &host, &path, &ops);
        let plan = hub.plan_with_ops(&host, &path, &ops);
        if !plan.delay.is_zero() {
            tokio::time::sleep(plan.delay).await;
        }
        response_ops = plan.response_headers.clone();
        response_replaces = plan.response_replaces.clone();
        response_json_ops = plan.response_json_ops.clone();
        cors = plan.cors;
        throttle_up_bps = plan.throttle_up_bps;
        throttle_down_bps = plan.throttle_down_bps;
        if !plan.request_headers.is_empty() {
            let mut pairs = header_pairs(&parts.headers);
            rules::apply_header_ops(&mut pairs, &plan.request_headers);
            replace_header_map(&mut parts.headers, &pairs);
            let existing = hub.with_session(|s| {
                s.get(id)
                    .map(|f| f.request_body.clone())
                    .unwrap_or_default()
            });
            hub.replace_request(id, pairs, existing);
            headers_replaced = true;
        }
        if cors && parts.method == Method::OPTIONS && plan.action.is_none() {
            if let Some(left) = incoming.take() {
                drain_incoming(left).await?;
            }
            let mut resp_headers = Vec::new();
            rules::ensure_cors(&mut resp_headers);
            rules::apply_header_ops(&mut resp_headers, &response_ops);
            resp_headers.push(("Content-Length".into(), "0".into()));
            return synthetic_headers(&hub, id, StatusCode::NO_CONTENT, resp_headers, Vec::new());
        }
        match &plan.action {
            Some(Action::MapLocal {
                path_prefix,
                file,
                status,
                fallback,
            }) => match rules::resolve_map_local_target(file.as_deref(), path_prefix, &path) {
                Ok(resolved) => {
                    if let Some(left) = incoming.take() {
                        record_request_bytes(&hub, id, left, encoding.as_deref()).await?;
                    }
                    return mapped_file_response(
                        &hub,
                        id,
                        resolved.as_deref(),
                        *status,
                        &response_ops,
                        cors,
                    )
                    .map(|res| res.map(|b| b.with_throttle(throttle_down_bps)));
                }
                Err(rules::MapLocalError::NotFound(_)) if *fallback => {
                    hub.append_comment(id, "map-local(fallback)");
                }
                Err(err) => {
                    if let Some(left) = incoming.take() {
                        record_request_bytes(&hub, id, left, encoding.as_deref()).await?;
                    }
                    let text = err.message();
                    hub.set_error(id, text.clone());
                    return synthetic_response(
                        &hub,
                        id,
                        StatusCode::BAD_GATEWAY,
                        "text/plain",
                        text.into_bytes(),
                    )
                    .map(|res| res.map(|b| b.with_throttle(throttle_down_bps)));
                }
            },
            Some(Action::BreakRequest) => {
                let decision = hub.wait_break(id, BreakSide::Request).await;
                if decision.drop {
                    if let Some(left) = incoming.take() {
                        let _ = record_request_bytes(&hub, id, left, encoding.as_deref()).await;
                    }
                    hub.set_error(id, "断点丢弃");
                    hub.mark_finished(id);
                    anyhow::bail!("断点丢弃");
                }
                if let Some(next) = decision.headers {
                    replace_header_map(&mut parts.headers, &next);
                    headers_replaced = true;
                }
                if let Some(b) = decision.body {
                    body_override = Some(b);
                    break_overrode_body = true;
                    strip_request_encoding = true;
                }
            }
            Some(Action::MapRemote { url, path_prefix }) => {
                if let Some((next_scheme, next_host, next_port, next_path)) = parse_http_url(url) {
                    let same = next_scheme == scheme && next_host == host && next_port == port;
                    host = next_host;
                    port = next_port;
                    path = rules::map_remote_path(&next_path, path_prefix, &path);
                    scheme = next_scheme.to_string();
                    let authority = display_authority(&host, port, &scheme);
                    hub.retarget(
                        id,
                        host.clone(),
                        path.clone(),
                        format!("{scheme}://{authority}{path}"),
                    );
                    if let Ok(val) = http::HeaderValue::from_str(&authority) {
                        parts.headers.insert(http::header::HOST, val);
                        let pairs = header_pairs(&parts.headers);
                        let existing = hub.with_session(|s| {
                            s.get(id)
                                .map(|f| f.request_body.clone())
                                .unwrap_or_default()
                        });
                        hub.replace_request(id, pairs, existing);
                        headers_replaced = true;
                    }
                    if !same {
                        mapped_remote = true;
                    }
                }
            }
            Some(Action::BreakResponse) => break_response = true,
            Some(Action::Block { drop_conn, status }) => {
                if let Some(left) = incoming.take() {
                    let _ = record_request_bytes(&hub, id, left, encoding.as_deref()).await;
                }
                if *drop_conn {
                    hub.set_error(id, "规则阻断 (连接已断开)");
                    hub.mark_finished(id);
                    anyhow::bail!("规则阻断 (连接已断开)");
                }
                let code = StatusCode::from_u16(*status).unwrap_or(StatusCode::FORBIDDEN);
                let mut resp_headers = vec![("Content-Length".into(), "0".into())];
                if cors {
                    rules::ensure_cors(&mut resp_headers);
                }
                rules::apply_header_ops(&mut resp_headers, &response_ops);
                return synthetic_headers(&hub, id, code, resp_headers, Vec::new());
            }
            None => {}
        }
        if let Some(next) = rules::apply_query_ops(&path, &plan.query_ops) {
            path = next;
            let authority = display_authority(&host, port, &scheme);
            hub.retarget(
                id,
                host.clone(),
                path.clone(),
                format!("{scheme}://{authority}{path}"),
            );
        }
        if let Some(next) = rules::replace_target(&path, &plan.request_replaces) {
            path = next;
            let authority = display_authority(&host, port, &scheme);
            hub.retarget(
                id,
                host.clone(),
                path.clone(),
                format!("{scheme}://{authority}{path}"),
            );
        }
        if !break_overrode_body
            && (!plan.request_replaces.is_empty() || !plan.request_json_ops.is_empty())
        {
            let known = parts
                .headers
                .get(http::header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|text| text.parse::<usize>().ok());
            if let Some(taken) = body_override.take() {
                if !taken.is_empty()
                    && let Some(next) = rewrite_body(
                        &hub,
                        id,
                        &taken,
                        encoding.as_deref(),
                        &plan.request_replaces,
                        &plan.request_json_ops,
                    )
                {
                    strip_request_encoding = true;
                    body_override = Some(next);
                } else {
                    body_override = Some(taken);
                }
            } else if request_prefixed.is_some()
                || known.is_some_and(|len| len > crate::session::BODY_CAP)
            {
                hub.append_comment(id, "[rule skipped: body > 1MB]");
            } else if incoming.as_ref().is_some_and(|b| !b.is_end_stream()) && known != Some(0) {
                match take_limited(
                    incoming.take().expect("request body"),
                    crate::session::BODY_CAP,
                )
                .await?
                {
                    Take::Whole(raw) => {
                        if !raw.is_empty()
                            && let Some(next) = rewrite_body(
                                &hub,
                                id,
                                &raw,
                                encoding.as_deref(),
                                &plan.request_replaces,
                                &plan.request_json_ops,
                            )
                        {
                            strip_request_encoding = true;
                            body_override = Some(next);
                        } else {
                            note_decoded(&hub, id, Direction::Request, &raw, encoding.as_deref());
                            request_recorded = true;
                            body_override = Some(raw.to_vec());
                        }
                    }
                    Take::Overflow { prefix, rest } => {
                        hub.append_comment(id, "[rule skipped: body > 1MB]");
                        request_prefixed = Some(prefix);
                        request_prefix_recorded = false;
                        incoming = Some(rest);
                    }
                }
            }
        }
    }
    if mapped_remote
        && body_override.is_none()
        && request_prefixed.is_none()
        && !websocket
        && !parts.headers.contains_key(http::header::CONTENT_LENGTH)
        && let Some(left) = incoming.take()
    {
        let raw = record_request_bytes(&hub, id, left, encoding.as_deref()).await?;
        request_recorded = true;
        body_override = Some(raw.to_vec());
    }
    let outbound_body = if let Some(bytes) = body_override {
        parts.headers.remove(http::header::CONTENT_LENGTH);
        parts.headers.remove(http::header::TRANSFER_ENCODING);
        if strip_request_encoding {
            parts.headers.remove(http::header::CONTENT_ENCODING);
        }
        if let Ok(value) = http::HeaderValue::from_str(&bytes.len().to_string()) {
            parts.headers.insert(http::header::CONTENT_LENGTH, value);
        }
        if let Some(left) = incoming.take() {
            drain_incoming(left).await?;
        }
        if strip_request_encoding || break_overrode_body || !request_recorded {
            hub.replace_request(id, header_pairs(&parts.headers), bytes.clone());
        } else if headers_replaced {
            let existing = hub.with_session(|s| {
                s.get(id)
                    .map(|f| f.request_body.clone())
                    .unwrap_or_default()
            });
            hub.replace_request(id, header_pairs(&parts.headers), existing);
        }
        RecordingBody::from_bytes(Bytes::from(bytes)).with_throttle(throttle_up_bps)
    } else if let Some(prefix) = request_prefixed {
        if headers_replaced {
            let existing = hub.with_session(|s| {
                s.get(id)
                    .map(|f| f.request_body.clone())
                    .unwrap_or_default()
            });
            hub.replace_request(id, header_pairs(&parts.headers), existing);
        }
        let body = if request_prefix_recorded {
            RecordingBody::prefixed(
                prefix,
                incoming.take().expect("request body"),
                hub.clone(),
                id,
                Direction::Request,
                encoding.as_deref(),
            )
        } else {
            RecordingBody::stream_with_prefix(
                prefix,
                incoming.take().expect("request body"),
                hub.clone(),
                id,
                Direction::Request,
                encoding.as_deref(),
            )
        };
        body.with_throttle(throttle_up_bps)
    } else if websocket {
        RecordingBody::passthrough(incoming.take().expect("request body"))
    } else {
        if headers_replaced {
            hub.replace_request(id, header_pairs(&parts.headers), Vec::new());
        }
        RecordingBody::new(
            incoming.take().expect("request body"),
            hub.clone(),
            id,
            Direction::Request,
            encoding.as_deref(),
        )
        .with_throttle(throttle_up_bps)
    };
    let send_result = if mapped_remote {
        fetch_remote(
            &hub,
            req_method.clone(),
            &scheme,
            &host,
            port,
            &path,
            &parts.headers,
            outbound_body,
            websocket,
        )
        .await
    } else {
        parts.version = match protocol {
            Protocol::Http2 => Version::HTTP_2,
            _ => Version::HTTP_11,
        };
        let authority = display_authority(&host, port, &scheme);
        parts.uri = absolute_uri(&scheme, &authority, &path)?;
        strip_hop_by_hop(&mut parts.headers, websocket);
        if websocket {
            parts
                .headers
                .remove(HeaderName::from_static("sec-websocket-extensions"));
        }
        let outbound = Request::from_parts(parts, outbound_body);
        sender.send(outbound).await
    };
    let mut response = match send_result {
        Ok((response, conn_info)) => {
            hub.set_conn_info(id, conn_info);
            response
        }
        Err(err) => {
            // The HTTP/2 lane already dropped its own generation. A blanket
            // drop here would remove the replacement it just opened.
            hub.set_error(id, explain_flow_error(&err));
            return Err(err);
        }
    };
    if websocket && response.status() == StatusCode::SWITCHING_PROTOCOLS {
        hub.mark_protocol(id, Protocol::WebSocket);
        if let Some(upgrade) = upgrade {
            let server_up = hyper::upgrade::on(&mut response);
            let hub_up = hub.clone();
            tokio::spawn(async move {
                splice_upgrades(upgrade, server_up, hub_up, id).await;
            });
        }
    }
    let resp_encoding = response
        .headers()
        .get(http::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let (mut parts, body) = response.into_parts();
    if websocket {
        parts
            .headers
            .remove(HeaderName::from_static("sec-websocket-extensions"));
    } else if !response_ops.is_empty() || cors {
        let mut pairs = header_pairs(&parts.headers);
        rules::apply_header_ops(&mut pairs, &response_ops);
        if cors {
            rules::ensure_cors(&mut pairs);
        }
        replace_header_map(&mut parts.headers, &pairs);
    }
    let no_resp_body = response_has_no_body(req_method.as_str(), parts.status.as_u16());
    if !websocket
        && !break_response
        && !no_resp_body
        && (!response_replaces.is_empty() || !response_json_ops.is_empty())
    {
        let known = parts
            .headers
            .get(http::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|text| text.parse::<usize>().ok());
        if known.is_some_and(|len| len > crate::session::BODY_CAP) {
            hub.append_comment(id, "[rule skipped: body > 1MB]");
        } else if !body.is_end_stream() && known != Some(0) {
            match take_limited(body, crate::session::BODY_CAP).await? {
                Take::Whole(raw) => {
                    if !raw.is_empty()
                        && let Some(next) = rewrite_body(
                            &hub,
                            id,
                            &raw,
                            resp_encoding.as_deref(),
                            &response_replaces,
                            &response_json_ops,
                        )
                    {
                        parts.headers.remove(http::header::CONTENT_ENCODING);
                        parts.headers.remove(http::header::TRANSFER_ENCODING);
                        parts.headers.remove(http::header::CONTENT_LENGTH);
                        if let Ok(value) = http::HeaderValue::from_str(&next.len().to_string()) {
                            parts.headers.insert(http::header::CONTENT_LENGTH, value);
                        }
                        return synthetic_headers(
                            &hub,
                            id,
                            parts.status,
                            header_pairs(&parts.headers),
                            next,
                        )
                        .map(|res| res.map(|b| b.with_throttle(throttle_down_bps)));
                    }
                    parts.headers.remove(http::header::TRANSFER_ENCODING);
                    parts.headers.remove(http::header::CONTENT_LENGTH);
                    if let Ok(value) = http::HeaderValue::from_str(&raw.len().to_string()) {
                        parts.headers.insert(http::header::CONTENT_LENGTH, value);
                    }
                    hub.set_response(
                        id,
                        parts.status.as_u16(),
                        header_pairs(&parts.headers),
                        None,
                    );
                    note_decoded(
                        &hub,
                        id,
                        Direction::Response,
                        &raw,
                        resp_encoding.as_deref(),
                    );
                    hub.mark_finished(id);
                    return Ok(Response::from_parts(
                        parts,
                        RecordingBody::from_bytes(raw).with_throttle(throttle_down_bps),
                    ));
                }
                Take::Overflow { prefix, rest } => {
                    hub.append_comment(id, "[rule skipped: body > 1MB]");
                    hub.set_response(
                        id,
                        parts.status.as_u16(),
                        header_pairs(&parts.headers),
                        None,
                    );
                    let body = RecordingBody::stream_with_prefix(
                        prefix,
                        rest,
                        hub,
                        id,
                        Direction::Response,
                        resp_encoding.as_deref(),
                    )
                    .with_throttle(throttle_down_bps);
                    return Ok(Response::from_parts(parts, body));
                }
            }
        }
    }
    if break_response && !websocket {
        return hold_hyper_response(hub, id, parts, body, resp_encoding)
            .await
            .map(|res| res.map(|b| b.with_throttle(throttle_down_bps)));
    }
    hub.set_response(
        id,
        parts.status.as_u16(),
        header_pairs(&parts.headers),
        None,
    );
    let body = if websocket {
        RecordingBody::passthrough(body)
    } else {
        RecordingBody::new(body, hub, id, Direction::Response, resp_encoding.as_deref())
            .with_throttle(throttle_down_bps)
    };
    Ok(Response::from_parts(parts, body))
}

async fn hold_hyper_response(
    hub: Arc<Hub>,
    id: u64,
    parts: http::response::Parts,
    body: Incoming,
    encoding: Option<String>,
) -> Result<Response<RecordingBody<Incoming>>> {
    let too_big = parts
        .headers
        .get(http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|text| text.parse::<usize>().ok())
        .is_some_and(|len| len > crate::session::BODY_CAP);
    if too_big {
        hub.set_error(id, "响应断点跳过，body 超过 1 MB");
        hub.set_response(
            id,
            parts.status.as_u16(),
            header_pairs(&parts.headers),
            None,
        );
        let body = RecordingBody::new(body, hub, id, Direction::Response, encoding.as_deref());
        return Ok(Response::from_parts(parts, body));
    }
    match take_limited(body, crate::session::BODY_CAP).await? {
        Take::Overflow { prefix, rest } => {
            hub.set_error(id, "响应断点跳过，body 超过 1 MB");
            hub.set_response(
                id,
                parts.status.as_u16(),
                header_pairs(&parts.headers),
                None,
            );
            note_decoded(&hub, id, Direction::Response, &prefix, encoding.as_deref());
            let body = RecordingBody::prefixed(
                prefix,
                rest,
                hub,
                id,
                Direction::Response,
                encoding.as_deref(),
            );
            Ok(Response::from_parts(parts, body))
        }
        Take::Whole(bytes) => {
            note_decoded(&hub, id, Direction::Response, &bytes, encoding.as_deref());
            hub.set_response(
                id,
                parts.status.as_u16(),
                header_pairs(&parts.headers),
                None,
            );
            let decision = hub.wait_break(id, BreakSide::Response).await;
            if decision.drop {
                hub.clear_response(id);
                hub.set_error(id, "断点丢弃");
                hub.mark_finished(id);
                anyhow::bail!("断点丢弃");
            }
            let status = decision.status.unwrap_or(parts.status.as_u16());
            let mut headers = decision
                .headers
                .unwrap_or_else(|| header_pairs(&parts.headers));
            let replaced_body = decision.body.is_some();
            let body = decision.body.unwrap_or_else(|| bytes.to_vec());
            if replaced_body {
                headers.retain(|(name, _)| !name.eq_ignore_ascii_case("content-encoding"));
            }
            headers.retain(|(name, _)| {
                !name.eq_ignore_ascii_case("content-length")
                    && !name.eq_ignore_ascii_case("transfer-encoding")
            });
            headers.push(("Content-Length".into(), body.len().to_string()));
            if replaced_body {
                hub.replace_response(id, status, headers.clone(), body.clone());
            } else {
                hub.replace_response_head(id, status, headers.clone());
            }
            hub.mark_finished(id);
            response_from_headers(status, &headers, &body)
        }
    }
}

enum Take {
    Whole(Bytes),
    Overflow { prefix: Bytes, rest: Incoming },
}

async fn take_limited(mut body: Incoming, cap: usize) -> Result<Take> {
    let mut buf = Vec::new();
    loop {
        match body.frame().await {
            Some(Ok(frame)) => {
                let Ok(data) = frame.into_data() else {
                    continue;
                };
                if buf.len().saturating_add(data.len()) > cap {
                    buf.extend_from_slice(&data);
                    return Ok(Take::Overflow {
                        prefix: Bytes::from(buf),
                        rest: body,
                    });
                }
                buf.extend_from_slice(&data);
            }
            Some(Err(err)) => return Err(anyhow!("读取响应体: {err}")),
            None => return Ok(Take::Whole(Bytes::from(buf))),
        }
    }
}

fn response_from_headers(
    status: u16,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<Response<RecordingBody<Incoming>>> {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
    let mut builder = Response::builder().status(code);
    let mut saw_length = false;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("content-length") {
            saw_length = true;
        }
        builder = builder.header(name, value);
    }
    if !saw_length {
        builder = builder.header(http::header::CONTENT_LENGTH, body.len().to_string());
    }
    builder
        .body(RecordingBody::from_bytes(Bytes::copy_from_slice(body)))
        .context("组装断点响应")
}

async fn record_request_bytes(
    hub: &Arc<Hub>,
    id: u64,
    body: Incoming,
    encoding: Option<&str>,
) -> Result<Bytes> {
    let collected = body
        .collect()
        .await
        .map_err(|err| anyhow!("读取请求体: {err}"))?;
    let raw = collected.to_bytes();
    note_decoded(hub, id, Direction::Request, &raw, encoding);
    Ok(raw)
}

async fn drain_incoming(body: Incoming) -> Result<()> {
    body.collect()
        .await
        .map_err(|err| anyhow!("丢掉请求体: {err}"))?;
    Ok(())
}

/// Decodes `Content-Encoding`, then applies text replacements. `None` keeps
/// the wire bytes: binary, a failed decode, or a body the pairs do not change.
fn rewrite_body(
    hub: &Hub,
    id: u64,
    raw: &[u8],
    encoding: Option<&str>,
    pairs: &[(String, String)],
    json_ops: &[rules::JsonOp],
) -> Option<Vec<u8>> {
    if (pairs.is_empty() && json_ops.is_empty()) || raw.len() > crate::session::BODY_CAP {
        if !json_ops.is_empty() && raw.len() > crate::session::BODY_CAP {
            hub.append_comment(id, "[json rule skipped: body exceeds 1 MB]");
        }
        return None;
    }
    let mut decoder = Decoder::from_header(encoding);
    let mut decoded = decoder.push(raw);
    decoded.extend(decoder.finish());
    if decoder.error().is_some() {
        if !json_ops.is_empty() {
            hub.append_comment(id, "[json rule skipped: content-encoding decode failed]");
        }
        return None;
    }
    let mut changed = false;
    if let Some(next) = rules::apply_replaces(&decoded, pairs) {
        decoded = next;
        changed = true;
    }
    if !json_ops.is_empty() {
        match rules::apply_json_ops(&decoded, json_ops) {
            Ok(Some(next)) => {
                decoded = next;
                changed = true;
            }
            Ok(None) => {}
            Err(reason) => {
                hub.append_comment(id, &format!("[json rule skipped: {reason}]"));
            }
        }
    }
    changed.then_some(decoded)
}

fn note_decoded(hub: &Hub, id: u64, direction: Direction, raw: &[u8], encoding: Option<&str>) {
    let mut decoder = Decoder::from_header(encoding);
    let early = decoder.error().map(|err| err.to_string());
    let mut decoded = decoder.push(raw);
    let tail = decoder.finish();
    if !tail.is_empty() {
        decoded.extend(tail);
    }
    let error = decoder.error().map(|err| err.to_string()).or(early);
    hub.record_chunk(id, direction, raw.len() as u64, &decoded, error.as_deref());
}

fn mapped_file_response(
    hub: &Arc<Hub>,
    id: u64,
    file: Option<&std::path::Path>,
    status: u16,
    response_ops: &[rules::HeaderOp],
    cors: bool,
) -> Result<Response<RecordingBody<Incoming>>> {
    let Some(file) = file else {
        let code = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
        let mut headers = vec![("Content-Length".into(), "0".into())];
        if cors {
            rules::ensure_cors(&mut headers);
        }
        rules::apply_header_ops(&mut headers, response_ops);
        return synthetic_headers(hub, id, code, headers, Vec::new());
    };
    match std::fs::read(file) {
        Ok(body) => {
            let code = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
            let mut headers = vec![
                ("Content-Type".into(), file_type(file).to_string()),
                ("Content-Length".into(), body.len().to_string()),
            ];
            if cors {
                rules::ensure_cors(&mut headers);
            }
            rules::apply_header_ops(&mut headers, response_ops);
            synthetic_headers(hub, id, code, headers, body)
        }
        Err(err) => {
            let text = format!("map-local 读不到文件: {err}");
            hub.set_error(id, text.clone());
            synthetic_response(
                hub,
                id,
                StatusCode::BAD_GATEWAY,
                "text/plain",
                text.into_bytes(),
            )
        }
    }
}

fn synthetic_response(
    hub: &Arc<Hub>,
    id: u64,
    status: StatusCode,
    content_type: &str,
    body: Vec<u8>,
) -> Result<Response<RecordingBody<Incoming>>> {
    let headers = vec![
        ("Content-Type".into(), content_type.to_string()),
        ("Content-Length".into(), body.len().to_string()),
    ];
    synthetic_headers(hub, id, status, headers, body)
}

fn synthetic_headers(
    hub: &Arc<Hub>,
    id: u64,
    status: StatusCode,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) -> Result<Response<RecordingBody<Incoming>>> {
    hub.set_response(id, status.as_u16(), headers.clone(), None);
    hub.append_body(id, Direction::Response, &body);
    hub.mark_finished(id);
    response_from_headers(status.as_u16(), &headers, &body)
}

fn replace_header_map(headers: &mut HeaderMap, pairs: &[(String, String)]) {
    headers.clear();
    for (name, value) in pairs {
        let Ok(name) = HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        let Ok(value) = http::HeaderValue::from_bytes(value.as_bytes()) else {
            continue;
        };
        headers.append(name, value);
    }
}

#[allow(clippy::too_many_arguments)]
async fn fetch_remote(
    hub: &Arc<Hub>,
    method: Method,
    scheme: &str,
    host: &str,
    port: u16,
    path: &str,
    headers: &HeaderMap,
    body: RecordingBody<Incoming>,
    websocket: bool,
) -> Result<(Response<Incoming>, crate::session::ConnInfo)> {
    if scheme == "https" {
        let (tls, _, conn_info) = connect_origin(host, port, &[b"http/1.1".to_vec()], hub).await?;
        let resp = fetch_on(
            tls, hub, method, scheme, host, port, path, headers, body, websocket,
        )
        .await?;
        Ok((resp, conn_info))
    } else {
        let (tcp, conn_info) = dial_timed(host, port, hub, false).await?;
        let resp = fetch_on(
            tcp, hub, method, scheme, host, port, path, headers, body, websocket,
        )
        .await?;
        Ok((resp, conn_info))
    }
}

#[allow(clippy::too_many_arguments)]
async fn fetch_on<IO>(
    io: IO,
    hub: &Arc<Hub>,
    method: Method,
    scheme: &str,
    host: &str,
    port: u16,
    path: &str,
    headers: &HeaderMap,
    body: RecordingBody<Incoming>,
    websocket: bool,
) -> Result<Response<Incoming>>
where
    IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let authority = display_authority(host, port, scheme);
    let upstream = hub.upstream();
    let via_http_upstream = upstream.is_some() && scheme == "http";
    let target_uri = if via_http_upstream {
        absolute_uri(scheme, &authority, path)?
    } else {
        let p = if path.is_empty() { "/" } else { path };
        p.parse::<Uri>().context("组装上游 URI")?
    };
    let mut outbound = Request::builder()
        .method(method)
        .uri(target_uri)
        .body(body)
        .context("组装映射请求")?;
    *outbound.headers_mut() = headers.clone();
    strip_hop_by_hop(outbound.headers_mut(), websocket);
    if via_http_upstream
        && let Some(auth) = upstream.and_then(|u| u.auth_header)
        && let Ok(val) = http::HeaderValue::from_str(&auth)
    {
        outbound
            .headers_mut()
            .insert(http::header::PROXY_AUTHORIZATION, val);
    }
    if websocket {
        outbound
            .headers_mut()
            .remove(HeaderName::from_static("sec-websocket-extensions"));
    } else {
        outbound.headers_mut().insert(
            http::header::CONNECTION,
            http::HeaderValue::from_static("close"),
        );
    }
    if let Ok(value) = http::HeaderValue::from_str(&authority) {
        outbound.headers_mut().insert(http::header::HOST, value);
    }
    let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
        .preserve_header_case(true)
        .handshake::<_, RecordingBody<Incoming>>(TokioIo::new(io))
        .await
        .context("映射连接")?;
    tokio::spawn(async move {
        let _ = connection.with_upgrades().await;
    });
    sender.ready().await.context("映射连接未就绪")?;
    sender.send_request(outbound).await.context("映射请求")
}

fn is_websocket(req: &Request<Incoming>) -> bool {
    req.headers()
        .get(http::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
        || req.method() == Method::CONNECT
}

fn header_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.to_string(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect()
}

fn strip_hop_by_hop(headers: &mut HeaderMap, websocket: bool) {
    let listed = connection_tokens(headers);
    let names: Vec<HeaderName> = headers.keys().cloned().collect();
    for name in names {
        if hop_by_hop(&name, &listed, websocket) {
            headers.remove(&name);
        }
    }
}

fn connection_tokens(headers: &HeaderMap) -> Vec<HeaderName> {
    headers
        .get_all(http::header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|token| HeaderName::from_bytes(token.trim().as_bytes()).ok())
        .collect()
}

fn hop_by_hop(name: &HeaderName, listed: &[HeaderName], websocket: bool) -> bool {
    if websocket
        && (name == http::header::CONNECTION
            || name == http::header::UPGRADE
            || name.as_str().starts_with("sec-websocket-"))
    {
        return false;
    }
    if listed.iter().any(|token| token == name) && name != http::header::CONNECTION {
        return true;
    }
    matches!(
        name.as_str(),
        "connection"
            | "proxy-connection"
            | "keep-alive"
            | "transfer-encoding"
            | "te"
            | "trailer"
            | "upgrade"
            | "proxy-authenticate"
            | "proxy-authorization"
    )
}

fn absolute_uri(scheme: &str, host: &str, path: &str) -> Result<Uri> {
    let path = if path.is_empty() { "/" } else { path };
    format!("{scheme}://{host}{path}")
        .parse()
        .context("组装上游 URI")
}

async fn splice_upgrades(
    client: impl std::future::Future<Output = Result<Upgraded, hyper::Error>>,
    server: impl std::future::Future<Output = Result<Upgraded, hyper::Error>>,
    hub: Arc<Hub>,
    id: u64,
) {
    let (client, server) = tokio::join!(client, server);
    let Ok(client) = client else {
        hub.mark_finished(id);
        return;
    };
    let Ok(server) = server else {
        hub.mark_finished(id);
        return;
    };
    let mut client = TokioIo::new(client);
    let mut server = TokioIo::new(server);
    let _ = copy_with_messages(&mut client, &mut server, &hub, id, &[]).await;
    hub.mark_finished(id);
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_port_is_the_side_before_the_arrow() {
        assert_eq!(
            super::local_endpoint_port("127.0.0.1:54321->1.2.3.4:443"),
            Some(54321)
        );
        assert_eq!(
            super::local_endpoint_port("[::1]:54321->1.2.3.4:443"),
            Some(54321)
        );
        assert_eq!(
            super::local_endpoint_port("10.0.0.5:8080->1.2.3.4:80"),
            Some(8080)
        );
        assert_eq!(
            super::local_endpoint_port("192.168.1.5:443->192.168.1.9:54321"),
            Some(443)
        );
        assert_eq!(super::local_endpoint_port("127.0.0.1:443"), Some(443));
    }

    #[test]
    fn forward_errors_include_a_guess() {
        let dead = anyhow::Error::msg("connection was not ready").context("HTTP/2 上游");
        let text = super::explain_flow_error(&dead);
        assert!(text.starts_with("HTTP/2 上游。猜测："), "{text}");
        assert!(text.contains("重新连接"), "{text}");

        let reset = anyhow::Error::from(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "connection reset",
        ))
        .context("HTTP/2 上游");
        assert!(super::explain_flow_error(&reset).contains("重置"));

        let timeout = anyhow::Error::msg("deadline has elapsed").context("连接源站超时");
        assert!(super::explain_flow_error(&timeout).contains("时限"));

        let clear = anyhow::Error::msg("编辑后的 URL 不能重放");
        assert_eq!(super::explain_flow_error(&clear), "编辑后的 URL 不能重放");

        let h1 = anyhow::Error::msg("upstream").context("上游 HTTP/1.1 没有返回");
        assert!(super::explain_flow_error(&h1).contains("没有把响应"));

        let closed = anyhow::Error::msg("gone").context("上游 HTTP/1.1 已经关闭");
        assert!(super::explain_flow_error(&closed).contains("HTTP/1.1 连接已经结束"));

        let mapped = anyhow::Error::msg("gone").context("映射连接未就绪");
        assert!(super::explain_flow_error(&mapped).contains("转发规则"));

        let kept = anyhow::Error::msg("证书被拒绝。猜测：客户端不信任 zniff CA。");
        assert_eq!(super::explain_flow_error(&kept), kept.to_string());
    }

    #[test]
    fn client_tls_reason_guesses_pinning_or_an_untrusted_ca() {
        let reset = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "connection reset");
        assert!(super::client_tls_reason(&reset, "www.google.com").contains("证书钉扎"));
        assert!(super::client_tls_reason(&reset, "api.example.com").contains("zniff CA"));
        let alert = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::AlertReceived(rustls::AlertDescription::UnknownCA),
        );
        assert!(super::client_tls_reason(&alert, "api.example.com").contains("不信任 zniff CA"));
        assert!(!super::likely_pinned("notgoogle.com"));
        assert!(super::likely_pinned("mail.google.com"));
    }

    #[test]
    fn origin_tls_reason_names_the_certificate_or_handshake() {
        let unknown = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer),
        );
        let unknown = anyhow::Error::from(unknown).context("源站 TLS");
        assert_eq!(
            super::origin_tls_reason(&unknown),
            "源站证书不在内置根证书里，已改为隧道"
        );

        let alert = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::AlertReceived(rustls::AlertDescription::HandshakeFailure),
        );
        let alert = anyhow::Error::from(alert).context("源站 TLS");
        assert_eq!(
            super::origin_tls_reason(&alert),
            "源站中止握手（HandshakeFailure），已改为隧道"
        );

        let reset = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "connection reset");
        let reset = anyhow::Error::from(reset).context("源站 TLS");
        assert_eq!(
            super::origin_tls_reason(&reset),
            "源站 TLS 失败，已改为隧道: connection reset"
        );
    }

    #[test]
    fn origin_tls_reason_differentiates_client_cert_and_preserves_context() {
        // 1. Encrypted key error preservation
        let encrypted_root = anyhow::anyhow!(
            "私钥已加密（ENCRYPTED PRIVATE KEY），zniff 暂不支持加密私钥；请先解密后再配置（例如：openssl pkcs8 -in encrypted.key -out decrypted.key）"
        );
        let err = encrypted_root
            .context("解析私钥 /var/certs/badssl.key")
            .context("读取客户端证书 /var/certs/badssl.crt")
            .context("加载源站 mTLS 客户端证书 (client.badssl.com)");

        let reason = super::origin_tls_reason(&err);
        assert!(
            reason.starts_with("客户端证书加载失败: "),
            "must have client cert failure prefix: {reason}"
        );
        assert!(
            reason.contains("client.badssl.com"),
            "must preserve host: {reason}"
        );
        assert!(
            reason.contains("/var/certs/badssl.crt"),
            "must preserve cert path: {reason}"
        );
        assert!(
            reason.contains("/var/certs/badssl.key"),
            "must preserve key path: {reason}"
        );
        assert!(
            reason.contains("ENCRYPTED PRIVATE KEY"),
            "must preserve encrypted key hint: {reason}"
        );
        assert!(
            reason.contains("openssl pkcs8 -in encrypted.key -out decrypted.key"),
            "must preserve openssl remediation command: {reason}"
        );
        assert!(
            !reason.contains("源站 TLS 失败"),
            "must NOT be labeled as origin TLS failure: {reason}"
        );

        // 2. Client auth key mismatch preservation
        let mismatch = rustls::Error::InconsistentKeys(rustls::InconsistentKeys::Unknown);
        let err = anyhow::Error::from(mismatch).context(
            "配置源站 mTLS 客户端证书 (client.badssl.com, cert=/var/certs/badssl.crt, key=/var/certs/badssl.key)",
        );
        let reason = super::origin_tls_reason(&err);
        assert!(
            reason.starts_with("客户端证书加载失败: "),
            "must have client cert failure prefix: {reason}"
        );
        assert!(
            reason.contains("client.badssl.com"),
            "must preserve host: {reason}"
        );
        assert!(
            reason.contains("/var/certs/badssl.crt") && reason.contains("/var/certs/badssl.key"),
            "must preserve cert and key paths on KeyMismatch: {reason}"
        );
        assert!(
            reason.contains("KeyMismatch") || reason.contains("keys may not be consistent"),
            "must preserve mismatch details: {reason}"
        );
        assert!(
            !reason.contains("源站 TLS 失败"),
            "must NOT be labeled as origin TLS failure: {reason}"
        );

        // 3. File not found preservation
        let io_err = std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No such file or directory (os error 2)",
        );
        let err = anyhow::Error::from(io_err)
            .context("读取私钥文件 /var/certs/missing.key")
            .context("读取客户端证书 /var/certs/missing.crt")
            .context("加载源站 mTLS 客户端证书 (api.internal.net)");
        let reason = super::origin_tls_reason(&err);
        assert!(
            reason.starts_with("客户端证书加载失败: "),
            "must have client cert failure prefix: {reason}"
        );
        assert!(
            reason.contains("api.internal.net"),
            "must preserve host: {reason}"
        );
        assert!(
            reason.contains("/var/certs/missing.key"),
            "must preserve missing file: {reason}"
        );
        assert!(
            reason.contains("No such file or directory"),
            "must preserve io error: {reason}"
        );
    }

    #[tokio::test]
    async fn client_cert_loading_failure_records_clear_reason() {
        let dir = std::env::temp_dir().join(format!("zniff-mtls-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let client_certs_dir = dir.join("client-certs");
        std::fs::create_dir_all(&client_certs_dir).unwrap();

        let (_, cert, _) = test_origin_cert();
        std::fs::write(client_certs_dir.join("mtls.badssl.com.crt"), cert.pem()).unwrap();
        std::fs::write(
            client_certs_dir.join("mtls.badssl.com.key"),
            "-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIF\n-----END ENCRYPTED PRIVATE KEY-----\n",
        )
        .unwrap();

        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());

        let res = hub.client_config_for_host("mtls.badssl.com", &[b"http/1.1".to_vec()]);
        assert!(res.is_err());
        let err = res.unwrap_err();
        let reason = super::origin_tls_reason(&err);

        assert!(reason.starts_with("客户端证书加载失败: "));
        assert!(reason.contains("mtls.badssl.com"));
        assert!(reason.contains("mtls.badssl.com.crt"));
        assert!(reason.contains("mtls.badssl.com.key"));
        assert!(reason.contains("ENCRYPTED PRIVATE KEY"));
        assert!(reason.contains("openssl pkcs8 -in encrypted.key -out decrypted.key"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_is_loopback_host() {
        assert!(super::is_loopback_host("localhost"));
        assert!(super::is_loopback_host("localhost:8877"));
        assert!(super::is_loopback_host("LOCALHOST:8877"));
        assert!(super::is_loopback_host("127.0.0.1"));
        assert!(super::is_loopback_host("127.0.0.1:8877"));
        assert!(super::is_loopback_host("127.0.0.2:8877"));
        assert!(super::is_loopback_host("[::1]"));
        assert!(super::is_loopback_host("[::1]:8877"));
        assert!(super::is_loopback_host("::1"));

        assert!(!super::is_loopback_host(""));
        assert!(!super::is_loopback_host("attacker.example"));
        assert!(!super::is_loopback_host("attacker.example:8877"));
        assert!(!super::is_loopback_host("192.168.1.10:8877"));
        assert!(!super::is_loopback_host("0.0.0.0:8877"));
        assert!(!super::is_loopback_host("evil.localhost:8877"));
        assert!(!super::is_loopback_host("localhost.evil.com:8877"));
    }

    #[tokio::test]
    async fn local_mcp_security_checks() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-mcp-sec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());

        let payload = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;

        // 1. Foreign Host (DNS-rebinding attempt) -> 403 Forbidden
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: attacker.example:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            proxy_addr.port(),
            payload.len()
        );
        let mut full_req = req.into_bytes();
        full_req.extend_from_slice(payload);
        let mut sock = TcpStream::connect(proxy_addr).await.unwrap();
        sock.write_all(&full_req).await.unwrap();
        let mut raw = Vec::new();
        sock.read_to_end(&mut raw).await.unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(
            text.starts_with("HTTP/1.1 403 Forbidden\r\n"),
            "Foreign Host must be rejected with 403: {text}"
        );

        // 2. Cross-origin request with Origin header -> 403 Forbidden
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: http://attacker.example\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            proxy_addr.port(),
            payload.len()
        );
        let mut full_req = req.into_bytes();
        full_req.extend_from_slice(payload);
        let mut sock = TcpStream::connect(proxy_addr).await.unwrap();
        sock.write_all(&full_req).await.unwrap();
        let mut raw = Vec::new();
        sock.read_to_end(&mut raw).await.unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(
            text.starts_with("HTTP/1.1 403 Forbidden\r\n"),
            "Request with Origin must be rejected with 403: {text}"
        );

        // 3. Non-JSON Content-Type (CORS simple request with text/plain) -> 415 Unsupported Media Type
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            proxy_addr.port(),
            payload.len()
        );
        let mut full_req = req.into_bytes();
        full_req.extend_from_slice(payload);
        let mut sock = TcpStream::connect(proxy_addr).await.unwrap();
        sock.write_all(&full_req).await.unwrap();
        let mut raw = Vec::new();
        sock.read_to_end(&mut raw).await.unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(
            text.starts_with("HTTP/1.1 415 Unsupported Media Type\r\n"),
            "Non-JSON Content-Type must be rejected with 415: {text}"
        );

        // 4. CORS preflight OPTIONS request -> 403 Forbidden
        let req = format!(
            "OPTIONS /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: http://attacker.example\r\nAccess-Control-Request-Method: POST\r\nConnection: close\r\n\r\n",
            proxy_addr.port()
        );
        let mut sock = TcpStream::connect(proxy_addr).await.unwrap();
        sock.write_all(req.as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        sock.read_to_end(&mut raw).await.unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(
            text.starts_with("HTTP/1.1 403 Forbidden\r\n"),
            "CORS preflight OPTIONS must be rejected with 403: {text}"
        );

        // 5. Valid loopback request with localhost -> 200 OK
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost:{}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            proxy_addr.port(),
            payload.len()
        );
        let mut full_req = req.into_bytes();
        full_req.extend_from_slice(payload);
        let mut sock = TcpStream::connect(proxy_addr).await.unwrap();
        sock.write_all(&full_req).await.unwrap();
        let mut raw = Vec::new();
        sock.read_to_end(&mut raw).await.unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(
            text.starts_with("HTTP/1.1 200 OK\r\n"),
            "Valid localhost request must succeed: {text}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_read_header_block_prefixed() {
        let initial = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec();
        let mut empty_stream = tokio::io::empty();
        let (head, rest) = super::read_header_block_prefixed(&mut empty_stream, initial)
            .await
            .unwrap();
        assert_eq!(head, b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n");
        assert_eq!(rest, b"hello");

        // Partial in prefix, rest in stream
        let initial = b"HTTP/1.1 200 OK\r\n".to_vec();
        let stream_bytes = b"Content-Length: 0\r\n\r\n";
        let mut stream = stream_bytes.as_slice();
        let (head, rest) = super::read_header_block_prefixed(&mut stream, initial)
            .await
            .unwrap();
        assert_eq!(head, b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        assert_eq!(rest, b"");
    }

    #[tokio::test]
    async fn plain_http_expect_100_continue_records_final_response() {
        let dir = std::env::temp_dir().join(format!("zniff-100-continue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        let req_body = b"hello-upload-data-12345";
        tokio::spawn(async move {
            let (mut sock, _) = origin.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let mut total_read = 0;
            while !buf[..total_read].windows(4).any(|w| w == b"\r\n\r\n") {
                let n = sock.read(&mut buf[total_read..]).await.unwrap();
                if n == 0 {
                    break;
                }
                total_read += n;
            }
            let head = String::from_utf8_lossy(&buf[..total_read]);
            assert!(
                head.to_ascii_lowercase().contains("expect: 100-continue"),
                "Origin must see Expect: 100-continue header"
            );
            // Verify origin has NOT received any body bytes yet because client waits for 100 Continue
            let head_len = buf[..total_read]
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|p| p + 4)
                .unwrap();
            assert_eq!(
                total_read, head_len,
                "Origin must not receive any body before sending 100 Continue"
            );

            // Origin sends 100 Continue interim response
            sock.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
                .await
                .unwrap();

            // Origin now reads the upload body
            let mut body_buf = Vec::new();
            while body_buf.len() < req_body.len() {
                let n = sock.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                body_buf.extend_from_slice(&buf[..n]);
            }
            assert_eq!(&body_buf[..req_body.len()], req_body);

            // Origin sends final 200 OK response with body
            let body = b"{\"result\":\"ok\"}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
            let mut drain = [0u8; 64];
            while let Ok(n) = sock.read(&mut drain).await {
                if n == 0 {
                    break;
                }
            }
        });
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "POST http://{origin_addr}/upload HTTP/1.1\r\nHost: {origin_addr}\r\nExpect: 100-continue\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            req_body.len()
        );
        // Client sends headers ONLY
        client.write_all(req.as_bytes()).await.unwrap();

        // Client explicitly awaits HTTP/1.1 100 Continue before sending body
        let mut interim_buf = Vec::new();
        let mut byte = [0u8; 1];
        while !interim_buf.ends_with(b"\r\n\r\n") {
            let n = timeout(Duration::from_secs(3), client.read(&mut byte))
                .await
                .expect("Client timed out waiting for 100 Continue (proxy deadlock detected!)")
                .unwrap();
            assert_ne!(n, 0, "Proxy closed connection before sending 100 Continue");
            interim_buf.push(byte[0]);
            assert!(
                interim_buf.len() < 1024,
                "Interim response header too large"
            );
        }
        let interim_text = String::from_utf8_lossy(&interim_buf);
        assert!(
            interim_text.starts_with("HTTP/1.1 100"),
            "Expected 100 Continue, got: {interim_text}"
        );

        // Client sends request body now that 100 Continue has arrived
        client.write_all(req_body).await.unwrap();

        // Client reads final 200 OK response
        let mut resp = Vec::new();
        let _ = timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
            .await
            .expect("Client timed out waiting for final 200 OK")
            .unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(
            text.contains("HTTP/1.1 200 OK"),
            "client must receive final 200 OK: {text}"
        );
        assert!(
            text.contains("{\"result\":\"ok\"}"),
            "client must receive body: {text}"
        );

        let flow = hub.with_session(|session| session.get(1).cloned());
        let flow = flow.expect("flow");
        assert_eq!(flow.status, Some(200));
        assert_eq!(flow.request_body, req_body);
        assert_eq!(flow.response_body, b"{\"result\":\"ok\"}");
        assert!(
            flow.elapsed_ms().is_some(),
            "flow must be marked finished with elapsed time"
        );
        let has_ct = flow
            .response_headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v.contains("application/json"));
        assert!(has_ct, "response headers must be recorded");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn plain_http_expect_100_continue_early_rejection_413() {
        let dir =
            std::env::temp_dir().join(format!("zniff-100-continue-413-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = origin.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let mut total_read = 0;
            while !buf[..total_read].windows(4).any(|w| w == b"\r\n\r\n") {
                let n = sock.read(&mut buf[total_read..]).await.unwrap();
                if n == 0 {
                    break;
                }
                total_read += n;
            }
            let head = String::from_utf8_lossy(&buf[..total_read]);
            assert!(head.to_ascii_lowercase().contains("expect: 100-continue"));
            assert!(
                head.to_ascii_lowercase()
                    .contains("content-length: 10485760")
            );

            // Origin rejects immediately with 413 Payload Too Large without sending 100 Continue
            let body = b"{\"error\":\"payload_too_large\"}";
            let resp = format!(
                "HTTP/1.1 413 Payload Too Large\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
        });
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        // Client declares a 10MB payload with Expect: 100-continue
        let req = format!(
            "POST http://{origin_addr}/upload HTTP/1.1\r\nHost: {origin_addr}\r\nExpect: 100-continue\r\nContent-Length: 10485760\r\nConnection: close\r\n\r\n"
        );
        client.write_all(req.as_bytes()).await.unwrap();
        // Under RFC 9110 §10.1.1, client does NOT send body bytes when awaiting 100-continue.
        // If zniff blocks waiting for 10MB body, client read_to_end will time out (deadlock!).
        let mut resp = Vec::new();
        let _ = timeout(Duration::from_secs(3), client.read_to_end(&mut resp))
            .await
            .expect("Client timed out waiting for 413 rejection (deadlock detected: proxy blocked waiting for body!)")
            .unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(
            text.contains("HTTP/1.1 413"),
            "Client must receive 413 response: {text}"
        );
        assert!(
            text.contains("payload_too_large"),
            "Client must receive 413 body: {text}"
        );

        let flow = hub
            .with_session(|session| session.get(1).cloned())
            .expect("flow");
        assert_eq!(flow.status, Some(413));
        assert!(
            flow.request_body.is_empty(),
            "No request body should be recorded since client never sent one"
        );
        assert_eq!(flow.response_body, b"{\"error\":\"payload_too_large\"}");
        assert!(
            flow.elapsed_ms().is_some(),
            "Flow must be marked finished with elapsed time"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn plain_http_expect_100_continue_with_early_hints_103_then_continue() {
        let dir = std::env::temp_dir().join(format!("zniff-103-then-100-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        let req_body = b"multi-interim-payload";
        tokio::spawn(async move {
            let (mut sock, _) = origin.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let mut total_read = 0;
            while !buf[..total_read].windows(4).any(|w| w == b"\r\n\r\n") {
                let n = sock.read(&mut buf[total_read..]).await.unwrap();
                if n == 0 {
                    break;
                }
                total_read += n;
            }
            // 1. Origin sends 103 Early Hints
            sock.write_all(b"HTTP/1.1 103 Early Hints\r\nLink: </app.js>; rel=preload\r\n\r\n")
                .await
                .unwrap();
            // 2. Origin sends 100 Continue
            sock.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
                .await
                .unwrap();
            // 3. Origin reads body
            let mut body_buf = Vec::new();
            while body_buf.len() < req_body.len() {
                let n = sock.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                body_buf.extend_from_slice(&buf[..n]);
            }
            assert_eq!(&body_buf[..req_body.len()], req_body);
            // 4. Origin sends 200 OK
            let body = b"{\"done\":true}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
        });
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "POST http://{origin_addr}/multi HTTP/1.1\r\nHost: {origin_addr}\r\nExpect: 100-continue\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            req_body.len()
        );
        client.write_all(req.as_bytes()).await.unwrap();

        // Client reads 103 Early Hints first
        let mut byte = [0u8; 1];
        let mut interim_103 = Vec::new();
        while !interim_103.ends_with(b"\r\n\r\n") {
            let n = timeout(Duration::from_secs(3), client.read(&mut byte))
                .await
                .expect("Client timed out waiting for 103 Early Hints")
                .unwrap();
            assert_ne!(
                n, 0,
                "Proxy closed connection before sending 103 Early Hints"
            );
            interim_103.push(byte[0]);
        }
        let text_103 = String::from_utf8_lossy(&interim_103);
        assert!(
            text_103.starts_with("HTTP/1.1 103 Early Hints"),
            "{text_103}"
        );

        // Client reads 100 Continue second
        let mut interim_100 = Vec::new();
        while !interim_100.ends_with(b"\r\n\r\n") {
            let n = timeout(Duration::from_secs(3), client.read(&mut byte))
                .await
                .expect("Client timed out waiting for 100 Continue")
                .unwrap();
            assert_ne!(n, 0, "Proxy closed connection before sending 100 Continue");
            interim_100.push(byte[0]);
        }
        let text_100 = String::from_utf8_lossy(&interim_100);
        assert!(text_100.starts_with("HTTP/1.1 100 Continue"), "{text_100}");

        // Now client writes body
        client.write_all(req_body).await.unwrap();

        // Client reads final 200 OK
        let mut resp = Vec::new();
        let _ = timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
            .await
            .expect("Client timed out waiting for 200 OK")
            .unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("HTTP/1.1 200 OK"), "{text}");
        assert!(text.contains("{\"done\":true}"), "{text}");

        let flow = hub
            .with_session(|session| session.get(1).cloned())
            .expect("flow");
        assert_eq!(flow.status, Some(200));
        assert_eq!(flow.request_body, req_body);
        assert_eq!(flow.response_body, b"{\"done\":true}");
        assert!(flow.elapsed_ms().is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_adversarial_100_continue_client_waits_for_continue() {
        let dir =
            std::env::temp_dir().join(format!("zniff-100-continue-adv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = origin.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let mut total_read = 0;
            while !buf[..total_read].windows(4).any(|w| w == b"\r\n\r\n") {
                let n = sock.read(&mut buf[total_read..]).await.unwrap();
                if n == 0 {
                    break;
                }
                total_read += n;
            }
            sock.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
                .await
                .unwrap();
            let mut body = [0u8; 5];
            sock.read_exact(&mut body).await.unwrap();
            assert_eq!(&body, b"hello");
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        });
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "POST http://{origin_addr}/adv HTTP/1.1\r\nHost: {origin_addr}\r\nExpect: 100-continue\r\nContent-Length: 5\r\nConnection: close\r\n\r\n"
        );
        client.write_all(req.as_bytes()).await.unwrap();
        let mut interim_buf = [0u8; 1024];
        let res = timeout(Duration::from_millis(2000), client.read(&mut interim_buf)).await;
        assert!(
            res.is_ok(),
            "DEADLOCK: Client timed out waiting for 100 Continue!"
        );
        let n = res.unwrap().unwrap();
        assert!(String::from_utf8_lossy(&interim_buf[..n]).contains("100 Continue"));
        client.write_all(b"hello").await.unwrap();
        let mut final_resp = Vec::new();
        client.read_to_end(&mut final_resp).await.unwrap();
        assert!(String::from_utf8_lossy(&final_resp).contains("200 OK"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn plain_http_early_hints_103_records_final_response() {
        let dir = std::env::temp_dir().join(format!("zniff-103-hints-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = origin.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = sock.read(&mut buf).await.unwrap();
            // Origin sends 103 Early Hints
            sock.write_all(b"HTTP/1.1 103 Early Hints\r\nLink: </style.css>; rel=preload\r\n\r\n")
                .await
                .unwrap();
            // Origin sends final 200 OK response
            let body = b"main page content";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
        });
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "GET http://{origin_addr}/page HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
        );
        client.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        let _ = timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(
            text.contains("103 Early Hints"),
            "client must receive 103 Early Hints"
        );
        assert!(text.contains("200 OK"), "client must receive 200 OK");
        assert!(
            text.contains("main page content"),
            "client must receive body"
        );

        let flow = hub.with_session(|session| session.get(1).cloned());
        let flow = flow.expect("flow");
        assert_eq!(flow.status, Some(200));
        assert_eq!(flow.response_body, b"main page content");
        assert!(flow.elapsed_ms().is_some(), "flow must be marked finished");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_page_follows_accept_language() {
        assert!(super::install_page_for(None).contains("安装 zniff"));
        assert!(super::install_page_for(Some("")).contains("安装 zniff"));
        assert!(super::install_page_for(Some("zh-CN,en;q=0.8")).contains("安装 zniff"));
        assert!(super::install_page_for(Some("en-US,en;q=0.9")).contains("Install the zniff"));
        assert!(super::install_page_for(Some("fr")).contains("Install the zniff"));
    }

    use super::*;

    #[tokio::test]
    async fn plain_http_is_recorded_and_forwarded() {
        let dir = std::env::temp_dir().join(format!("snoop-proxy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = origin.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = sock.read(&mut buf).await.unwrap();
            let body = b"hello-snoop";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
        });
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "GET http://{origin_addr}/hi HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
        );
        client.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        let _ = timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("hello-snoop"), "{text}");
        let flow = hub.with_session(|session| session.get(1).cloned());
        let flow = flow.expect("flow");
        assert_eq!(flow.status, Some(200));
        assert_eq!(flow.response_body, b"hello-snoop");
        assert_eq!(flow.protocol, Protocol::Http11);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn replace_rewrites_a_plain_response_body() {
        let dir = std::env::temp_dir().join(format!("snoop-replace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        hub.set_rules("replace 127.0.0.1 res hello -> hi there");
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = origin.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = sock.read(&mut buf).await.unwrap();
            let body = b"say hello";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
        });
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "GET http://{origin_addr}/hi HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
        );
        client.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        let _ = timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("say hi there"), "{text}");
        assert!(!text.contains("say hello"), "{text}");
        let flow = hub.with_session(|session| session.get(1).cloned()).unwrap();
        assert_eq!(flow.response_body, b"say hi there");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn https_follows_origin_alpn_and_records_the_body() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("snoop-mitm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let mut hub = Hub::new(ca.clone()).unwrap();

        let origin_ca_key = rcgen::KeyPair::generate().unwrap();
        let mut origin_ca_params = rcgen::CertificateParams::default();
        origin_ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let mut origin_name = rcgen::DistinguishedName::new();
        origin_name.push(rcgen::DnType::CommonName, "origin-test");
        origin_ca_params.distinguished_name = origin_name;
        origin_ca_params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::DigitalSignature,
        ];
        let origin_ca = origin_ca_params.self_signed(&origin_ca_key).unwrap();
        let issuer = rcgen::Issuer::from_params(&origin_ca_params, &origin_ca_key);
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let mut leaf_params = rcgen::CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
        leaf_params.is_ca = rcgen::IsCa::NoCa;
        leaf_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        leaf_params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        let origin_leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        let hub = Arc::new(hub);

        let mut server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![origin_leaf.der().clone(), origin_ca.der().clone()],
                rustls::pki_types::PrivateKeyDer::Pkcs8(
                    rustls::pki_types::PrivatePkcs8KeyDer::from(leaf_key.serialize_der()),
                ),
            )
            .unwrap();
        server_config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (sock, _) = origin.accept().await.unwrap();
            let mut tls = acceptor.accept(sock).await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = tls.read(&mut buf).await.unwrap();
            let body = b"mitm-ok";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
                body.len()
            );
            tls.write_all(resp.as_bytes()).await.unwrap();
            tls.write_all(body).await.unwrap();
        });

        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_conn(socket_from(proxy).await, hub_serve).await {
                log::debug!("测试连接: {err:#}");
            }
        });

        let mut tcp = TcpStream::connect(proxy_addr).await.unwrap();
        let connect = format!("CONNECT {origin_addr} HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n");
        tcp.write_all(connect.as_bytes()).await.unwrap();
        let mut header = Vec::new();
        let mut byte = [0u8; 1];
        while !header.ends_with(b"\r\n\r\n") {
            let n = timeout(Duration::from_secs(5), tcp.read(&mut byte))
                .await
                .unwrap()
                .unwrap();
            assert_ne!(n, 0, "代理没有回 CONNECT 响应");
            header.push(byte[0]);
            assert!(header.len() < 1024, "CONNECT 响应过长");
        }
        let header_text = String::from_utf8_lossy(&header);
        assert!(header_text.starts_with("HTTP/1.1 200"), "{header_text}");

        let mut roots = RootCertStore::empty();
        roots.add(ca.cert_der()).unwrap();
        let mut client_config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        client_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        let connector = TlsConnector::from(Arc::new(client_config));
        let mut tls = timeout(
            Duration::from_secs(5),
            connector.connect(ServerName::IpAddress(origin_addr.ip().into()), tcp),
        )
        .await
        .expect("客户端 TLS 超时")
        .expect("客户端 TLS");
        assert_eq!(
            tls.get_ref().1.alpn_protocol(),
            Some(b"http/1.1".as_slice())
        );

        let req = format!(
            "GET /hi HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            origin_addr.ip()
        );
        tls.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        timeout(Duration::from_secs(5), tls.read_to_end(&mut resp))
            .await
            .expect("读响应超时")
            .unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("mitm-ok"), "{text}");

        let flow = hub
            .with_session(|session| session.get(1).cloned())
            .expect("flow");
        assert_eq!(flow.status, Some(200));
        assert_eq!(flow.response_body, b"mitm-ok");
        assert_eq!(flow.protocol, Protocol::Http11);
        assert_eq!(flow.method, "GET");
        assert!(flow.url.contains("/hi"), "{}", flow.url);
        let _ = std::fs::remove_dir_all(&dir);
    }

    async fn socket_from(listener: TcpListener) -> TcpStream {
        listener.accept().await.unwrap().0
    }

    #[tokio::test]
    async fn https_follows_h2_when_the_origin_selects_it() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("snoop-h2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        let hub = Arc::new(hub);

        let mut server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![origin_leaf.der().clone(), origin_ca.der().clone()],
                rustls::pki_types::PrivateKeyDer::Pkcs8(
                    rustls::pki_types::PrivatePkcs8KeyDer::from(leaf_key.serialize_der()),
                ),
            )
            .unwrap();
        server_config.alpn_protocols = vec![b"h2".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (sock, _) = origin.accept().await.unwrap();
            let tls = acceptor.accept(sock).await.unwrap();
            let service = hyper::service::service_fn(|_req: Request<Incoming>| async move {
                Ok::<_, std::convert::Infallible>(Response::new(http_body_util::Full::new(
                    Bytes::from_static(b"h2-ok"),
                )))
            });
            let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(tls), service)
                .await;
        });

        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_conn(socket_from(proxy).await, hub_serve).await {
                log::debug!("测试连接: {err:#}");
            }
        });

        let mut tcp = TcpStream::connect(proxy_addr).await.unwrap();
        let connect = format!("CONNECT {origin_addr} HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n");
        tcp.write_all(connect.as_bytes()).await.unwrap();
        read_http_head(&mut tcp).await;

        let mut roots = RootCertStore::empty();
        roots.add(ca.cert_der()).unwrap();
        let mut client_config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        client_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        let tls = timeout(
            Duration::from_secs(5),
            TlsConnector::from(Arc::new(client_config))
                .connect(ServerName::IpAddress(origin_addr.ip().into()), tcp),
        )
        .await
        .expect("客户端 TLS 超时")
        .expect("客户端 TLS");
        assert_eq!(tls.get_ref().1.alpn_protocol(), Some(b"h2".as_slice()));

        let (mut sender, connection) =
            hyper::client::conn::http2::Builder::new(TokioExecutor::new())
                .handshake::<_, http_body_util::Full<Bytes>>(TokioIo::new(tls))
                .await
                .expect("客户端 HTTP/2");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let req = Request::builder()
            .uri("/hi")
            .body(http_body_util::Full::new(Bytes::new()))
            .unwrap();
        let response = timeout(Duration::from_secs(5), sender.send_request(req))
            .await
            .expect("HTTP/2 响应超时")
            .expect("HTTP/2 响应");
        assert_eq!(response.status(), StatusCode::OK);
        let body = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        assert_eq!(&body[..], b"h2-ok");

        let flow = hub
            .with_session(|session| session.get(1).cloned())
            .expect("flow");
        assert_eq!(flow.protocol, Protocol::Http2);
        assert_eq!(flow.method, "GET");
        assert_eq!(flow.status, Some(200));
        assert_eq!(flow.response_body, b"h2-ok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn test_origin_cert() -> (rcgen::Certificate, rcgen::Certificate, rcgen::KeyPair) {
        let origin_ca_key = rcgen::KeyPair::generate().unwrap();
        let mut origin_ca_params = rcgen::CertificateParams::default();
        origin_ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let mut origin_name = rcgen::DistinguishedName::new();
        origin_name.push(rcgen::DnType::CommonName, "origin-test");
        origin_ca_params.distinguished_name = origin_name;
        origin_ca_params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::DigitalSignature,
        ];
        let origin_ca = origin_ca_params.self_signed(&origin_ca_key).unwrap();
        let issuer = rcgen::Issuer::from_params(&origin_ca_params, &origin_ca_key);
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let mut leaf_params = rcgen::CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
        leaf_params.is_ca = rcgen::IsCa::NoCa;
        leaf_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        leaf_params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        let origin_leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
        (origin_ca, origin_leaf, leaf_key)
    }

    async fn read_http_head(tcp: &mut TcpStream) {
        let mut header = Vec::new();
        let mut byte = [0u8; 1];
        while !header.ends_with(b"\r\n\r\n") {
            let n = timeout(Duration::from_secs(5), tcp.read(&mut byte))
                .await
                .unwrap()
                .unwrap();
            assert_ne!(n, 0, "代理没有回 CONNECT 响应");
            header.push(byte[0]);
            assert!(header.len() < 1024, "CONNECT 响应过长");
        }
        let header_text = String::from_utf8_lossy(&header);
        assert!(header_text.starts_with("HTTP/1.1 200"), "{header_text}");
    }

    #[tokio::test]
    async fn websocket_handshake_is_recorded_and_frames_are_not() {
        let dir = std::env::temp_dir().join(format!("snoop-ws-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = origin.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap();
            let text = String::from_utf8_lossy(&buf[..n]);
            assert!(text.contains("Upgrade: websocket"), "{text}");
            let resp = "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n";
            sock.write_all(resp.as_bytes()).await.unwrap();
            let mut frame = [0u8; 4];
            sock.read_exact(&mut frame).await.unwrap();
            assert_eq!(&frame, b"ping");
            sock.write_all(b"pong").await.unwrap();
        });
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "GET http://{origin_addr}/chat HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        client.write_all(req.as_bytes()).await.unwrap();
        let mut header = Vec::new();
        let mut byte = [0u8; 1];
        while !header.ends_with(b"\r\n\r\n") {
            let n = timeout(Duration::from_secs(5), client.read(&mut byte))
                .await
                .unwrap()
                .unwrap();
            assert_ne!(n, 0);
            header.push(byte[0]);
        }
        let header_text = String::from_utf8_lossy(&header);
        assert!(header_text.contains("101"), "{header_text}");
        client.write_all(b"ping").await.unwrap();
        let mut pong = [0u8; 4];
        timeout(Duration::from_secs(5), client.read_exact(&mut pong))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&pong, b"pong");
        let flow = hub
            .with_session(|session| session.get(1).cloned())
            .expect("flow");
        assert_eq!(flow.protocol, Protocol::WebSocket);
        assert_eq!(flow.status, Some(101));
        assert!(flow.response_body.is_empty(), "{:?}", flow.response_body);
        assert!(flow.request_body.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn plaintext_keep_alive_reuses_the_origin_socket() {
        let dir = std::env::temp_dir().join(format!("zniff-pool-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let accepts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        let accepts_bg = accepts.clone();
        tokio::spawn(async move {
            let (mut sock, _) = origin.accept().await.unwrap();
            accepts_bg.fetch_add(1, Ordering::SeqCst);
            for mark in [b"one", b"two"] {
                let mut buf = Vec::new();
                let mut byte = [0u8; 1];
                while !buf.ends_with(b"\r\n\r\n") {
                    let n = timeout(Duration::from_secs(5), sock.read(&mut byte))
                        .await
                        .unwrap()
                        .unwrap();
                    assert_ne!(n, 0);
                    buf.push(byte[0]);
                }
                let resp = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", mark.len());
                sock.write_all(resp.as_bytes()).await.unwrap();
                sock.write_all(mark).await.unwrap();
            }
        });
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = proxy.accept().await else {
                    break;
                };
                let hub = hub_serve.clone();
                tokio::spawn(async move {
                    let _ = handle_conn(socket, hub).await;
                });
            }
        });
        for mark in ["one", "two"] {
            let mut client = TcpStream::connect(proxy_addr).await.unwrap();
            let req = format!("GET http://{origin_addr}/x HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n");
            client.write_all(req.as_bytes()).await.unwrap();
            let mut resp = Vec::new();
            timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
                .await
                .unwrap()
                .unwrap();
            let text = String::from_utf8_lossy(&resp);
            assert!(text.contains(mark), "{text}");
        }
        assert_eq!(accepts.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn upstream_proxy_receives_absolute_form() {
        let dir = std::env::temp_dir().join(format!("zniff-up-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = upstream.accept().await.unwrap();
            let mut buf = vec![0u8; 2048];
            let n = timeout(Duration::from_secs(5), sock.read(&mut buf))
                .await
                .unwrap()
                .unwrap();
            let text = String::from_utf8_lossy(&buf[..n]);
            assert!(text.starts_with("GET http://example.test/hi "), "{text}");
            let body = b"via-upstream";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
        });
        hub.set_upstream(&format!("127.0.0.1:{}", upstream_addr.port()));
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        let req = "GET http://example.test/hi HTTP/1.1\r\nHost: example.test\r\nConnection: close\r\n\r\n";
        client.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
            .await
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&resp).contains("via-upstream"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn bypass_tunnels_tls_without_a_leaf() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-bypass-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        hub.set_bypass("127.0.0.1");
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![origin_leaf.der().clone(), origin_ca.der().clone()],
                rustls::pki_types::PrivateKeyDer::Pkcs8(
                    rustls::pki_types::PrivatePkcs8KeyDer::from(leaf_key.serialize_der()),
                ),
            )
            .unwrap();
        server_config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (sock, _) = origin.accept().await.unwrap();
            let mut tls = acceptor.accept(sock).await.unwrap();
            let mut buf = vec![0u8; 1024];
            let _ = tls.read(&mut buf).await.unwrap();
            let body = b"tunnel-ok";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            tls.write_all(resp.as_bytes()).await.unwrap();
            tls.write_all(body).await.unwrap();
            tls.shutdown().await.unwrap();
        });
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });
        let mut tcp = TcpStream::connect(proxy_addr).await.unwrap();
        let connect = format!("CONNECT {origin_addr} HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n");
        tcp.write_all(connect.as_bytes()).await.unwrap();
        read_http_head(&mut tcp).await;
        let mut roots = RootCertStore::empty();
        roots.add(origin_ca.der().clone()).unwrap();
        let client_config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(client_config));
        let mut tls = timeout(
            Duration::from_secs(5),
            connector.connect(ServerName::IpAddress(origin_addr.ip().into()), tcp),
        )
        .await
        .expect("客户端 TLS 超时")
        .expect("直通后应使用源站证书");
        tls.write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut resp = Vec::new();
        let read = timeout(Duration::from_secs(5), tls.read_to_end(&mut resp))
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("tunnel-ok"), "{read:?} {text}");
        let flow = hub
            .with_session(|session| session.get(1).cloned())
            .expect("flow");
        assert_eq!(flow.protocol, Protocol::Tunnel);
        let reason = flow.error.unwrap_or_default();
        assert!(reason.contains("直通"), "{reason}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn websocket_text_frame_is_recorded_beside_the_handshake() {
        let dir = std::env::temp_dir().join(format!("zniff-ws-frame-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = origin.accept().await.unwrap();
            let mut buf = vec![0u8; 1024];
            let _ = sock.read(&mut buf).await.unwrap();
            sock.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n")
                .await
                .unwrap();
            sock.write_all(&[0x81, 0x02, b'h', b'i']).await.unwrap();
        });
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "GET http://{origin_addr}/chat HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n"
        );
        client.write_all(req.as_bytes()).await.unwrap();
        let mut header = Vec::new();
        let mut byte = [0u8; 1];
        while !header.ends_with(b"\r\n\r\n") {
            let n = timeout(Duration::from_secs(5), client.read(&mut byte))
                .await
                .unwrap()
                .unwrap();
            assert_ne!(n, 0);
            header.push(byte[0]);
        }
        assert!(String::from_utf8_lossy(&header).contains("101"));
        let mut frame = [0u8; 4];
        timeout(Duration::from_secs(5), client.read_exact(&mut frame))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&frame, &[0x81, 0x02, b'h', b'i']);
        let flow = hub
            .with_session(|session| session.get(1).cloned())
            .expect("flow");
        assert_eq!(flow.protocol, Protocol::WebSocket);
        assert_eq!(flow.messages.len(), 1);
        assert_eq!(flow.messages[0].kind, "文本");
        assert_eq!(flow.messages[0].text, "hi");
        assert!(flow.response_body.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn wss_websocket_echoes_and_records_frames() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-wss-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        let hub = Arc::new(hub);

        let mut server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![origin_leaf.der().clone(), origin_ca.der().clone()],
                rustls::pki_types::PrivateKeyDer::Pkcs8(
                    rustls::pki_types::PrivatePkcs8KeyDer::from(leaf_key.serialize_der()),
                ),
            )
            .unwrap();
        server_config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (sock, _) = origin.accept().await.unwrap();
            let mut tls = acceptor.accept(sock).await.unwrap();
            let mut req_head = Vec::new();
            let mut byte = [0u8; 1];
            while !req_head.ends_with(b"\r\n\r\n") {
                let n = tls.read(&mut byte).await.unwrap();
                assert_ne!(n, 0);
                req_head.push(byte[0]);
            }
            tls.write_all(
                b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n",
            )
            .await
            .unwrap();
            // Masked client frame: "ok" with mask [1, 2, 3, 4]
            let mut client_frame = [0u8; 8];
            tls.read_exact(&mut client_frame).await.unwrap();
            let mask = [
                client_frame[2],
                client_frame[3],
                client_frame[4],
                client_frame[5],
            ];
            let payload = [client_frame[6] ^ mask[0], client_frame[7] ^ mask[1]];
            assert_eq!(&payload, b"ok");
            // Unmasked server echo frame: "ok"
            tls.write_all(&[0x81, 0x02, payload[0], payload[1]])
                .await
                .unwrap();
            let _ = tls.shutdown().await;
        });

        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });

        let mut tcp = TcpStream::connect(proxy_addr).await.unwrap();
        let connect = format!("CONNECT {origin_addr} HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n");
        tcp.write_all(connect.as_bytes()).await.unwrap();
        read_http_head(&mut tcp).await;

        let mut roots = RootCertStore::empty();
        roots.add(ca.cert_der()).unwrap();
        let mut client_config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        client_config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let mut tls = timeout(
            Duration::from_secs(5),
            TlsConnector::from(Arc::new(client_config))
                .connect(ServerName::IpAddress(origin_addr.ip().into()), tcp),
        )
        .await
        .expect("客户端 TLS 超时")
        .expect("客户端 TLS");

        let req = format!(
            "GET /ws HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n"
        );
        tls.write_all(req.as_bytes()).await.unwrap();
        let mut header = Vec::new();
        let mut byte = [0u8; 1];
        while !header.ends_with(b"\r\n\r\n") {
            let n = timeout(Duration::from_secs(5), tls.read(&mut byte))
                .await
                .unwrap()
                .unwrap();
            assert_ne!(n, 0);
            header.push(byte[0]);
        }
        let head = String::from_utf8_lossy(&header);
        assert!(head.contains("101"), "{head}");

        // Send masked text frame "ok" with mask [1, 2, 3, 4]
        let masked = [0x81, 0x82, 1, 2, 3, 4, b'o' ^ 1, b'k' ^ 2];
        tls.write_all(&masked).await.unwrap();

        let mut frame = [0u8; 4];
        timeout(Duration::from_secs(5), tls.read_exact(&mut frame))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&frame, &[0x81, 0x02, b'o', b'k']);
        drop(tls);

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let done = hub.with_session(|session| {
                session
                    .get(1)
                    .is_some_and(|flow| flow.messages.len() == 2 && flow.finished_at.is_some())
            });
            if done || std::time::Instant::now() > deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let flow = hub
            .with_session(|session| session.get(1).cloned())
            .expect("flow");
        assert_eq!(flow.protocol, Protocol::WebSocket);
        assert_eq!(flow.messages.len(), 2);
        assert_eq!(flow.messages[0].text, "ok");
        assert_eq!(flow.messages[1].text, "ok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn map_local_answers_without_dialing() {
        let dir = std::env::temp_dir().join(format!("zniff-map-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let file = dir.join("body.txt");
        std::fs::write(&file, b"from-file").unwrap();
        hub.set_rules(&format!("map-local 127.0.0.1/hi {}", file.display()));
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(b"GET http://127.0.0.1:9/hi HTTP/1.1\r\nHost: 127.0.0.1:9\r\n\r\n")
            .await
            .unwrap();
        let mut resp = Vec::new();
        timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("200"), "{text}");
        assert!(text.contains("from-file"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn map_local_status_and_cors_preflight() {
        let dir = std::env::temp_dir().join(format!("zniff-mock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let file = dir.join("body.txt");
        std::fs::write(&file, b"nope").unwrap();
        hub.set_rules(&format!(
            "map-local 127.0.0.1/hi {} status=429\ncors 127.0.0.1\nheader 127.0.0.1 res set X-Zniff yes\n",
            file.display()
        ));
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(b"GET http://127.0.0.1:9/hi HTTP/1.1\r\nHost: 127.0.0.1:9\r\n\r\n")
            .await
            .unwrap();
        let mut resp = Vec::new();
        timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("429"), "{text}");
        assert!(text.to_ascii_lowercase().contains("x-zniff: yes"), "{text}");
        assert!(
            text.to_ascii_lowercase()
                .contains("access-control-allow-origin: *"),
            "{text}"
        );

        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(b"OPTIONS http://127.0.0.1:9/other HTTP/1.1\r\nHost: 127.0.0.1:9\r\n\r\n")
            .await
            .unwrap();
        let mut resp = Vec::new();
        timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("204"), "{text}");
        assert!(
            text.to_ascii_lowercase()
                .contains("access-control-allow-origin: *"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn response_break_can_replace_status_and_body() {
        let dir = std::env::temp_dir().join(format!("zniff-break-res-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = origin.accept().await.unwrap();
            let mut buf = vec![0u8; 2048];
            let _ = sock.read(&mut buf).await.unwrap();
            let body = b"orig";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
        });
        hub.set_rules("break-res 127.0.0.1/hi");
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });
        let hub_wait = hub.clone();
        let waiter = tokio::spawn(async move {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                if hub_wait.paused_id() == Some(1) {
                    assert_eq!(
                        hub_wait.paused().map(|(_, side)| side),
                        Some(BreakSide::Response)
                    );
                    hub_wait.resolve_break(BreakDecision {
                        drop: false,
                        headers: None,
                        body: Some(b"edited".to_vec()),
                        status: Some(201),
                    });
                    return;
                }
                if std::time::Instant::now() > deadline {
                    panic!("响应断点没有暂停");
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "GET http://{origin_addr}/hi HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
        );
        client.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
            .await
            .unwrap()
            .unwrap();
        waiter.await.unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("201"), "{text}");
        assert!(text.contains("edited"), "{text}");
        let flow = hub
            .with_session(|session| session.get(1).cloned())
            .expect("flow");
        assert_eq!(flow.status, Some(201));
        assert_eq!(flow.response_body, b"edited");
        assert!(flow.finished_at.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn https_response_break_can_replace_status_and_marks_finished() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir =
            std::env::temp_dir().join(format!("zniff-https-break-res-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        let hub = Arc::new(hub);

        let mut server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![origin_leaf.der().clone(), origin_ca.der().clone()],
                rustls::pki_types::PrivateKeyDer::Pkcs8(
                    rustls::pki_types::PrivatePkcs8KeyDer::from(leaf_key.serialize_der()),
                ),
            )
            .unwrap();
        server_config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (sock, _) = origin.accept().await.unwrap();
            let mut tls = acceptor.accept(sock).await.unwrap();
            let mut buf = vec![0u8; 1024];
            let _ = tls.read(&mut buf).await.unwrap();
            tls.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\norig")
                .await
                .unwrap();
            let _ = tls.shutdown().await;
        });

        hub.set_rules("break-res 127.0.0.1/hi");
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });

        let hub_wait = hub.clone();
        let waiter = tokio::spawn(async move {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                if hub_wait.paused_id() == Some(1) {
                    hub_wait.touch_break();
                    assert_eq!(
                        hub_wait.paused().map(|(_, side)| side),
                        Some(BreakSide::Response)
                    );
                    hub_wait.resolve_break(BreakDecision {
                        drop: false,
                        headers: None,
                        body: None,
                        status: Some(201),
                    });
                    return;
                }
                if std::time::Instant::now() > deadline {
                    panic!("HTTPS 响应断点没有暂停");
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });

        let mut tcp = TcpStream::connect(proxy_addr).await.unwrap();
        let connect = format!("CONNECT {origin_addr} HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n");
        tcp.write_all(connect.as_bytes()).await.unwrap();
        read_http_head(&mut tcp).await;

        let mut roots = RootCertStore::empty();
        roots.add(ca.cert_der()).unwrap();
        let mut client_config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        client_config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let mut tls = timeout(
            Duration::from_secs(5),
            TlsConnector::from(Arc::new(client_config))
                .connect(ServerName::IpAddress(origin_addr.ip().into()), tcp),
        )
        .await
        .expect("客户端 TLS 超时")
        .expect("客户端 TLS");

        let req = format!(
            "GET /hi HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            origin_addr.ip()
        );
        tls.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        timeout(Duration::from_secs(5), tls.read_to_end(&mut resp))
            .await
            .unwrap()
            .unwrap();
        waiter.await.unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("201"), "{text}");
        assert!(text.contains("orig"), "{text}");
        let flow = hub
            .with_session(|session| session.get(1).cloned())
            .expect("flow");
        assert_eq!(flow.status, Some(201));
        assert_eq!(flow.response_body, b"orig");
        assert!(flow.finished_at.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn breakpoint_drop_closes_client_connection_and_marks_failed() {
        let dir = std::env::temp_dir().join(format!("zniff-break-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = origin.accept().await {
                let _ = read_header_block(&mut sock).await;
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}",
                    )
                    .await;
            }
        });
        hub.set_rules("break 127.0.0.1/hold\nbreak-res 127.0.0.1/hold-res");
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());

        // 1. Request breakpoint drop
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(b"GET http://127.0.0.1:9/hold HTTP/1.1\r\nHost: 127.0.0.1:9\r\n\r\n")
            .await
            .unwrap();
        let hub_wait = hub.clone();
        let waiter = tokio::spawn(async move {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                if hub_wait.paused_id() == Some(1) {
                    hub_wait.resolve_break(BreakDecision {
                        drop: true,
                        headers: None,
                        body: None,
                        status: None,
                    });
                    return;
                }
                if std::time::Instant::now() > deadline {
                    panic!("断点没有暂停");
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let mut resp = Vec::new();
        let _ = timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
            .await
            .unwrap();
        waiter.await.unwrap();
        assert!(
            resp.is_empty(),
            "dropped request breakpoint must not write HTTP 204: {:?}",
            String::from_utf8_lossy(&resp)
        );

        // 2. Response breakpoint drop (origin returned 200 OK, drop clears status -> None)
        let mut client2 = TcpStream::connect(proxy_addr).await.unwrap();
        client2
            .write_all(
                format!(
                    "GET http://{origin_addr}/hold-res HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let hub_wait2 = hub.clone();
        let waiter2 = tokio::spawn(async move {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                if hub_wait2.paused_id() == Some(2) {
                    hub_wait2.resolve_break(BreakDecision {
                        drop: true,
                        headers: None,
                        body: None,
                        status: None,
                    });
                    return;
                }
                if std::time::Instant::now() > deadline {
                    panic!("响应断点没有暂停");
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let mut resp2 = Vec::new();
        let _ = timeout(Duration::from_secs(5), client2.read_to_end(&mut resp2))
            .await
            .unwrap();
        waiter2.await.unwrap();
        assert!(
            resp2.is_empty(),
            "dropped response breakpoint must not write HTTP 204: {:?}",
            String::from_utf8_lossy(&resp2)
        );

        hub.with_session(|session| {
            for fid in [1u64, 2u64] {
                let flow = session.get(fid).expect("flow");
                assert_eq!(flow.status, None);
                assert_eq!(flow.error.as_deref(), Some("断点丢弃"));
                assert!(flow.finished_at.is_some());
            }
            assert_eq!(session.ids_matching("失败"), vec![1, 2]);
            let stats = session.filtered_stats("");
            assert_eq!(stats.s2xx, 0);
            assert_eq!(stats.failed, 2);
            let facets = session.facets();
            assert_eq!(facets.domains[0].error_count, 2);
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_routes_leave_proxied_roots_alone() {
        assert!(matches!(
            local_route("GET", "/zniff"),
            Some(LocalRoute::Install)
        ));
        assert!(matches!(local_route("GET", "/"), Some(LocalRoute::Install)));
        assert!(matches!(
            local_route("GET", "http://192.168.1.8:8877/zniff"),
            Some(LocalRoute::Install)
        ));
        assert!(local_route("GET", "http://example.com/").is_none());
        assert!(local_route("GET", "http://example.com/index").is_none());
        let id = "550e8400-e29b-41d4-a716-446655440000";
        assert!(matches!(
            local_route("GET", &format!("/s/{id}")),
            Some(LocalRoute::Share(_, crate::share::SharePath::Page))
        ));
        assert!(local_route("GET", &format!("http://example.com/s/{id}")).is_none());
        assert!(matches!(
            local_route("GET", "http://127.0.0.1/zniff-ca.crt"),
            Some(LocalRoute::Cert)
        ));
        assert!(matches!(
            local_route("GET", "/favicon.ico"),
            Some(LocalRoute::Favicon)
        ));
        assert!(local_route("GET", "http://example.com/favicon.ico").is_none());
        assert!(matches!(
            local_route_for("GET", "http://127.0.0.1:8877/favicon.ico", true),
            Some(LocalRoute::Favicon)
        ));
        assert!(matches!(
            local_route_for("GET", &format!("http://127.0.0.1:8877/s/{id}"), true),
            Some(LocalRoute::Share(_, crate::share::SharePath::Page))
        ));
        assert!(local_route("POST", "/zniff").is_none());
        assert!(local_route("CONNECT", "example.com:443").is_none());
        assert!(local_route("PUT", &format!("http://example.com/s/{id}/flows.json")).is_none());
        assert!(matches!(
            local_route("PUT", &format!("/s/{id}/flows.json")),
            Some(LocalRoute::Relay(
                _,
                crate::share::SharePath::Data { .. },
                RelayVerb::Put
            ))
        ));
        assert!(matches!(
            local_route("DELETE", &format!("/s/{id}")),
            Some(LocalRoute::Relay(
                _,
                crate::share::SharePath::Page,
                RelayVerb::Delete
            ))
        ));
        assert!(
            local_route_for(
                "PUT",
                &format!("http://example.com/s/{id}/flows.json"),
                false
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn favicon_and_self_targeted_requests_do_not_loop_or_record() {
        let dir = std::env::temp_dir().join(format!("zniff-self-loop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        let req =
            format!("GET /favicon.ico HTTP/1.1\r\nHost: {proxy_addr}\r\nConnection: close\r\n\r\n");
        client.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.starts_with("HTTP/1.1 204"), "{text}");

        let mut client2 = TcpStream::connect(proxy_addr).await.unwrap();
        let req2 = format!(
            "GET http://{proxy_addr}/unknown-local-path HTTP/1.1\r\nHost: {proxy_addr}\r\nConnection: close\r\n\r\n"
        );
        client2.write_all(req2.as_bytes()).await.unwrap();
        let mut resp2 = Vec::new();
        timeout(Duration::from_secs(5), client2.read_to_end(&mut resp2))
            .await
            .unwrap()
            .unwrap();
        let text2 = String::from_utf8_lossy(&resp2);
        assert!(text2.starts_with("HTTP/1.1 404"), "{text2}");

        assert_eq!(hub.with_session(|session| session.flows().len()), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn relay_accept_puts_shares_in_the_session() {
        let dir = std::env::temp_dir().join(format!("zniff-relay-in-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let proxy = listen().await.unwrap();
        let addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());

        let id = "550e8400-e29b-41d4-a716-446655440000";
        let other = "550e8400-e29b-41d4-a716-446655440001";
        let missing = "550e8400-e29b-41d4-a716-446655440002";
        let expires = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let alpha = inbound_doc("alpha.test", 7, "ready", false, expires);
        let beta = inbound_doc("beta.test", 8, "beta", false, expires);

        let (status, _) = relay_exchange(
            addr,
            &relay_put(&format!("/s/{id}/flows.json"), "secret", &alpha),
        )
        .await;
        assert_eq!(status, 404);
        assert_eq!(hub.with_session(|session| session.flows().len()), 0);

        hub.set_relay_ingress(true, "a b");
        let (status, _) = relay_exchange(
            addr,
            &relay_put(&format!("/s/{id}/flows.json"), "a b", &alpha),
        )
        .await;
        assert_eq!(status, 401);
        hub.set_relay_token("secret");
        let (status, _) = relay_exchange(
            addr,
            &relay_put(&format!("/s/{id}/flows.json"), "nope", &alpha),
        )
        .await;
        assert_eq!(status, 401);
        assert_eq!(hub.with_session(|session| session.flows().len()), 0);

        let (status, _) = relay_exchange(
            addr,
            &relay_put(&format!("/s/{id}/flows.json"), "secret", &alpha),
        )
        .await;
        assert_eq!(status, 204);
        let (local_id, host, label) = hub.with_session(|session| {
            let flow = session.flows().into_iter().next().unwrap();
            (flow.id, flow.host.clone(), flow.client_ip.clone())
        });
        assert_eq!(host, "alpha.test");
        assert_eq!(label, "中继 550e8400");
        let flow = hub
            .with_session(|session| session.get(local_id).cloned())
            .unwrap();
        assert_eq!(flow.relay_source.as_deref(), Some(id));
        assert_eq!(flow.relay_remote_id, Some(7));
        assert_eq!(flow.response_body, b"ready");
        assert!(flow.comment.is_empty());
        assert_eq!(flow.messages.len(), 1);
        assert!(hub.with_session(|session| session.flows_matching("550e8400").len()) == 1);

        let (status, page) = relay_exchange(
            addr,
            &format!("GET /s/{id} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"),
        )
        .await;
        assert_eq!(status, 200);
        assert!(page.contains("alpha.test"), "{page}");
        let (status, flows) = relay_exchange(
            addr,
            &format!(
                "GET /s/{id}/flows.json?since=1 HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert_eq!(status, 200);
        assert!(flows.contains("\"incremental\":false"), "{flows}");
        assert!(!flows.contains("\"incremental\":true"), "{flows}");

        let packed = raw_exchange(
            addr,
            &format!(
                "GET /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        let packed_text = String::from_utf8_lossy(&packed);
        assert!(
            packed_text.contains("Content-Encoding: gzip"),
            "{packed_text}"
        );
        assert!(
            packed.windows(2).any(|pair| pair == [0x1f, 0x8b]),
            "gzip magic missing"
        );

        let (status, _) = relay_exchange(
            addr,
            &format!(
                "PUT http://{addr}/s/{other}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer secret\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{beta}",
                beta.len()
            ),
        )
        .await;
        assert_eq!(status, 204);
        let hosts = hub.with_session(|session| {
            session
                .flows()
                .into_iter()
                .map(|flow| flow.host)
                .collect::<Vec<_>>()
        });
        assert!(hosts.contains(&"alpha.test".to_string()), "{hosts:?}");
        assert!(hosts.contains(&"beta.test".to_string()), "{hosts:?}");

        assert!(hub.with_session(|session| session.set_pinned(local_id, true)));
        let gamma = inbound_doc("gamma.test", 7, "gamma", false, expires);
        let (status, _) = relay_exchange(
            addr,
            &relay_put(&format!("/s/{id}/flows.json"), "secret", &gamma),
        )
        .await;
        assert_eq!(status, 204);
        let updated = hub
            .with_session(|session| session.get(local_id).cloned())
            .unwrap();
        assert_eq!(updated.host, "gamma.test");
        assert!(updated.pinned);
        assert!(hub.with_session(|session| {
            session.flows().iter().any(|flow| flow.host == "beta.test")
        }));

        let (tx, rx) = std::sync::mpsc::channel();
        hub.replay_later(local_id, ReplayOverride::default(), move |result| {
            let _ = tx.send(result.err().map(|err| err.to_string()).unwrap_or_default());
        });
        let err = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(err.contains("中继"), "{err}");
        let (tx, rx) = std::sync::mpsc::channel();
        hub.replay_batch_later(
            local_id,
            2,
            1,
            0,
            ReplayOverride::default(),
            move |result| {
                let _ = tx.send(result.err().map(|err| err.to_string()).unwrap_or_default());
            },
        );
        let err = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(err.contains("中继"), "{err}");

        let spilled = inbound_doc("gamma.test", 7, "hello", true, expires);
        relay_exchange(
            addr,
            &relay_put(&format!("/s/{id}/flows.json"), "secret", &spilled),
        )
        .await;
        assert!(
            hub.with_session(|session| session.get(local_id).unwrap().response_body.is_empty())
        );
        let flow_body =
            r#"{"id":7,"request_body":"ping","response_body":"hello","messages":["← text pong"]}"#;
        let (status, _) = relay_exchange(
            addr,
            &relay_put(&format!("/s/{missing}/flows/7.json"), "secret", flow_body),
        )
        .await;
        assert_eq!(status, 409);
        let (status, _) = relay_exchange(
            addr,
            &relay_put(&format!("/s/{id}/flows/7.json"), "secret", flow_body),
        )
        .await;
        assert_eq!(status, 204);
        let filled = hub
            .with_session(|session| session.get(local_id).cloned())
            .unwrap();
        assert_eq!(filled.response_body, b"hello");
        assert_eq!(filled.id, local_id);
        let (status, body_page) = relay_exchange(
            addr,
            &format!(
                "GET /s/{id}/flows/7.json HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert_eq!(status, 200);
        assert!(body_page.contains("hello"), "{body_page}");

        let (status, _) = relay_exchange(
            addr,
            &format!(
                "DELETE /s/{other} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer nope\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert_eq!(status, 401);
        assert!(
            hub.with_session(|session| session.flows().iter().any(|flow| flow.host == "beta.test"))
        );
        let (status, _) = relay_exchange(
            addr,
            &format!(
                "DELETE /s/{other} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer secret\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert_eq!(status, 204);
        assert!(
            hub.with_session(|session| session.flows().iter().all(|flow| flow.host != "beta.test"))
        );
        assert!(hub.with_session(|session| session.get(local_id).is_some()));

        let local = hub.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "local.test".into(),
            path: "/mine".into(),
            url: "http://local.test/mine".into(),
            request_headers: Vec::new(),
        });
        let (status, _) = relay_exchange(
            addr,
            &format!(
                "DELETE /s/{id} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer secret\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert_eq!(status, 204);
        assert!(hub.with_session(|session| session.get(local_id).is_none()));
        assert!(hub.with_session(|session| session.get(local).is_some()));
        let (status, _) = relay_exchange(
            addr,
            &format!(
                "DELETE /s/{missing} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer secret\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert_eq!(status, 204);

        let (status, text) = relay_exchange(
            addr,
            &format!(
                "PUT /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer secret\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert_eq!(status, 411, "{text}");
        let (status, text) = relay_exchange(
            addr,
            &format!(
                "PUT /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer secret\r\nContent-Length: 99999999\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert_eq!(status, 413, "{text}");

        hub.set_relay_accept(false);
        let (status, _) = relay_exchange(
            addr,
            &relay_put(&format!("/s/{id}/flows.json"), "secret", &alpha),
        )
        .await;
        assert_eq!(status, 404);
        assert!(hub.with_session(|session| session.get(local).is_some()));
        assert_eq!(hub.with_session(|session| session.flows().len()), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn inbound_doc(host: &str, remote_id: u64, body: &str, spilled: bool, expires: u64) -> String {
        let flag = if spilled { "true" } else { "false" };
        let response = if spilled { "" } else { body };
        format!(
            r#"{{"expires":{expires},"filter":"host:{host}","omitted":0,"rev":3,"incremental":true,"ids":[{remote_id}],"flows":[{{"id":{remote_id},"method":"POST","status":201,"protocol":"HTTP/1.1","host":"{host}","path":"/inbox","url":"http://{host}/inbox","request_headers":[["Accept","text/plain"]],"response_headers":[["Content-Type","text/plain"]],"request_body":"ping","response_body":"{response}","request_body_spilled":false,"response_body_spilled":{flag},"request_bytes":4,"response_bytes":5,"ttfb_ms":8,"total_ms":20,"comment":"","rule_hit":"","messages":["→ text hi"]}}]}}"#
        )
    }

    fn relay_put(path: &str, token: &str, body: &str) -> String {
        format!(
            "PUT {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    async fn relay_exchange(addr: SocketAddr, request: &str) -> (u16, String) {
        let bytes = raw_exchange(addr, request).await;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        (status, text)
    }

    async fn raw_exchange(addr: SocketAddr, request: &str) -> Vec<u8> {
        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(request.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
            .await
            .unwrap()
            .unwrap();
        resp
    }

    #[test]
    fn share_document_keeps_only_the_frozen_filter() {
        let dir = std::env::temp_dir().join(format!("zniff-share-filter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Hub::new(ca).unwrap();
        hub.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "a.test".into(),
            path: "/secret".into(),
            url: "http://a.test/secret".into(),
            request_headers: vec![("Cookie".into(), "session=1".into())],
        });
        hub.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "b.test".into(),
            path: "/other".into(),
            url: "http://b.test/other".into(),
            request_headers: vec![("Accept".into(), "text/plain".into())],
        });
        hub.begin_share(Duration::from_secs(60), "header:cookie");
        let id = hub.share_id().expect("share id");
        let json = hub.share_document(&id).expect("document");
        assert!(json.contains("http://a.test/secret"), "{json}");
        assert!(!json.contains("http://b.test/other"), "{json}");
        assert!(json.contains("header:cookie"), "{json}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gzip_is_offered_only_when_the_coding_is_accepted() {
        assert!(!accepts_gzip(None));
        assert!(!accepts_gzip(Some("")));
        assert!(!accepts_gzip(Some("identity")));
        assert!(!accepts_gzip(Some("*")));
        assert!(!accepts_gzip(Some("gzip;q=0")));
        assert!(!accepts_gzip(Some("deflate, gzip;q=0.0")));
        assert!(!accepts_gzip(Some("gzip;q=nope")));
        assert!(accepts_gzip(Some("gzip")));
        assert!(accepts_gzip(Some("GZIP")));
        assert!(accepts_gzip(Some("deflate, gzip")));
        assert!(accepts_gzip(Some("br, gzip;q=0.5")));
        let packed = gzip_bytes(b"{\"ok\":true}").expect("gzip");
        assert!(packed.starts_with(&[0x1f, 0x8b]));
        let mut decoder = flate2::read::GzDecoder::new(packed.as_slice());
        let mut plain = Vec::new();
        std::io::Read::read_to_end(&mut decoder, &mut plain).unwrap();
        assert_eq!(plain, b"{\"ok\":true}");
    }

    #[tokio::test]
    async fn share_flows_json_is_gzipped_only_when_the_client_accepts_it() {
        let dir = std::env::temp_dir().join(format!("zniff-share-gzip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let flow_id = hub.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "share.example".into(),
            path: "/items".into(),
            url: "http://share.example/items".into(),
            request_headers: vec![("Accept".into(), "application/json".into())],
        });
        hub.set_response(
            flow_id,
            200,
            vec![("Content-Type".into(), "application/json".into())],
            None,
        );
        hub.append_body(flow_id, Direction::Response, b"{\"n\":1}");
        hub.mark_finished(flow_id);
        hub.begin_share(Duration::from_secs(60), "");
        let id = hub.share_id().expect("share id");
        let before = hub.with_session(|session| session.flows().len());

        let proxy = listen().await.unwrap();
        let addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());

        let plain_req =
            format!("GET /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
        let (plain_head, plain_body) = read_closed_http(addr, &plain_req).await;
        assert!(plain_head.starts_with("HTTP/1.1 200"), "{plain_head}");
        assert!(
            header_field(&plain_head, "content-encoding").is_none(),
            "{plain_head}"
        );
        assert_eq!(
            header_field(&plain_head, "content-length"),
            Some(plain_body.len().to_string()).as_deref()
        );
        assert_eq!(header_field(&plain_head, "cache-control"), Some("no-store"));
        assert!(
            plain_head
                .to_ascii_lowercase()
                .contains("connection: close")
        );
        assert!(
            plain_body
                .windows(b"share.example".len())
                .any(|w| w == b"share.example")
        );

        let gzip_req = format!(
            "GET /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAccept-Encoding: deflate, gzip\r\nConnection: close\r\n\r\n"
        );
        let (gzip_head, gzip_body) = read_closed_http(addr, &gzip_req).await;
        assert!(gzip_head.starts_with("HTTP/1.1 200"), "{gzip_head}");
        assert_eq!(header_field(&gzip_head, "content-encoding"), Some("gzip"));
        assert_eq!(
            header_field(&gzip_head, "content-length"),
            Some(gzip_body.len().to_string()).as_deref(),
            "{gzip_head}"
        );
        assert_eq!(
            header_field(&gzip_head, "content-type"),
            Some("application/json; charset=utf-8")
        );
        assert!(gzip_body.starts_with(&[0x1f, 0x8b]), "{gzip_body:?}");
        assert_ne!(gzip_body, plain_body);
        let mut decoder = flate2::read::GzDecoder::new(gzip_body.as_slice());
        let mut inflated = Vec::new();
        std::io::Read::read_to_end(&mut decoder, &mut inflated).unwrap();
        assert_eq!(inflated, plain_body);

        let refused = format!(
            "GET /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAccept-Encoding: gzip;q=0, identity\r\nConnection: close\r\n\r\n"
        );
        let (refused_head, refused_body) = read_closed_http(addr, &refused).await;
        assert!(
            header_field(&refused_head, "content-encoding").is_none(),
            "{refused_head}"
        );
        assert_eq!(refused_body, plain_body);

        let page_req = format!(
            "GET /s/{id} HTTP/1.1\r\nHost: {addr}\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n"
        );
        let (page_head, page_body) = read_closed_http(addr, &page_req).await;
        assert!(page_head.starts_with("HTTP/1.1 200"), "{page_head}");
        assert!(
            header_field(&page_head, "content-encoding").is_none(),
            "{page_head}"
        );
        assert!(page_body.starts_with(b"<!DOCTYPE html") || page_body.starts_with(b"<html"));

        let one_req = format!(
            "GET /s/{id}/flows/{flow_id}.json HTTP/1.1\r\nHost: {addr}\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n"
        );
        let (one_head, one_body) = read_closed_http(addr, &one_req).await;
        assert!(one_head.starts_with("HTTP/1.1 200"), "{one_head}");
        assert!(
            header_field(&one_head, "content-encoding").is_none(),
            "{one_head}"
        );
        let one_text = String::from_utf8_lossy(&one_body);
        assert!(one_text.contains("response_body"), "{one_text}");
        assert!(one_text.contains('n'), "{one_text}");

        assert_eq!(hub.with_session(|session| session.flows().len()), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    async fn read_closed_http(addr: std::net::SocketAddr, request: &str) -> (String, Vec<u8>) {
        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(request.as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        timeout(Duration::from_secs(5), client.read_to_end(&mut raw))
            .await
            .unwrap()
            .unwrap();
        let split = raw
            .windows(4)
            .position(|mark| mark == b"\r\n\r\n")
            .expect("http header");
        let head = String::from_utf8_lossy(&raw[..split]).into_owned();
        let body = raw[split + 4..].to_vec();
        (head, body)
    }

    fn header_field<'a>(head: &'a str, name: &str) -> Option<&'a str> {
        head.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then_some(value.trim())
        })
    }

    #[test]
    fn share_document_supports_incremental_since_and_lazy_spilled_body() {
        let dir = std::env::temp_dir().join(format!("zniff-share-inc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Hub::new(ca).unwrap();
        let f1 = hub.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "a.test".into(),
            path: "/big".into(),
            url: "http://a.test/big".into(),
            request_headers: Vec::new(),
        });
        hub.set_response(
            f1,
            200,
            vec![("Content-Type".into(), "text/plain".into())],
            None,
        );
        let big_payload = "x".repeat(20 * 1024);
        hub.append_body(f1, Direction::Response, big_payload.as_bytes());
        hub.mark_finished(f1);

        let f2 = hub.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "b.test".into(),
            path: "/hidden".into(),
            url: "http://b.test/hidden".into(),
            request_headers: Vec::new(),
        });
        hub.set_response(f2, 200, Vec::new(), None);
        hub.append_body(f2, Direction::Response, b"secret-b");
        hub.mark_finished(f2);

        hub.begin_share(Duration::from_secs(60), "host:a.test");
        let share_id = hub.share_id().expect("share id");

        // Initial snapshot: spilled body (>16KB) is NOT embedded in flows.json
        let snap_str = hub
            .share_document_since(&share_id, None)
            .expect("initial snapshot");
        let snap: serde_json::Value = serde_json::from_str(&snap_str).unwrap();
        let rev1 = snap["rev"].as_u64().unwrap();
        assert!(rev1 > 0);
        assert_eq!(snap["incremental"], false);
        assert_eq!(snap["ids"], serde_json::json!([f1]));
        assert_eq!(snap["flows"].as_array().unwrap().len(), 1);
        assert_eq!(snap["flows"][0]["response_body_spilled"], true);
        assert_eq!(snap["flows"][0]["response_body"], "");

        // Polling with since=rev1 when nothing changed returns empty flows
        let unchanged_str = hub
            .share_document_since(&share_id, Some(rev1))
            .expect("unchanged poll");
        let unchanged: serde_json::Value = serde_json::from_str(&unchanged_str).unwrap();
        assert_eq!(unchanged["incremental"], true);
        assert_eq!(unchanged["rev"], rev1);
        assert_eq!(unchanged["ids"], serde_json::json!([f1]));
        assert!(unchanged["flows"].as_array().unwrap().is_empty());

        // Clicking flow f1 fetches its spilled body on demand; f2 (filtered out) is rejected
        let body_str = hub
            .share_flow_body(&share_id, f1)
            .expect("lazy body for matching flow");
        let body_doc: serde_json::Value = serde_json::from_str(&body_str).unwrap();
        assert_eq!(body_doc["id"], f1);
        assert!(
            body_doc["response_body"]
                .as_str()
                .unwrap()
                .starts_with("xxxx")
        );
        assert!(hub.share_flow_body(&share_id, f2).is_none());

        // Adding a new matching flow f3: incremental poll since=rev1 returns ONLY f3
        let f3 = hub.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "POST".into(),
            host: "a.test".into(),
            path: "/small".into(),
            url: "http://a.test/small".into(),
            request_headers: Vec::new(),
        });
        let delta_str = hub
            .share_document_since(&share_id, Some(rev1))
            .expect("delta poll");
        let delta: serde_json::Value = serde_json::from_str(&delta_str).unwrap();
        assert_eq!(delta["incremental"], true);
        assert!(delta["rev"].as_u64().unwrap() > rev1);
        assert_eq!(delta["ids"], serde_json::json!([f1, f3]));
        assert_eq!(delta["flows"].as_array().unwrap().len(), 1);
        assert_eq!(delta["flows"][0]["id"], f3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn install_page_links_to_the_certificate() {
        let dir = std::env::temp_dir().join(format!("zniff-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(b"GET /zniff HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .unwrap();
        let mut resp = Vec::new();
        timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("200"), "{text}");
        assert!(text.contains("text/html"), "{text}");
        assert!(text.contains("href=\"/zniff-ca.crt\""), "{text}");
        let flow = hub
            .with_session(|session| session.get(1).cloned())
            .expect("flow");
        assert_eq!(flow.path, "/zniff");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ca_cert_route_returns_the_pem() {
        let dir = std::env::temp_dir().join(format!("zniff-ca-route-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let pem = std::fs::read(ca.cert_path()).unwrap();
        let hub = Arc::new(Hub::new(ca).unwrap());
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(b"GET http://127.0.0.1/zniff-ca.crt HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .unwrap();
        let mut resp = Vec::new();
        timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("200"), "{text}");
        assert!(text.contains("application/x-x509-ca-cert"), "{text}");
        assert!(
            resp.windows(pem.len())
                .any(|window| window == pem.as_slice())
        );
        let flow = hub
            .with_session(|session| session.get(1).cloned())
            .expect("flow");
        assert_eq!(flow.path, "/zniff-ca.crt");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn splits_connect_authority() {
        assert_eq!(
            split_host_port("example.com:443", 80).unwrap(),
            ("example.com".into(), 443)
        );
        assert_eq!(
            split_host_port("[::1]:8443", 443).unwrap(),
            ("::1".into(), 8443)
        );
    }

    #[test]
    fn strip_ws_extension_drops_only_that_header() {
        let raw = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nSec-WebSocket-Extensions: permessage-deflate\r\nConnection: Upgrade\r\n\r\n";
        let text = String::from_utf8(strip_ws_extension(raw)).unwrap();
        assert!(
            !text
                .to_ascii_lowercase()
                .contains("sec-websocket-extensions")
        );
        assert!(text.contains("Upgrade: websocket"));
        assert!(text.contains("Connection: Upgrade"));
        assert!(text.ends_with("\r\n\r\n"));
    }

    #[test]
    fn rule_switches_skip_matching_without_rewriting_the_text() {
        let dir = std::env::temp_dir().join(format!("zniff-mute-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Hub::new(ca).unwrap();
        hub.set_rules("delay example.com 1s\nbreak example.com");
        assert!(matches!(
            hub.plan("example.com", "/").action,
            Some(Action::BreakRequest)
        ));
        hub.set_breaks_enabled(false);
        let kept = hub.plan("example.com", "/");
        assert!(kept.action.is_none());
        assert!(!kept.delay.is_zero());
        hub.set_rules_enabled(false);
        assert_eq!(hub.plan("example.com", "/"), rules::Effect::default());
        assert_eq!(
            hub.rules
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .len(),
            2
        );
        let id = hub.with_session(|session| {
            session.open(FlowDraft {
                protocol: Protocol::Http11,
                method: "GET".into(),
                host: "example.com".into(),
                path: "/".into(),
                url: "http://example.com/".into(),
                request_headers: Vec::new(),
            })
        });
        hub.note_rules(id, "example.com", "/");
        assert!(hub.with_session(|session| session.get(id).unwrap().rule_hit.is_empty()));
        hub.set_rules_enabled(true);
        hub.note_rules(id, "example.com", "/");
        assert_eq!(
            hub.with_session(|session| session.get(id).unwrap().rule_hit.clone()),
            "delay"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_har_restores_timing_after_mark_finished() {
        let dir = std::env::temp_dir().join(format!("zniff-har-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Hub::new(ca).unwrap();
        let har = serde_json::json!({
            "log": {
                "entries": [{
                    "startedDateTime": "2023-11-14T22:13:20.000Z",
                    "time": 40,
                    "request": {
                        "method": "POST",
                        "url": "http://example.com/v1",
                        "httpVersion": "HTTP/1.1",
                        "headers": [],
                        "bodySize": 18,
                        "postData": {"mimeType": "text/plain", "text": "hi"}
                    },
                    "response": {
                        "status": 200,
                        "httpVersion": "HTTP/1.1",
                        "headers": [],
                        "bodySize": 4,
                        "content": {"mimeType": "text/plain", "text": "ok"}
                    },
                    "timings": {"wait": 15, "receive": 25}
                }]
            }
        });
        assert_eq!(hub.import_har(&har), 1);
        let flow = hub.with_session(|session| session.get(1).cloned()).unwrap();
        assert_eq!(flow.request_body, b"hi");
        assert_eq!(flow.response_body, b"ok");
        assert_eq!(flow.request_bytes, 18);
        assert_eq!(flow.response_bytes, 4);
        let wait = flow
            .response_at
            .unwrap()
            .duration_since(flow.started)
            .unwrap();
        let total = flow
            .finished_at
            .unwrap()
            .duration_since(flow.started)
            .unwrap();
        assert_eq!(wait, Duration::from_millis(15));
        assert_eq!(total, Duration::from_millis(40));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn websocket_extension_is_not_forwarded() {
        let dir = std::env::temp_dir().join(format!("zniff-ws-ext-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(String::new()));
        let seen_task = seen.clone();
        tokio::spawn(async move {
            let (mut sock, _) = origin.accept().await.unwrap();
            let req = read_h1_message(&mut sock).await;
            *seen_task.lock().unwrap() = String::from_utf8_lossy(&req).into_owned();
            sock.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nSec-WebSocket-Extensions: permessage-deflate\r\nConnection: Upgrade\r\n\r\n")
                .await
                .unwrap();
            sock.write_all(&[0x81, 0x05, b'p', b'l', b'a', b'i', b'n'])
                .await
                .unwrap();
        });
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_one(proxy, hub.clone());
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "GET http://{origin_addr}/chat HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Extensions: permessage-deflate\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        client.write_all(req.as_bytes()).await.unwrap();
        let header = read_h1_message(&mut client).await;
        let header_text = String::from_utf8_lossy(&header);
        assert!(header_text.contains("101"), "{header_text}");
        assert!(
            !header_text
                .to_ascii_lowercase()
                .contains("sec-websocket-extensions"),
            "{header_text}"
        );
        let mut frame = [0u8; 7];
        timeout(Duration::from_secs(5), client.read_exact(&mut frame))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&frame, &[0x81, 0x05, b'p', b'l', b'a', b'i', b'n']);
        let origin_text = seen.lock().unwrap().clone();
        assert!(
            !origin_text
                .to_ascii_lowercase()
                .contains("sec-websocket-extensions"),
            "{origin_text}"
        );
        assert!(
            origin_text
                .to_ascii_lowercase()
                .contains("upgrade: websocket"),
            "{origin_text}"
        );
        let flow = hub
            .with_session(|session| session.get(1).cloned())
            .expect("flow");
        assert_eq!(flow.messages.len(), 1);
        assert_eq!(flow.messages[0].text, "plain");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn replay_override_recalculates_content_length_and_skips_rules() {
        let dir = std::env::temp_dir().join(format!("zniff-replay-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let file = dir.join("mapped.txt");
        std::fs::write(&file, b"from-rule").unwrap();
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        hub.set_rules(&format!("map-local 127.0.0.1/new {}", file.display()));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_task = seen.clone();
        tokio::spawn(async move {
            let (mut sock, _) = origin.accept().await.unwrap();
            let req = read_h1_message(&mut sock).await;
            *seen_task.lock().unwrap() = req;
            sock.write_all(
                b"HTTP/1.1 201 Created\r\nContent-Length: 4\r\nConnection: close\r\n\r\nokay",
            )
            .await
            .unwrap();
        });
        let id = hub.with_session(|session| {
            session.open(FlowDraft {
                protocol: Protocol::Http11,
                method: "GET".into(),
                host: "127.0.0.1".into(),
                path: "/old".into(),
                url: format!("http://{origin_addr}/old"),
                request_headers: vec![
                    ("Host".into(), origin_addr.to_string()),
                    ("Content-Length".into(), "1".into()),
                    ("X-Old".into(), "keep".into()),
                ],
            })
        });
        let (tx, rx) = tokio::sync::oneshot::channel();
        hub.replay_later(
            id,
            ReplayOverride {
                method: Some("POST".into()),
                url: Some(format!("http://{origin_addr}/new?x=1")),
                headers: Some(vec![
                    ("Content-Length".into(), "1".into()),
                    ("X-Edit".into(), "yes".into()),
                    ("Host".into(), "wrong.test".into()),
                ]),
                body: Some(b"abcdef".to_vec()),
            },
            move |result| {
                let _ = tx.send(result);
            },
        );
        let new_id = timeout(Duration::from_secs(5), rx)
            .await
            .expect("重放超时")
            .expect("重放任务中断")
            .expect("重放失败");
        let text = String::from_utf8(seen.lock().unwrap().clone()).unwrap();
        assert!(text.starts_with("POST /new?x=1 HTTP/1.1"), "{text}");
        assert!(text.contains("Content-Length: 6\r\n"), "{text}");
        assert!(!text.contains("Content-Length: 1"), "{text}");
        assert!(text.contains("X-Edit: yes"), "{text}");
        assert!(!text.contains("X-Old"), "{text}");
        assert!(
            !text.to_ascii_lowercase().contains("host: wrong.test"),
            "{text}"
        );
        assert!(text.ends_with("abcdef"), "{text}");
        let flow = hub
            .with_session(|session| session.get(new_id).cloned())
            .unwrap();
        assert_eq!(flow.method, "POST");
        assert!(flow.url.contains("/new?x=1"), "{}", flow.url);
        assert_eq!(flow.status, Some(201));
        assert_eq!(flow.request_body, b"abcdef");
        assert_eq!(flow.response_body, b"okay");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn h2_sender_cache_reuses_the_origin_connection() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-h2-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        let hub = Arc::new(hub);
        let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
        let (origin_addr, accepts) = spawn_h2_origin(server).await;
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());
        let first = h2_get(&ca, proxy_addr, origin_addr, "/a").await;
        assert_eq!(first.0, StatusCode::OK);
        assert_eq!(&first.1[..], b"h2-/a");
        let second = h2_get(&ca, proxy_addr, origin_addr, "/b").await;
        assert_eq!(second.0, StatusCode::OK);
        assert_eq!(&second.1[..], b"h2-/b");
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(accepts.load(std::sync::atomic::Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn h2_sender_cache_misses_after_idle() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-h2-idle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        let hub = Arc::new(hub);
        let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
        let (origin_addr, accepts) = spawn_h2_origin(server).await;
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());
        let first = h2_get(&ca, proxy_addr, origin_addr, "/a").await;
        assert_eq!(first.0, StatusCode::OK);
        hub.expire_h2_cache();
        let second = h2_get(&ca, proxy_addr, origin_addr, "/b").await;
        assert_eq!(second.0, StatusCode::OK);
        assert_eq!(&second.1[..], b"h2-/b");
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(accepts.load(std::sync::atomic::Ordering::SeqCst), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn h2_lane_redials_on_the_same_client_connection() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-h2-redial-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        let hub = Arc::new(hub);
        let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
        let (origin_addr, accepts) = spawn_h2_origin(server).await;
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());
        let mut sender = h2_client(&ca, proxy_addr, origin_addr).await;
        let first = h2_send(&mut sender, "/a").await;
        assert_eq!(first.0, StatusCode::OK);
        assert_eq!(&first.1[..], b"h2-/a");
        hub.refresh_h2
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let second = h2_send(&mut sender, "/b").await;
        assert_eq!(second.0, StatusCode::OK);
        assert_eq!(&second.1[..], b"h2-/b");
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(accepts.load(std::sync::atomic::Ordering::SeqCst), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn https_map_local_answers_from_the_file() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-https-map-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        let file = dir.join("body.txt");
        std::fs::write(&file, b"from-file").unwrap();
        hub.set_rules(&format!("map-local 127.0.0.1/hi {}", file.display()));
        let hub = Arc::new(hub);
        let server = origin_tls_config(
            &origin_ca,
            &origin_leaf,
            &leaf_key,
            vec![b"http/1.1".to_vec()],
        );
        let (origin_addr, seen) = spawn_tls_sink(server);
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_one(proxy, hub.clone());
        let mut tls = connect_mitm(proxy_addr, origin_addr, &ca, vec![b"http/1.1".to_vec()]).await;
        tls.write_all(b"GET /hi HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let resp = read_h1_message(&mut tls).await;
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("200"), "{text}");
        assert!(text.contains("from-file"), "{text}");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            seen.lock().unwrap().is_empty(),
            "map local 仍把请求送到了源站"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn https_map_remote_fetches_the_other_origin() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-https-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        let remote = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let remote_addr = remote.local_addr().unwrap();
        hub.set_rules(&format!(
            "map-remote 127.0.0.1/hi http://{remote_addr}/landed"
        ));
        let hub = Arc::new(hub);
        let seen = Arc::new(Mutex::new(String::new()));
        let seen_task = seen.clone();
        tokio::spawn(async move {
            let (mut sock, _) = remote.accept().await.unwrap();
            let req = read_h1_message(&mut sock).await;
            *seen_task.lock().unwrap() = String::from_utf8_lossy(&req).into_owned();
            sock.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nremote-ok",
            )
            .await
            .unwrap();
        });
        let server = origin_tls_config(
            &origin_ca,
            &origin_leaf,
            &leaf_key,
            vec![b"http/1.1".to_vec()],
        );
        let (origin_addr, _) = spawn_tls_sink(server);
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_one(proxy, hub.clone());
        let mut tls = connect_mitm(proxy_addr, origin_addr, &ca, vec![b"http/1.1".to_vec()]).await;
        tls.write_all(b"GET /hi HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let resp = read_h1_message(&mut tls).await;
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("remote-ok"), "{text}");
        let remote_text = seen.lock().unwrap().clone();
        assert!(remote_text.contains("/landed"), "{remote_text}");
        let flow = hub.with_session(|session| session.get(1).cloned()).unwrap();
        assert!(flow.url.contains("/landed"), "{}", flow.url);
        assert_eq!(flow.response_body, b"remote-ok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn https_map_remote_preserves_subpath_query_host_and_origin_form() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir =
            std::env::temp_dir().join(format!("zniff-https-remote-subpath-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());

        let remote = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let remote_addr = remote.local_addr().unwrap();
        hub.set_rules(&format!("map-remote 127.0.0.1/api http://{remote_addr}/v2"));
        let hub = Arc::new(hub);
        let seen = Arc::new(Mutex::new(String::new()));
        let seen_task = seen.clone();
        tokio::spawn(async move {
            let (mut sock, _) = remote.accept().await.unwrap();
            let req = read_h1_message(&mut sock).await;
            *seen_task.lock().unwrap() = String::from_utf8_lossy(&req).into_owned();
            sock.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nremote-ok",
            )
            .await
            .unwrap();
        });

        let server = origin_tls_config(
            &origin_ca,
            &origin_leaf,
            &leaf_key,
            vec![b"http/1.1".to_vec()],
        );
        let (origin_addr, _) = spawn_tls_sink(server);
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_one(proxy, hub.clone());

        let mut tls = connect_mitm(proxy_addr, origin_addr, &ca, vec![b"http/1.1".to_vec()]).await;
        tls.write_all(
            b"GET /api/users?id=1 HTTP/1.1\r\nHost: client-custom.test\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        let resp = read_h1_message(&mut tls).await;
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("remote-ok"), "{text}");

        let remote_text = seen.lock().unwrap().clone();
        // 1. Verify origin-form request line (NOT absolute-form "GET http://...")
        assert!(
            remote_text.starts_with("GET /v2/users?id=1 HTTP/1.1\r\n"),
            "Expected origin-form request line, got: {remote_text}"
        );
        // 2. Verify Host header was rewritten to destination authority
        assert!(
            remote_text
                .to_ascii_lowercase()
                .contains(&format!("host: {remote_addr}\r\n")),
            "Expected rewritten Host header {remote_addr}, got: {remote_text}"
        );
        // 3. Verify original Host was NOT forwarded
        assert!(
            !remote_text.contains("client-custom.test"),
            "Original Host leaked: {remote_text}"
        );

        let flow = hub.with_session(|session| session.get(1).cloned()).unwrap();
        assert_eq!(flow.url, format!("http://{remote_addr}/v2/users?id=1"));
        assert_eq!(flow.path, "/v2/users?id=1");
        assert_eq!(flow.host, "127.0.0.1");
        assert_eq!(flow.response_body, b"remote-ok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn plain_http_map_remote_preserves_subpath_query_and_rewrites_host() {
        let dir = std::env::temp_dir().join(format!("zniff-plain-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Hub::new(ca.clone()).unwrap();

        let remote = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let remote_addr = remote.local_addr().unwrap();
        hub.set_rules(&format!("map-remote fake.test/api http://{remote_addr}/v2"));
        let hub = Arc::new(hub);
        let seen = Arc::new(Mutex::new(String::new()));
        let seen_task = seen.clone();
        tokio::spawn(async move {
            let (mut sock, _) = remote.accept().await.unwrap();
            let req = read_h1_message(&mut sock).await;
            *seen_task.lock().unwrap() = String::from_utf8_lossy(&req).into_owned();
            sock.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\nplain-remote-ok",
            )
            .await
            .unwrap();
        });

        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_one(proxy, hub.clone());

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(
                b"GET http://fake.test/api/users?id=1 HTTP/1.1\r\nHost: fake.test\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        let resp = read_h1_message(&mut client).await;
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("plain-remote-ok"), "{text}");

        let remote_text = seen.lock().unwrap().clone();
        assert!(
            remote_text.starts_with("GET /v2/users?id=1 HTTP/1.1\r\n"),
            "Expected origin-form request line, got: {remote_text}"
        );
        assert!(
            remote_text
                .to_ascii_lowercase()
                .contains(&format!("host: {remote_addr}\r\n")),
            "Expected rewritten Host header, got: {remote_text}"
        );
        assert!(
            !remote_text.contains("fake.test"),
            "Original Host leaked: {remote_text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn https_break_continue_rewrites_the_request() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-https-break-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        hub.set_rules("break 127.0.0.1/hold");
        let hub = Arc::new(hub);
        let server = origin_tls_config(
            &origin_ca,
            &origin_leaf,
            &leaf_key,
            vec![b"http/1.1".to_vec()],
        );
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(String::new()));
        let seen_task = seen.clone();
        tokio::spawn(async move {
            let (sock, _) = origin.accept().await.unwrap();
            let mut tls = acceptor.accept(sock).await.unwrap();
            let req = read_h1_message(&mut tls).await;
            *seen_task.lock().unwrap() = String::from_utf8_lossy(&req).into_owned();
            tls.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\ncontinued",
            )
            .await
            .unwrap();
        });
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_one(proxy, hub.clone());
        let hub_wait = hub.clone();
        let waiter = tokio::spawn(async move {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                if hub_wait.paused_id() == Some(1) {
                    assert!(hub_wait.resolve_break(BreakDecision {
                        drop: false,
                        headers: Some(vec![
                            ("Host".into(), "127.0.0.1".into()),
                            ("X-Edit".into(), "yes".into()),
                            ("Content-Length".into(), "99".into()),
                        ]),
                        body: Some(b"abcdef".to_vec()),
                        status: None,
                    }));
                    return;
                }
                if std::time::Instant::now() > deadline {
                    panic!("断点没有暂停");
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let mut tls = connect_mitm(proxy_addr, origin_addr, &ca, vec![b"http/1.1".to_vec()]).await;
        tls.write_all(b"POST /hold HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 8\r\nConnection: close\r\n\r\noriginal")
            .await
            .unwrap();
        let resp = read_h1_message(&mut tls).await;
        waiter.await.unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("continued"), "{text}");
        let origin_text = seen.lock().unwrap().clone();
        assert!(origin_text.contains("x-edit: yes"), "{origin_text}");
        assert!(origin_text.contains("Content-Length: 6"), "{origin_text}");
        assert!(origin_text.ends_with("abcdef"), "{origin_text}");
        assert!(!origin_text.contains("original"), "{origin_text}");
        let flow = hub.with_session(|session| session.get(1).cloned()).unwrap();
        assert_eq!(flow.request_body, b"abcdef");
        assert_eq!(flow.response_body, b"continued");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn https_h2_break_drop_does_not_reach_the_origin() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-h2-break-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        hub.set_rules("break 127.0.0.1/hold");
        let hub = Arc::new(hub);
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hits_task = hits.clone();
        let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (sock, _) = origin.accept().await.unwrap();
            let Ok(tls) = acceptor.accept(sock).await else {
                return;
            };
            let hits_task = hits_task.clone();
            let service = hyper::service::service_fn(move |_req: Request<Incoming>| {
                hits_task.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move {
                    Ok::<_, std::convert::Infallible>(Response::new(http_body_util::Full::new(
                        Bytes::from_static(b"origin"),
                    )))
                }
            });
            let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(tls), service)
                .await;
        });
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_one(proxy, hub.clone());
        let hub_wait = hub.clone();
        let waiter = tokio::spawn(async move {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                if hub_wait.paused_id().is_some() {
                    assert!(hub_wait.resolve_break(BreakDecision {
                        drop: true,
                        headers: None,
                        body: None,
                        status: None,
                    }));
                    return;
                }
                if std::time::Instant::now() > deadline {
                    panic!("断点没有暂停");
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let mut sender = h2_client(&ca, proxy_addr, origin_addr).await;
        let req = Request::builder()
            .method(Method::GET)
            .uri("/hold")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let res = sender.send_request(req).await;
        waiter.await.unwrap();
        assert!(res.is_err(), "expected H2 stream reset on breakpoint drop");
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
        let flow = hub
            .with_session(|session| session.flows().into_iter().next())
            .unwrap();
        assert_eq!(flow.status, None);
        assert_eq!(flow.error.as_deref(), Some("断点丢弃"));
        assert!(flow.finished_at.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn https_h2_map_local_answers_from_the_file() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-h2-map-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        let file = dir.join("body.txt");
        std::fs::write(&file, b"from-h2-file").unwrap();
        hub.set_rules(&format!("map-local 127.0.0.1/hi {}", file.display()));
        let hub = Arc::new(hub);
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hits_task = hits.clone();
        let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (sock, _) = origin.accept().await.unwrap();
            let Ok(tls) = acceptor.accept(sock).await else {
                return;
            };
            let hits_task = hits_task.clone();
            let service = hyper::service::service_fn(move |_req: Request<Incoming>| {
                hits_task.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move {
                    Ok::<_, std::convert::Infallible>(Response::new(http_body_util::Full::new(
                        Bytes::from_static(b"origin"),
                    )))
                }
            });
            let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(tls), service)
                .await;
        });
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_one(proxy, hub.clone());
        let (status, body) = h2_get(&ca, proxy_addr, origin_addr, "/hi").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(&body[..], b"from-h2-file");
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn serve_one(proxy: TcpListener, hub: Arc<Hub>) {
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub).await;
        });
    }

    fn serve_proxy(proxy: TcpListener, hub: Arc<Hub>) {
        tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = proxy.accept().await else {
                    break;
                };
                let hub = hub.clone();
                tokio::spawn(async move {
                    let _ = handle_conn(socket, hub).await;
                });
            }
        });
    }

    fn origin_tls_config(
        origin_ca: &rcgen::Certificate,
        origin_leaf: &rcgen::Certificate,
        leaf_key: &rcgen::KeyPair,
        alpn: Vec<Vec<u8>>,
    ) -> rustls::ServerConfig {
        let mut server = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![origin_leaf.der().clone(), origin_ca.der().clone()],
                rustls::pki_types::PrivateKeyDer::Pkcs8(
                    rustls::pki_types::PrivatePkcs8KeyDer::from(leaf_key.serialize_der()),
                ),
            )
            .unwrap();
        server.alpn_protocols = alpn;
        server
    }

    fn spawn_tls_sink(server: rustls::ServerConfig) -> (std::net::SocketAddr, Arc<Mutex<Vec<u8>>>) {
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_task = seen.clone();
        let origin = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        origin.set_nonblocking(true).unwrap();
        let addr = origin.local_addr().unwrap();
        let origin = TcpListener::from_std(origin).unwrap();
        tokio::spawn(async move {
            let Ok((sock, _)) = origin.accept().await else {
                return;
            };
            let Ok(mut tls) = acceptor.accept(sock).await else {
                return;
            };
            let mut buf = vec![0u8; 1024];
            if let Ok(Ok(n)) = timeout(Duration::from_millis(250), tls.read(&mut buf)).await {
                seen_task.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
        (addr, seen)
    }

    async fn connect_mitm(
        proxy_addr: std::net::SocketAddr,
        origin_addr: std::net::SocketAddr,
        ca: &Ca,
        alpn: Vec<Vec<u8>>,
    ) -> tokio_rustls::client::TlsStream<TcpStream> {
        let mut tcp = TcpStream::connect(proxy_addr).await.unwrap();
        let connect = format!("CONNECT {origin_addr} HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n");
        tcp.write_all(connect.as_bytes()).await.unwrap();
        read_http_head(&mut tcp).await;
        let mut roots = RootCertStore::empty();
        roots.add(ca.cert_der()).unwrap();
        let mut client_config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        client_config.alpn_protocols = alpn;
        timeout(
            Duration::from_secs(5),
            TlsConnector::from(Arc::new(client_config))
                .connect(ServerName::IpAddress(origin_addr.ip().into()), tcp),
        )
        .await
        .expect("客户端 TLS 超时")
        .expect("客户端 TLS")
    }

    async fn spawn_h2_origin(
        server: rustls::ServerConfig,
    ) -> (std::net::SocketAddr, Arc<std::sync::atomic::AtomicUsize>) {
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        let accepts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let accepts_task = accepts.clone();
        tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = origin.accept().await else {
                    break;
                };
                accepts_task.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let Ok(tls) = acceptor.accept(sock).await else {
                        return;
                    };
                    let service = hyper::service::service_fn(|req: Request<Incoming>| async move {
                        let body = format!("h2-{}", req.uri().path());
                        Ok::<_, std::convert::Infallible>(Response::new(http_body_util::Full::new(
                            Bytes::from(body),
                        )))
                    });
                    let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                        .serve_connection(TokioIo::new(tls), service)
                        .await;
                });
            }
        });
        (origin_addr, accepts)
    }

    async fn h2_client(
        ca: &Ca,
        proxy_addr: std::net::SocketAddr,
        origin_addr: std::net::SocketAddr,
    ) -> hyper::client::conn::http2::SendRequest<http_body_util::Full<Bytes>> {
        let tls = connect_mitm(proxy_addr, origin_addr, ca, vec![b"h2".to_vec()]).await;
        assert_eq!(tls.get_ref().1.alpn_protocol(), Some(b"h2".as_slice()));
        let (sender, connection) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
            .handshake::<_, http_body_util::Full<Bytes>>(TokioIo::new(tls))
            .await
            .expect("客户端 HTTP/2");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        sender
    }

    async fn h2_send(
        sender: &mut hyper::client::conn::http2::SendRequest<http_body_util::Full<Bytes>>,
        path: &str,
    ) -> (StatusCode, Bytes) {
        let req = Request::builder()
            .uri(path)
            .body(http_body_util::Full::new(Bytes::new()))
            .unwrap();
        let response = timeout(Duration::from_secs(5), sender.send_request(req))
            .await
            .expect("HTTP/2 响应超时")
            .expect("HTTP/2 响应");
        let status = response.status();
        let body = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        (status, body)
    }

    async fn h2_get(
        ca: &Ca,
        proxy_addr: std::net::SocketAddr,
        origin_addr: std::net::SocketAddr,
        path: &str,
    ) -> (StatusCode, Bytes) {
        let tls = connect_mitm(proxy_addr, origin_addr, ca, vec![b"h2".to_vec()]).await;
        assert_eq!(tls.get_ref().1.alpn_protocol(), Some(b"h2".as_slice()));
        let (mut sender, connection) =
            hyper::client::conn::http2::Builder::new(TokioExecutor::new())
                .handshake::<_, http_body_util::Full<Bytes>>(TokioIo::new(tls))
                .await
                .expect("客户端 HTTP/2");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let req = Request::builder()
            .uri(path)
            .body(http_body_util::Full::new(Bytes::new()))
            .unwrap();
        let response = timeout(Duration::from_secs(5), sender.send_request(req))
            .await
            .expect("HTTP/2 响应超时")
            .expect("HTTP/2 响应");
        let status = response.status();
        let body = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        (status, body)
    }

    async fn read_h1_message(reader: &mut (impl AsyncRead + Unpin)) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let n = timeout(Duration::from_secs(5), reader.read(&mut byte))
                .await
                .expect("读超时")
                .expect("读失败");
            assert_ne!(n, 0, "连接提前关闭");
            buf.push(byte[0]);
            if buf.ends_with(b"\r\n\r\n") {
                break;
            }
            assert!(buf.len() < 16 * 1024, "头过长");
        }
        let head = String::from_utf8_lossy(&buf);
        let len = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("content-length") {
                    value.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);
        let start = buf.len();
        buf.resize(start + len, 0);
        if len > 0 {
            timeout(Duration::from_secs(5), reader.read_exact(&mut buf[start..]))
                .await
                .expect("读 body 超时")
                .unwrap();
        }
        buf
    }

    use bytes::Bytes;
    use http_body::{Body, Frame};
    use std::pin::Pin;
    use std::task::Poll;

    struct ChunkThenEnd {
        pending: std::collections::VecDeque<Bytes>,
    }

    impl Body for ChunkThenEnd {
        type Data = Bytes;
        type Error = std::convert::Infallible;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
            if let Some(chunk) = self.pending.pop_front() {
                return Poll::Ready(Some(Ok(Frame::data(chunk))));
            }
            // The caller stops once `is_end_stream` is true and never polls `None`.
            Poll::Pending
        }

        fn is_end_stream(&self) -> bool {
            self.pending.is_empty()
        }
    }

    struct AlreadyEnded;

    impl Body for AlreadyEnded {
        type Data = Bytes;
        type Error = std::convert::Infallible;

        fn poll_frame(
            self: Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
            Poll::Pending
        }

        fn is_end_stream(&self) -> bool {
            true
        }
    }

    fn open_recorded(hub: &Hub) -> u64 {
        hub.with_session(|session| {
            session.open(FlowDraft {
                protocol: Protocol::Http2,
                method: "GET".into(),
                host: "example.com".into(),
                path: "/".into(),
                url: "https://example.com/".into(),
                request_headers: Vec::new(),
            })
        })
    }

    fn poll_once<B>(body: &mut RecordingBody<B>)
    where
        B: Body<Data = Bytes> + Unpin,
    {
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        let _ = Pin::new(body).poll_frame(&mut cx);
    }

    #[test]
    fn recording_body_finishes_on_end_of_stream_without_a_none_poll() {
        let dir = std::env::temp_dir().join(format!("zniff-body-end-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let id = open_recorded(&hub);
        hub.with_session(|session| session.set_response(id, 200, Vec::new(), None));
        {
            let mut body = RecordingBody::new(
                ChunkThenEnd {
                    pending: std::collections::VecDeque::from([Bytes::from_static(b"hi")]),
                },
                hub.clone(),
                id,
                Direction::Response,
                None,
            );
            poll_once(&mut body);
            let flow = hub
                .with_session(|session| session.get(id).cloned())
                .unwrap();
            assert_eq!(flow.response_body, b"hi");
            assert!(
                flow.finished_at.is_some(),
                "last frame must finish the flow"
            );
        }

        let empty = open_recorded(&hub);
        hub.with_session(|session| session.set_response(empty, 304, Vec::new(), None));
        {
            let body =
                RecordingBody::new(AlreadyEnded, hub.clone(), empty, Direction::Response, None);
            drop(body);
        }
        let flow = hub
            .with_session(|session| session.get(empty).cloned())
            .unwrap();
        assert!(flow.response_body.is_empty());
        assert!(
            flow.finished_at.is_some(),
            "dropping an already-ended body must finish the flow"
        );

        let request = open_recorded(&hub);
        {
            let body =
                RecordingBody::new(AlreadyEnded, hub.clone(), request, Direction::Request, None);
            drop(body);
        }
        let flow = hub
            .with_session(|session| session.get(request).cloned())
            .unwrap();
        assert!(
            flow.finished_at.is_none(),
            "the request side copy must not mark the flow finished"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn status_only_map_local_query_rewrite_and_composed_request_work() {
        let dir = std::env::temp_dir().join(format!("zniff-p1-proxy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        hub.set_rules(
            "map-local 127.0.0.1/status-only status=204\n\
             query 127.0.0.1/items set env staging\n\
             query 127.0.0.1/items del drop\n",
        );

        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });

        // 1. Status-only map-local returns 204 with empty body without dialing origin.
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(b"GET http://127.0.0.1:9/status-only HTTP/1.1\r\nHost: 127.0.0.1:9\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut raw = Vec::new();
        client.read_to_end(&mut raw).await.unwrap();
        let head = String::from_utf8_lossy(&raw);
        assert!(head.starts_with("HTTP/1.1 204 "), "{head}");

        // 2. Standalone composed request + query rule rewrite against a real local origin.
        let origin = listen().await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = origin.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            let req_text = String::from_utf8_lossy(&buf[..n]);
            assert!(
                req_text.starts_with("GET /items?keep=1&env=staging HTTP/1.1\r\n"),
                "{req_text}"
            );
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        });

        let origin2 = listen().await.unwrap();
        let origin2_addr = origin2.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = origin2.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = socket.read(&mut buf).await.unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 201 Created\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndone",
                )
                .await
                .unwrap();
        });

        let composed_id = replay_once(
            hub.clone(),
            None,
            ReplayOverride {
                method: Some("POST".into()),
                url: Some(format!("http://{origin2_addr}/scratch")),
                headers: Some(vec![("X-From".into(), "scratchpad".into())]),
                body: Some(b"hello".to_vec()),
            },
        )
        .await
        .unwrap();
        let composed_flow = hub
            .with_session(|session| session.get(composed_id).cloned())
            .unwrap();
        assert_eq!(composed_flow.status, Some(201));
        assert_eq!(composed_flow.response_body, b"done");

        let proxy3 = listen().await.unwrap();
        let proxy3_addr = proxy3.local_addr().unwrap();
        let hub_serve3 = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy3.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve3).await;
        });
        let mut client2 = TcpStream::connect(proxy3_addr).await.unwrap();
        let req = format!(
            "GET http://{origin_addr}/items?keep=1&drop=2 HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
        );
        client2.write_all(req.as_bytes()).await.unwrap();
        let mut raw2 = Vec::new();
        client2.read_to_end(&mut raw2).await.unwrap();
        assert!(String::from_utf8_lossy(&raw2).ends_with("ok"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn breakpoint_fifo_queue_and_local_app_cache_work() {
        let dir = std::env::temp_dir().join(format!("zniff-break-q-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());

        let h1 = hub.clone();
        let w1 = tokio::spawn(async move { h1.wait_break(10, BreakSide::Request).await });
        tokio::time::sleep(Duration::from_millis(15)).await;
        let h2 = hub.clone();
        let w2 = tokio::spawn(async move { h2.wait_break(20, BreakSide::Response).await });
        let h3 = hub.clone();
        let w3 = tokio::spawn(async move { h3.wait_break(30, BreakSide::Request).await });

        let deadline = Instant::now() + Duration::from_secs(2);
        while hub.paused_queue_len() < 3 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(hub.paused_queue_len(), 3);
        assert_eq!(hub.paused(), Some((10, BreakSide::Request)));

        assert!(hub.resolve_break(BreakDecision {
            drop: true,
            headers: None,
            body: None,
            status: None,
        }));
        let d1 = w1.await.unwrap();
        assert!(d1.drop);

        assert_eq!(hub.paused_queue_len(), 2);
        assert_eq!(hub.paused(), Some((20, BreakSide::Response)));

        assert_eq!(hub.resolve_all_breaks(), 2);
        assert_eq!(hub.paused_queue_len(), 0);
        let d2 = w2.await.unwrap();
        let d3 = w3.await.unwrap();
        assert!(!d2.drop);
        assert!(!d3.drop);

        store_cached_local_app(65431, "curl".into());
        assert_eq!(cached_local_app(65431).as_deref(), Some("curl"));
        assert_eq!(resolve_local_process_name(0), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn promote_break_selects_that_slot_without_releasing_the_others() {
        let dir = std::env::temp_dir().join(format!("zniff-break-promote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());

        let h1 = hub.clone();
        let w1 = tokio::spawn(async move { h1.wait_break(10, BreakSide::Request).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let h2 = hub.clone();
        let w2 = tokio::spawn(async move { h2.wait_break(20, BreakSide::Response).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let h3 = hub.clone();
        let mut w3 = tokio::spawn(async move { h3.wait_break(30, BreakSide::Request).await });

        let deadline = Instant::now() + Duration::from_secs(2);
        while hub.paused_queue_len() < 3 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            hub.paused_queue(),
            vec![
                (10, BreakSide::Request),
                (20, BreakSide::Response),
                (30, BreakSide::Request),
            ]
        );
        assert!(hub.promote_break(30));
        assert_eq!(hub.paused(), Some((30, BreakSide::Request)));
        assert_eq!(
            hub.paused_queue(),
            vec![
                (30, BreakSide::Request),
                (10, BreakSide::Request),
                (20, BreakSide::Response),
            ]
        );
        assert!(!hub.promote_break(99));
        assert_eq!(hub.paused_queue_len(), 3);
        assert!(hub.promote_break(30));
        assert_eq!(hub.paused_queue()[0], (30, BreakSide::Request));

        let mut w1 = w1;
        let mut w2 = w2;
        assert!(
            timeout(Duration::from_millis(80), &mut w1).await.is_err(),
            "promoting another slot released the first"
        );
        assert!(timeout(Duration::from_millis(80), &mut w2).await.is_err());

        assert!(hub.resolve_break(BreakDecision {
            drop: true,
            headers: None,
            body: None,
            status: None,
        }));
        let dropped = timeout(Duration::from_secs(2), &mut w3)
            .await
            .expect("promoted break did not finish")
            .unwrap();
        assert!(dropped.drop);
        assert_eq!(hub.paused(), Some((10, BreakSide::Request)));
        assert_eq!(hub.paused_queue_len(), 2);

        assert_eq!(hub.resolve_all_breaks(), 2);
        let kept_first = w1.await.unwrap();
        let kept_second = w2.await.unwrap();
        assert!(!kept_first.drop);
        assert!(!kept_second.drop);
        assert!(hub.paused_queue().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn replay_batch_repeats_sequentially_and_records_each_flow() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-repeat-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());

        let origin = listen().await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            for seq in 1..=3u32 {
                let (mut socket, _) = origin.accept().await.unwrap();
                let mut buf = vec![0u8; 4096];
                let _ = socket.read(&mut buf).await.unwrap();
                let body = format!("hit-{seq}");
                let res = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(res.as_bytes()).await.unwrap();
            }
        });

        let source_id = hub.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: origin_addr.to_string(),
            path: "/repeat-target".into(),
            url: format!("http://{origin_addr}/repeat-target"),
            request_headers: vec![("Host".into(), origin_addr.to_string())],
        });
        hub.set_response(source_id, 200, Vec::new(), None);
        hub.mark_finished(source_id);
        let source_flow = hub
            .with_session(|session| session.hydrated(source_id))
            .unwrap();

        let ids = replay_batch(
            hub.clone(),
            source_flow.clone(),
            3,
            1,
            5,
            ReplayOverride::default(),
        )
        .await
        .unwrap();
        assert_eq!(ids.len(), 3);
        for (idx, id) in ids.iter().enumerate() {
            let flow = hub
                .with_session(|session| session.get(*id).cloned())
                .unwrap();
            assert_eq!(flow.status, Some(200));
            assert_eq!(
                String::from_utf8_lossy(&flow.response_body),
                format!("hit-{}", idx + 1)
            );
            assert!(
                flow.comment
                    .contains(&format!("repeat:{source_id} ({}/3)", idx + 1))
            );
        }
        let repeat_filtered =
            hub.with_session(|session| session.flows_matching(&format!("repeat:{source_id}")));
        assert_eq!(repeat_filtered.len(), 4);

        // A transient origin failure on an intermediate iteration records ERR on that flow
        // and continues replaying the remaining iterations.
        let flaky_origin = listen().await.unwrap();
        let flaky_addr = flaky_origin.local_addr().unwrap();
        tokio::spawn(async move {
            for seq in 1..=3u32 {
                let (mut socket, _) = flaky_origin.accept().await.unwrap();
                let mut buf = vec![0u8; 4096];
                let _ = socket.read(&mut buf).await.unwrap();
                if seq == 2 {
                    drop(socket);
                    continue;
                }
                let body = format!("flaky-{seq}");
                let res = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(res.as_bytes()).await.unwrap();
            }
        });
        let mut flaky_source = source_flow.clone();
        flaky_source.host = flaky_addr.to_string();
        flaky_source.url = format!("http://{flaky_addr}/repeat-target");
        flaky_source.request_headers = vec![("Host".into(), flaky_addr.to_string())];
        let flaky_ids = replay_batch(
            hub.clone(),
            flaky_source,
            3,
            1,
            0,
            ReplayOverride::default(),
        )
        .await
        .unwrap();
        assert_eq!(flaky_ids.len(), 3);
        let first = hub
            .with_session(|session| session.get(flaky_ids[0]).cloned())
            .unwrap();
        let second = hub
            .with_session(|session| session.get(flaky_ids[1]).cloned())
            .unwrap();
        let third = hub
            .with_session(|session| session.get(flaky_ids[2]).cloned())
            .unwrap();
        assert_eq!(first.status, Some(200));
        assert_eq!(second.status, None);
        assert!(second.error.is_some());
        assert_eq!(third.status, Some(200));
        assert_eq!(String::from_utf8_lossy(&third.response_body), "flaky-3");

        // Concurrent batch replay launches multiple in-flight requests and tags each flow.
        let conc_origin = listen().await.unwrap();
        let conc_addr = conc_origin.local_addr().unwrap();
        tokio::spawn(async move {
            for _ in 0..4 {
                let (mut socket, _) = conc_origin.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let _ = socket.read(&mut buf).await.unwrap();
                    let body = "conc-ok";
                    let res = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    socket.write_all(res.as_bytes()).await.unwrap();
                });
            }
        });
        let mut conc_source = source_flow;
        conc_source.host = conc_addr.to_string();
        conc_source.url = format!("http://{conc_addr}/repeat-conc");
        conc_source.request_headers = vec![("Host".into(), conc_addr.to_string())];
        conc_source.comment = "load-test".into();
        let conc_ids = replay_batch(hub.clone(), conc_source, 4, 3, 0, ReplayOverride::default())
            .await
            .unwrap();
        assert_eq!(conc_ids.len(), 4);
        for (idx, id) in conc_ids.iter().enumerate() {
            let flow = hub
                .with_session(|session| session.get(*id).cloned())
                .unwrap();
            assert_eq!(flow.status, Some(200));
            assert_eq!(
                flow.comment,
                format!("load-test · repeat:{source_id} ({}/4)", idx + 1)
            );
        }
        assert_eq!(throttle_sleep_duration(4096, Some(4096)).as_millis(), 1000);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn https_upstream_mtls_authenticates_with_client_cert_rule() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-mtls-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();

        // Issue a client certificate signed by `origin_ca`
        let mut ca_params = rcgen::CertificateParams::default();
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let client_ca_key = rcgen::KeyPair::generate().unwrap();
        let client_ca = ca_params.self_signed(&client_ca_key).unwrap();
        let client_issuer = rcgen::Issuer::new(ca_params, client_ca_key);
        let client_key = rcgen::KeyPair::generate().unwrap();
        let mut client_params = rcgen::CertificateParams::default();
        client_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
        let client_cert = client_params
            .signed_by(&client_key, &client_issuer)
            .unwrap();

        let client_cert_path = dir.join("client.crt");
        let client_key_path = dir.join("client.key");
        std::fs::write(&client_cert_path, client_cert.pem()).unwrap();
        std::fs::write(&client_key_path, client_key.serialize_pem()).unwrap();

        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        hub.set_rules(&format!(
            "client-cert 127.0.0.1 {} {}",
            client_cert_path.display(),
            client_key_path.display()
        ));
        let hub = Arc::new(hub);

        // Configure an origin server that requires client certificate authentication
        let mut client_roots = RootCertStore::empty();
        client_roots.add(client_ca.der().clone()).unwrap();
        let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(client_roots))
            .build()
            .unwrap();
        let cert_der = rustls::pki_types::CertificateDer::from(origin_leaf.der().to_vec());
        let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(
            rustls::pki_types::PrivatePkcs8KeyDer::from(leaf_key.serialize_der()),
        );
        let mut server_cfg = rustls::ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(vec![cert_der], key_der)
            .unwrap();
        server_cfg.alpn_protocols = vec![b"h2".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_cfg));

        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (sock, _) = origin.accept().await.unwrap();
            let tls = acceptor.accept(sock).await.unwrap();
            let peer_certs = tls.get_ref().1.peer_certificates();
            assert!(peer_certs.is_some_and(|c| !c.is_empty()));
            let service = hyper::service::service_fn(move |_req: Request<Incoming>| async move {
                Ok::<_, std::convert::Infallible>(Response::new(http_body_util::Full::new(
                    Bytes::from_static(b"mtls-verified"),
                )))
            });
            let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(tls), service)
                .await;
        });

        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_one(proxy, hub.clone());
        let (status, body) = h2_get(&ca, proxy_addr, origin_addr, "/mtls-check").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(&body[..], b"mtls-verified");
        let flow = hub
            .with_session(|session| session.flows().into_iter().next())
            .unwrap();
        assert_eq!(flow.rule_hit, "client-cert");
        assert_eq!(
            flow.conn_info.client_cert,
            client_cert_path.display().to_string()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn https_mtls_broken_or_mismatched_client_cert_fails_closed_with_502_and_paths() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir =
            std::env::temp_dir().join(format!("zniff-mtls-failclosed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());

        let cert_a_key = rcgen::KeyPair::generate().unwrap();
        let cert_a = rcgen::CertificateParams::default()
            .self_signed(&cert_a_key)
            .unwrap();
        let wrong_key = rcgen::KeyPair::generate().unwrap();

        let cert_path = dir.join("client-a.crt");
        let wrong_key_path = dir.join("wrong-b.key");
        std::fs::write(&cert_path, cert_a.pem()).unwrap();
        std::fs::write(&wrong_key_path, wrong_key.serialize_pem()).unwrap();

        let hub = Hub::new(ca.clone()).unwrap();
        hub.set_rules(&format!(
            "client-cert 127.0.0.1 {} {}",
            cert_path.display(),
            wrong_key_path.display()
        ));
        let hub = Arc::new(hub);

        // Origin listener that must NEVER receive a fail-open tunnel connection
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        let origin_accepts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let accepts_bg = origin_accepts.clone();
        tokio::spawn(async move {
            if origin.accept().await.is_ok() {
                accepts_bg.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        });

        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());

        // 1. KeyMismatch via `client-cert` rule returns 502 Bad Gateway over H2 without dialing origin
        let (status, body) = h2_get(&ca, proxy_addr, origin_addr, "/secure-api").await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        let body_text = String::from_utf8_lossy(&body);
        assert!(body_text.contains("客户端证书加载失败:"), "{body_text}");
        assert!(
            body_text.contains(&cert_path.display().to_string()),
            "{body_text}"
        );
        assert!(
            body_text.contains(&wrong_key_path.display().to_string()),
            "{body_text}"
        );
        assert!(!body_text.contains("已改为隧道"), "{body_text}");
        assert_eq!(origin_accepts.load(std::sync::atomic::Ordering::SeqCst), 0);

        let flows = hub.with_session(|session| session.flows());
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].status, Some(502));
        assert_eq!(flows[0].rule_hit, "client-cert");
        let err_msg = flows[0].error.as_deref().unwrap_or_default();
        assert!(err_msg.starts_with("客户端证书加载失败:"), "{err_msg}");
        assert!(err_msg.contains("cert="), "{err_msg}");
        assert!(err_msg.contains("key="), "{err_msg}");
        assert!(!err_msg.contains("已改为隧道"), "{err_msg}");

        // 2. Encrypted PEM in `<data_dir>/client-certs/127.0.0.1.pem` also fails closed with 502
        hub.set_rules("");
        let cert_dir = dir.join("client-certs");
        std::fs::create_dir_all(&cert_dir).unwrap();
        let enc_pem = cert_dir.join("127.0.0.1.pem");
        std::fs::write(
            &enc_pem,
            format!(
                "{}-----BEGIN ENCRYPTED PRIVATE KEY-----\nZm9v\n-----END ENCRYPTED PRIVATE KEY-----\n",
                cert_a.pem()
            ),
        )
        .unwrap();

        let (status2, body2) = h2_get(&ca, proxy_addr, origin_addr, "/dir-cert").await;
        assert_eq!(status2, StatusCode::BAD_GATEWAY);
        let body2_text = String::from_utf8_lossy(&body2);
        assert!(body2_text.contains("ENCRYPTED PRIVATE KEY"), "{body2_text}");
        assert_eq!(origin_accepts.load(std::sync::atomic::Ordering::SeqCst), 0);

        let flows = hub.with_session(|session| session.flows());
        assert_eq!(flows.len(), 2);
        assert_eq!(flows[1].status, Some(502));
        assert_eq!(flows[1].rule_hit, "client-cert");

        // 3. Valid directory-discovered client cert records `conn_info.client_cert` and `rule_hit`
        // even when the origin returns HTTP 400.
        std::fs::write(
            &enc_pem,
            format!("{}{}", cert_a.pem(), cert_a_key.serialize_pem()),
        )
        .unwrap();
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub2 = Hub::new(ca.clone()).unwrap();
        hub2.add_trust_anchor(origin_ca.der().clone());
        let hub2 = Arc::new(hub2);

        let cert_der = rustls::pki_types::CertificateDer::from(origin_leaf.der().to_vec());
        let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(
            rustls::pki_types::PrivatePkcs8KeyDer::from(leaf_key.serialize_der()),
        );
        let mut server_cfg = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
            .unwrap();
        server_cfg.alpn_protocols = vec![b"h2".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_cfg));
        let origin400 = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin400_addr = origin400.local_addr().unwrap();
        tokio::spawn(async move {
            let (sock, _) = origin400.accept().await.unwrap();
            let tls = acceptor.accept(sock).await.unwrap();
            let service = hyper::service::service_fn(move |_req: Request<Incoming>| async move {
                let mut res = Response::new(http_body_util::Full::new(Bytes::from_static(
                    b"No required SSL certificate was sent",
                )));
                *res.status_mut() = StatusCode::BAD_REQUEST;
                Ok::<_, std::convert::Infallible>(res)
            });
            let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(tls), service)
                .await;
        });

        let proxy2 = listen().await.unwrap();
        let proxy2_addr = proxy2.local_addr().unwrap();
        serve_one(proxy2, hub2.clone());
        let (status3, _) = h2_get(&ca, proxy2_addr, origin400_addr, "/badssl").await;
        assert_eq!(status3, StatusCode::BAD_REQUEST);
        let flow3 = hub2
            .with_session(|session| session.flows().into_iter().next())
            .unwrap();
        assert_eq!(flow3.rule_hit, "client-cert");
        assert_eq!(flow3.conn_info.client_cert, enc_pem.display().to_string());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn https_h2_map_local_directory_serves_assets_and_falls_back_to_upstream() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-h2-mapdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dist = dir.join("dist");
        std::fs::create_dir_all(dist.join("js")).unwrap();
        std::fs::write(dist.join("index.html"), b"<html>index</html>").unwrap();
        std::fs::write(dist.join("js/app.js"), b"export const v = 1;").unwrap();

        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        hub.set_rules(&format!(
            "map-local 127.0.0.1/assets \"{}\" fallback",
            dist.display()
        ));
        let hub = Arc::new(hub);

        let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
        let (origin_addr, accepts) = spawn_h2_origin(server).await;
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());

        let mut sender = h2_client(&ca, proxy_addr, origin_addr).await;

        // 1. Existing local asset is served from disk without hitting upstream handler
        let (status, body) = h2_send(&mut sender, "/assets/js/app.js?v=42").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(&body[..], b"export const v = 1;");

        // 2. Directory root defaults to index.html
        let (status, body) = h2_send(&mut sender, "/assets/").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(&body[..], b"<html>index</html>");

        // 3. Missing file falls back to upstream when `fallback` is enabled
        let (status, body) = h2_send(&mut sender, "/assets/js/remote-only.js").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(&body[..], b"h2-/assets/js/remote-only.js");
        assert_eq!(accepts.load(std::sync::atomic::Ordering::SeqCst), 1);

        // 4. Directory traversal is blocked with 502 even when `fallback` is enabled
        let (status, _body) = h2_send(&mut sender, "/assets/%2e%2e/ca.crt").await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);

        let flows = hub.with_session(|session| session.flows());
        assert_eq!(flows.len(), 4);
        assert!(
            flows[0].response_headers.iter().any(|(k, v)| {
                k.eq_ignore_ascii_case("content-type") && v.starts_with("application/javascript")
            }),
            "expected application/javascript Content-Type on .js asset"
        );
        assert!(flows[2].comment.contains("map-local(fallback)"));
        assert!(
            flows[3]
                .error
                .as_deref()
                .is_some_and(|e| e.contains("拒绝越界路径"))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn https_h2_records_conn_info_and_marks_subsequent_streams_reused() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-h2-conninfo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        let hub = Arc::new(hub);

        let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
        let (origin_addr, _accepts) = spawn_h2_origin(server).await;
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());

        let mut sender = h2_client(&ca, proxy_addr, origin_addr).await;
        let (status1, _) = h2_send(&mut sender, "/first").await;
        assert_eq!(status1, StatusCode::OK);
        let (status2, _) = h2_send(&mut sender, "/second").await;
        assert_eq!(status2, StatusCode::OK);

        let flows = hub.with_session(|session| session.flows());
        assert_eq!(flows.len(), 2);
        assert!(!flows[0].conn_info.reused);
        assert!(flows[0].conn_info.tcp_ms.is_some());
        assert!(flows[0].conn_info.tls_ms.is_some());
        assert_eq!(flows[0].conn_info.server_addr, origin_addr.to_string());
        assert!(flows[0].conn_info.tls_version.starts_with("TLS 1."));

        assert!(flows[1].conn_info.reused);
        assert!(flows[1].conn_info.tcp_ms.is_none());
        assert!(flows[1].conn_info.tls_ms.is_none());
        assert_eq!(flows[1].conn_info.server_addr, origin_addr.to_string());
        assert_eq!(
            flows[1].conn_info.tls_version,
            flows[0].conn_info.tls_version
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn local_mcp_endpoint_serves_jsonrpc_without_recording_flow() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-mcp-http-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());

        let payload = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            proxy_addr.port(),
            payload.len()
        );
        let mut sock = TcpStream::connect(proxy_addr).await.unwrap();
        sock.write_all(req.as_bytes()).await.unwrap();
        sock.write_all(payload).await.unwrap();
        let mut raw = Vec::new();
        sock.read_to_end(&mut raw).await.unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        let (_, body) = text.split_once("\r\n\r\n").unwrap();
        let json: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(json["id"], 1);
        assert!(json["result"]["tools"].is_array());
        // MCP calls must not pollute the captured flow session
        assert_eq!(hub.with_session(|session| session.len()), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn https_h2_applies_no_cache_and_block_rules() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir =
            std::env::temp_dir().join(format!("zniff-h2-nocache-block-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        hub.set_rules(
            "no-cache 127.0.0.1/api\n\
             block 127.0.0.1/ads\n\
             block 127.0.0.1/ban status=451\n\
             block 127.0.0.1/drop drop\n",
        );
        let hub = Arc::new(hub);

        let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
        let (origin_addr, _accepts) = spawn_h2_origin(server).await;
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());

        let mut sender = h2_client(&ca, proxy_addr, origin_addr).await;

        // 1. no-cache sets Cache-Control on request and response
        let req = Request::builder()
            .method(Method::GET)
            .uri("/api/items")
            .header("If-None-Match", "\"etag-1\"")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let res = sender.send_request(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get("cache-control")
                .and_then(|v| v.to_str().ok()),
            Some("no-cache, no-store, must-revalidate")
        );
        let _ = res.into_body().collect().await.unwrap();

        // 2. block default 403
        let (status_403, body_403) = h2_send(&mut sender, "/ads/pixel").await;
        assert_eq!(status_403, StatusCode::FORBIDDEN);
        assert!(body_403.is_empty());

        // 3. block custom status 451
        let (status_451, body_451) = h2_send(&mut sender, "/ban/check").await;
        assert_eq!(status_451.as_u16(), 451);
        assert!(body_451.is_empty());

        // 4. block drop resets stream / closes connection and records error
        let drop_req = Request::builder()
            .method(Method::GET)
            .uri("/drop")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let drop_res = sender.send_request(drop_req).await;
        assert!(drop_res.is_err());

        let flows = hub.with_session(|session| session.flows());
        assert_eq!(flows.len(), 4);
        assert!(
            !flows[0]
                .request_headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("if-none-match"))
        );
        assert!(flows[0].request_headers.iter().any(|(k, v)| {
            k.eq_ignore_ascii_case("cache-control") && v == "no-cache, no-store, must-revalidate"
        }));
        assert_eq!(flows[1].status, Some(403));
        assert_eq!(flows[2].status, Some(451));
        assert_eq!(flows[3].error.as_deref(), Some("规则阻断 (连接已断开)"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn https_h1_redials_on_the_same_client_connection_when_origin_closes_idle() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-h1-redial-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        let hub = Arc::new(hub);

        let server = origin_tls_config(
            &origin_ca,
            &origin_leaf,
            &leaf_key,
            vec![b"http/1.1".to_vec()],
        );
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        let accepts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let accepts_bg = accepts.clone();
        tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = origin.accept().await else {
                    break;
                };
                let n = accepts_bg.fetch_add(1, Ordering::SeqCst) + 1;
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let mut tls = acceptor.accept(sock).await.unwrap();
                    let mut buf = vec![0u8; 4096];
                    let _ = tls.read(&mut buf).await.unwrap();
                    let body = format!("h1-origin-{n}");
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n{body}",
                        body.len()
                    );
                    tls.write_all(resp.as_bytes()).await.unwrap();
                    tls.shutdown().await.ok();
                });
            }
        });

        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());

        let mut client_tcp = TcpStream::connect(proxy_addr).await.unwrap();
        let connect = format!("CONNECT {origin_addr} HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n");
        client_tcp.write_all(connect.as_bytes()).await.unwrap();
        let mut head = [0u8; 128];
        let n = client_tcp.read(&mut head).await.unwrap();
        assert!(String::from_utf8_lossy(&head[..n]).starts_with("HTTP/1.1 200"));

        let mut roots = RootCertStore::empty();
        roots.add(ca.cert_der()).unwrap();
        let mut client_cfg = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        client_cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        let connector = TlsConnector::from(Arc::new(client_cfg));
        let tls = connector
            .connect(server_name("127.0.0.1").unwrap(), client_tcp)
            .await
            .unwrap();
        let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
            .await
            .unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });

        // First request on the client TLS connection
        let req1 = Request::builder()
            .method(Method::GET)
            .uri("/first")
            .header("Host", origin_addr.to_string())
            .body(Full::new(Bytes::new()))
            .unwrap();
        let res1 = sender.send_request(req1).await.unwrap();
        assert_eq!(res1.status(), StatusCode::OK);
        let body1 = res1.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body1[..], b"h1-origin-1");

        // Wait briefly for the first origin connection to close on EOF
        tokio::time::sleep(Duration::from_millis(40)).await;

        // Second request reuses the SAME client-to-proxy TLS connection; proxy redials origin
        sender.ready().await.unwrap();
        let req2 = Request::builder()
            .method(Method::GET)
            .uri("/second")
            .header("Host", origin_addr.to_string())
            .body(Full::new(Bytes::new()))
            .unwrap();
        let res2 = sender.send_request(req2).await.unwrap();
        assert_eq!(res2.status(), StatusCode::OK);
        let body2 = res2.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body2[..], b"h1-origin-2");
        assert_eq!(accepts.load(Ordering::SeqCst), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn map_remote_http_and_https_preserve_post_body_and_apply_req_res_rules() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir =
            std::env::temp_dir().join(format!("zniff-map-remote-post-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());

        let server = origin_tls_config(
            &origin_ca,
            &origin_leaf,
            &leaf_key,
            vec![b"http/1.1".to_vec()],
        );
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
        let tls_origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tls_addr = tls_origin.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::<(String, Vec<u8>)>::new()));
        let seen_bg = seen.clone();

        tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = tls_origin.accept().await else {
                    break;
                };
                let acceptor = acceptor.clone();
                let seen_bg = seen_bg.clone();
                tokio::spawn(async move {
                    let mut tls = acceptor.accept(sock).await.unwrap();
                    let (head_bytes, mut rest) = read_header_block(&mut tls).await.unwrap();
                    let head = parse_head(&head_bytes).unwrap();
                    let len = content_length(&head.headers).unwrap_or(0);
                    while rest.len() < len {
                        let mut chunk = vec![0u8; len - rest.len()];
                        let n = tls.read(&mut chunk).await.unwrap();
                        if n == 0 {
                            break;
                        }
                        rest.extend_from_slice(&chunk[..n]);
                    }
                    seen_bg
                        .lock()
                        .unwrap()
                        .push((head.start.clone(), rest[..len].to_vec()));

                    // Send 103 Early Hints before final 200 OK JSON
                    tls.write_all(
                        b"HTTP/1.1 103 Early Hints\r\nLink: </app.js>; rel=preload\r\n\r\n",
                    )
                    .await
                    .unwrap();
                    let resp_body = br#"{"ok":false,"src":"origin"}"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        resp_body.len()
                    );
                    tls.write_all(resp.as_bytes()).await.unwrap();
                    tls.write_all(resp_body).await.unwrap();
                    let _ = tls.shutdown().await;
                });
            }
        });

        hub.set_rules(&format!(
            "map-remote api.example.test/v1 https://{tls_addr}/mapped\n\
             query api.example.test set env prod\n\
             json api.example.test req set /msg \"from-rule\"\n\
             header api.example.test res set X-Mapped-Res applied\n\
             cors api.example.test\n\
             json api.example.test res set /ok true\n\
             map-remote 127.0.0.1/v2 https://{tls_addr}/mapped2\n\
             query 127.0.0.1/v2 set env h2\n\
             json 127.0.0.1/v2 req set /msg \"from-h2\"\n\
             header 127.0.0.1/v2 res set X-Mapped-Res h2-ok\n\
             json 127.0.0.1/v2 res set /ok true\n"
        ));
        let hub = Arc::new(hub);

        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());

        // 1. Plain HTTP -> HTTPS map-remote with POST JSON body
        let payload = br#"{"msg":"orig","keep":1}"#;
        let req = format!(
            "POST http://api.example.test/v1/submit HTTP/1.1\r\nHost: api.example.test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        );
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client.write_all(req.as_bytes()).await.unwrap();
        client.write_all(payload).await.unwrap();
        let mut raw_resp = Vec::new();
        client.read_to_end(&mut raw_resp).await.unwrap();
        let text = String::from_utf8_lossy(&raw_resp);
        assert!(text.contains("HTTP/1.1 200 OK"), "{text}");
        assert!(
            text.to_ascii_lowercase().contains("x-mapped-res: applied"),
            "{text}"
        );
        assert!(
            text.to_ascii_lowercase()
                .contains("access-control-allow-origin: *"),
            "{text}"
        );
        assert!(text.ends_with(r#"{"ok":true,"src":"origin"}"#), "{text}");

        // 2. HTTPS -> HTTPS cross-port map-remote (`fetch_remote`) with POST JSON body
        let dummy_server =
            origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
        let (dummy_addr, _) = spawn_h2_origin(dummy_server).await;
        let mut h2_sender = h2_client(&ca, proxy_addr, dummy_addr).await;
        let h2_req = Request::builder()
            .method(Method::POST)
            .uri("/v2/items")
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from_static(br#"{"msg":"h2-orig"}"#)))
            .unwrap();
        let h2_res = h2_sender.send_request(h2_req).await.unwrap();
        assert_eq!(h2_res.status(), StatusCode::OK);
        assert_eq!(
            h2_res
                .headers()
                .get("x-mapped-res")
                .and_then(|v| v.to_str().ok()),
            Some("h2-ok")
        );
        let h2_body = h2_res.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&h2_body[..], br#"{"ok":true,"src":"origin"}"#);

        let captured = seen.lock().unwrap().clone();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].0, "POST /mapped/submit?env=prod HTTP/1.1");
        assert_eq!(captured[0].1, br#"{"msg":"from-rule","keep":1}"#);
        assert_eq!(captured[1].0, "POST /mapped2/items?env=h2 HTTP/1.1");
        assert_eq!(captured[1].1, br#"{"msg":"from-h2"}"#);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn head_and_204_without_content_length_do_not_hang_keep_alive_and_replay_skips_103() {
        let dir = std::env::temp_dir().join(format!("zniff-no-body-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());

        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        let accepts = Arc::new(AtomicU64::new(0));
        let accepts_bg = accepts.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = origin.accept().await {
                let conn_num = accepts_bg.fetch_add(1, Ordering::SeqCst) + 1;
                tokio::spawn(async move {
                    loop {
                        let Ok((head_bytes, _)) = read_header_block(&mut sock).await else {
                            break;
                        };
                        let Ok(head) = parse_head(&head_bytes) else {
                            break;
                        };
                        if head.start.starts_with("HEAD /doc ") {
                            // Non-zero Content-Length on HEAD, zero body bytes, keep socket open!
                            sock.write_all(
                                b"HTTP/1.1 200 OK\r\nContent-Length: 9999\r\nKeep-Alive: timeout=5\r\n\r\n",
                            )
                            .await
                            .unwrap();
                        } else if head.start.starts_with("GET /empty ") {
                            // 204 with NO Content-Length header, keep socket open!
                            sock.write_all(b"HTTP/1.1 204 No Content\r\n\r\n")
                                .await
                                .unwrap();
                        } else if head.start.starts_with("GET /hint ") {
                            sock.write_all(
                                b"HTTP/1.1 103 Early Hints\r\nLink: </s.css>; rel=preload\r\n\r\n\
                                  HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nhinted",
                            )
                            .await
                            .unwrap();
                            break;
                        } else {
                            let body = format!("conn-{conn_num}");
                            let reply = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                                body.len()
                            );
                            sock.write_all(reply.as_bytes()).await.unwrap();
                        }
                    }
                });
            }
        });

        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());

        // 1. HEAD with Content-Length: 9999 returns immediately without waiting for 9999 bytes
        let mut s1 = TcpStream::connect(proxy_addr).await.unwrap();
        s1.write_all(
            format!("HEAD http://{origin_addr}/doc HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
        let (h1, r1) = timeout(Duration::from_secs(2), read_header_block(&mut s1))
            .await
            .expect("HEAD must not hang")
            .unwrap();
        assert!(String::from_utf8_lossy(&h1).starts_with("HTTP/1.1 200 OK"));
        assert!(r1.is_empty());

        // 2. 204 without Content-Length returns immediately and parks origin socket for reuse
        let mut s2 = TcpStream::connect(proxy_addr).await.unwrap();
        s2.write_all(
            format!("GET http://{origin_addr}/empty HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
        let (h2, r2) = timeout(Duration::from_secs(2), read_header_block(&mut s2))
            .await
            .expect("204 without Content-Length must not hang")
            .unwrap();
        assert!(String::from_utf8_lossy(&h2).starts_with("HTTP/1.1 204"));
        assert!(r2.is_empty());

        // 3. Follow-up GET reuses the same parked origin TCP socket (conn-1)
        tokio::time::sleep(Duration::from_millis(20)).await;
        let mut s3 = TcpStream::connect(proxy_addr).await.unwrap();
        s3.write_all(
            format!("GET http://{origin_addr}/after HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
        let (_, mut r3) = timeout(Duration::from_secs(2), read_header_block(&mut s3))
            .await
            .unwrap()
            .unwrap();
        while r3.len() < 6 {
            let mut tmp = [0u8; 16];
            let n = s3.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            r3.extend_from_slice(&tmp[..n]);
        }
        assert_eq!(&r3[..], b"conn-1");
        assert_eq!(accepts.load(Ordering::SeqCst), 1);

        // 4. replay_once skips 103 Early Hints and records 200 + final body
        let replay_id = timeout(
            Duration::from_secs(2),
            replay_once(
                hub.clone(),
                None,
                ReplayOverride {
                    method: Some("GET".into()),
                    url: Some(format!("http://{origin_addr}/hint")),
                    headers: Some(Vec::new()),
                    body: Some(Vec::new()),
                },
            ),
        )
        .await
        .expect("replay must not hang")
        .unwrap();
        let replayed = hub.with_session(|s| s.get(replay_id).cloned()).unwrap();
        assert_eq!(replayed.status, Some(200));
        assert_eq!(replayed.response_body, b"hinted");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn switching_upstream_invalidates_pools_and_https_h1_preserves_header_case() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-pool-inval-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        let hub = Arc::new(hub);

        // Part A: H2 origin + dynamic upstream CONNECT proxy
        let h2_server =
            origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
        let (h2_origin_addr, _origin_accepts) = spawn_h2_origin(h2_server).await;

        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        let upstream_hits = Arc::new(AtomicU64::new(0));
        let upstream_hits_bg = upstream_hits.clone();
        tokio::spawn(async move {
            while let Ok((mut client_sock, _)) = upstream.accept().await {
                upstream_hits_bg.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let (head_bytes, _) = read_header_block(&mut client_sock).await.unwrap();
                    let head = parse_head(&head_bytes).unwrap();
                    let target = head.start.split_whitespace().nth(1).unwrap();
                    let mut out = TcpStream::connect(target).await.unwrap();
                    client_sock
                        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                        .await
                        .unwrap();
                    let _ = tokio::io::copy_bidirectional(&mut client_sock, &mut out).await;
                });
            }
        });

        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());

        let mut sender = h2_client(&ca, proxy_addr, h2_origin_addr).await;
        let (st1, _) = h2_send(&mut sender, "/direct").await;
        assert_eq!(st1, StatusCode::OK);
        assert_eq!(upstream_hits.load(Ordering::SeqCst), 0);

        // Dynamically switch upstream proxy while `sender` (`H2Lane`) and `h2_pool` are live!
        hub.set_upstream(&upstream_addr.to_string());
        let (st2, _) = h2_send(&mut sender, "/via-upstream").await;
        assert_eq!(st2, StatusCode::OK);
        assert_eq!(
            upstream_hits.load(Ordering::SeqCst),
            1,
            "live H2 lane must redial through the new upstream proxy immediately"
        );
        hub.set_upstream("");

        // Part B: HTTPS HTTP/1.1 preserves non-Title-Case response headers (e.g. ETag, x-custom-ID)
        let h1_server = origin_tls_config(
            &origin_ca,
            &origin_leaf,
            &leaf_key,
            vec![b"http/1.1".to_vec()],
        );
        let h1_acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(h1_server));
        let h1_origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let h1_origin_addr = h1_origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (sock, _) = h1_origin.accept().await.unwrap();
            let mut tls = h1_acceptor.accept(sock).await.unwrap();
            let _ = read_header_block(&mut tls).await.unwrap();
            tls.write_all(
                b"HTTP/1.1 200 OK\r\nETag: \"v1\"\r\nx-custom-ID: 42\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi",
            )
            .await
            .unwrap();
            let _ = tls.shutdown().await;
        });

        let mut tls_client =
            connect_mitm(proxy_addr, h1_origin_addr, &ca, vec![b"http/1.1".to_vec()]).await;
        tls_client
            .write_all(b"GET /case HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let (raw_head, _) = read_header_block(&mut tls_client).await.unwrap();
        let head_str = String::from_utf8_lossy(&raw_head);
        assert!(head_str.contains("ETag: \"v1\"\r\n"), "{head_str}");
        assert!(head_str.contains("x-custom-ID: 42\r\n"), "{head_str}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn head_local_routes_preserve_method_and_omit_wire_body() {
        let dir = std::env::temp_dir().join(format!("zniff-head-local-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let (share_id, _) = hub.begin_share(Duration::from_secs(60), "");

        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());

        for path in [
            "/zniff".to_string(),
            "/zniff-ca.crt".to_string(),
            format!("/s/{share_id}"),
        ] {
            let mut sock = TcpStream::connect(proxy_addr).await.unwrap();
            let req = format!(
                "HEAD {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
                proxy_addr.port()
            );
            sock.write_all(req.as_bytes()).await.unwrap();
            let mut raw = Vec::new();
            timeout(Duration::from_secs(2), sock.read_to_end(&mut raw))
                .await
                .unwrap()
                .unwrap();
            let split = raw
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .expect("header terminator")
                + 4;
            let head = parse_head(&raw[..split]).unwrap();
            assert!(
                head.start.starts_with("HTTP/1.1 200 OK"),
                "unexpected status for {path}: {}",
                head.start
            );
            let declared_len = content_length(&head.headers).expect("Content-Length present");
            assert!(
                declared_len > 0,
                "Content-Length should reflect resource size for {path}"
            );
            assert!(
                raw[split..].is_empty(),
                "HEAD {path} must not write response body bytes on the wire (got {} bytes)",
                raw[split..].len()
            );
        }

        let flows = hub.with_session(|s| s.flows());
        assert_eq!(flows.len(), 2);
        assert_eq!(flows[0].method, "HEAD");
        assert_eq!(flows[0].path, "/zniff");
        assert!(flows[0].response_body.is_empty());
        assert_eq!(flows[1].method, "HEAD");
        assert_eq!(flows[1].path, "/zniff-ca.crt");
        assert!(flows[1].response_body.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn replay_batch_and_compose_support_pure_h2_origin_without_pooling() {
        use std::io::Write as _;

        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-h2-replay-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        let hub = Arc::new(hub);

        // Pure h2-only TLS origin returning gzipped JSON body
        let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        let hits = Arc::new(AtomicU64::new(0));
        let hits_bg = hits.clone();
        tokio::spawn(async move {
            while let Ok((sock, _)) = origin.accept().await {
                let acceptor = acceptor.clone();
                let hits_bg = hits_bg.clone();
                tokio::spawn(async move {
                    let Ok(tls) = acceptor.accept(sock).await else {
                        return;
                    };
                    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(b"h2".as_slice()));
                    let service = hyper::service::service_fn(move |req: Request<Incoming>| {
                        let hits_bg = hits_bg.clone();
                        async move {
                            let n = hits_bg.fetch_add(1, Ordering::SeqCst) + 1;
                            let method = req.method().to_string();
                            let pq = req
                                .uri()
                                .path_and_query()
                                .map(|p| p.as_str().to_string())
                                .unwrap_or_default();
                            let req_bytes = req.into_body().collect().await.unwrap().to_bytes();
                            let plain = serde_json::json!({
                                "hit": n,
                                "method": method,
                                "path": pq,
                                "req": String::from_utf8_lossy(&req_bytes),
                            })
                            .to_string();
                            let mut gz = flate2::write::GzEncoder::new(
                                Vec::new(),
                                flate2::Compression::fast(),
                            );
                            gz.write_all(plain.as_bytes()).unwrap();
                            let compressed = gz.finish().unwrap();
                            let resp = Response::builder()
                                .status(StatusCode::CREATED)
                                .header("content-type", "application/json")
                                .header("content-encoding", "gzip")
                                .header("x-origin-proto", "h2")
                                .body(Full::new(Bytes::from(compressed)))
                                .unwrap();
                            Ok::<_, std::convert::Infallible>(resp)
                        }
                    });
                    let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                        .serve_connection(TokioIo::new(tls), service)
                        .await;
                });
            }
        });

        // 1. Standalone composed HTTPS POST (`source.is_none()`) against pure-h2 origin
        let (tx_comp, rx_comp) = tokio::sync::oneshot::channel();
        hub.send_composed_later(
            ReplayOverride {
                method: Some("POST".into()),
                url: Some(format!(
                    "https://127.0.0.1:{}/v2/compose",
                    origin_addr.port()
                )),
                headers: Some(vec![("Content-Type".into(), "application/json".into())]),
                body: Some(br#"{"compose":1}"#.to_vec()),
            },
            move |res| {
                let _ = tx_comp.send(res);
            },
        );
        let composed_id = timeout(Duration::from_secs(5), rx_comp)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let composed = hub
            .with_session(|s| s.hydrated(composed_id))
            .expect("composed flow");
        assert_eq!(composed.protocol, Protocol::Http2);
        assert_eq!(composed.status, Some(201));
        assert!(composed.response_at.is_some());
        assert!(composed.finished_at.is_some());
        assert!(!composed.conn_info.server_addr.is_empty());
        assert!(!composed.conn_info.tls_version.is_empty());
        assert!(composed.conn_info.tcp_ms.is_some());
        assert!(composed.conn_info.tls_ms.is_some());
        let composed_body = String::from_utf8(composed.response_body.clone()).unwrap();
        assert!(
            composed_body.contains(r#""path":"/v2/compose""#),
            "{composed_body}"
        );
        assert!(
            composed_body.contains(r#"\"compose\":1"#),
            "{composed_body}"
        );

        // 2. Single replay of that Protocol::Http2 flow (`replay_later`)
        let (tx_rep, rx_rep) = tokio::sync::oneshot::channel();
        hub.replay_later(composed_id, ReplayOverride::default(), move |res| {
            let _ = tx_rep.send(res);
        });
        let replay_id = timeout(Duration::from_secs(5), rx_rep)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let replayed = hub
            .with_session(|s| s.hydrated(replay_id))
            .expect("replayed flow");
        assert_eq!(replayed.protocol, Protocol::Http2);
        assert_eq!(replayed.status, Some(201));
        assert!(
            replayed
                .response_headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("x-origin-proto") && v == "h2")
        );

        // 3. Batch replay of that Protocol::Http2 flow (`replay_batch_later`)
        let (tx_batch, rx_batch) = tokio::sync::oneshot::channel();
        hub.replay_batch_later(
            composed_id,
            2,
            2,
            0,
            ReplayOverride::default(),
            move |res| {
                let _ = tx_batch.send(res);
            },
        );
        let batch_ids = timeout(Duration::from_secs(5), rx_batch)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(batch_ids.len(), 2);
        for bid in batch_ids {
            let bflow = hub.with_session(|s| s.hydrated(bid)).unwrap();
            assert_eq!(bflow.protocol, Protocol::Http2);
            assert_eq!(bflow.status, Some(201));
        }

        // Verify single-use H2 handshakes NEVER entered `h2_pool`
        assert!(
            hub.h2_pool.lock().unwrap().is_empty(),
            "replay H2 connections must not enter h2_pool"
        );
        assert_eq!(hits.load(Ordering::SeqCst), 4);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn chunked_plain_http_op_replace_json_and_overflow_passthrough_roundtrip() {
        use std::io::Write as _;

        let dir = std::env::temp_dir().join(format!("zniff-chunked-rules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());

        type CapturedReq = (Vec<(String, String)>, Vec<u8>);
        let seen_reqs: Arc<Mutex<Vec<CapturedReq>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_bg = seen_reqs.clone();
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = origin.accept().await {
                let seen_bg = seen_bg.clone();
                tokio::spawn(async move {
                    let Ok((head_bytes, rest)) = read_header_block(&mut sock).await else {
                        return;
                    };
                    let head = parse_head(&head_bytes).unwrap();
                    let mut reader = Prefixed {
                        prefix: rest,
                        pos: 0,
                        inner: &mut sock,
                    };
                    let body = if let Some(len) = content_length(&head.headers) {
                        read_exact(&mut reader, len).await.unwrap()
                    } else if is_chunked(&head.headers) {
                        let mut out = Vec::new();
                        loop {
                            let line = read_line(&mut reader).await.unwrap();
                            let n = usize::from_str_radix(
                                String::from_utf8_lossy(&line)
                                    .trim()
                                    .split(';')
                                    .next()
                                    .unwrap(),
                                16,
                            )
                            .unwrap();
                            if n == 0 {
                                let _ = read_line(&mut reader).await.unwrap();
                                break;
                            }
                            out.extend(read_exact(&mut reader, n).await.unwrap());
                            let _ = read_exact(&mut reader, 2).await.unwrap();
                        }
                        out
                    } else {
                        Vec::new()
                    };
                    seen_bg.lock().unwrap().push((head.headers.clone(), body));

                    if head.start.contains("/big") {
                        // Stream > 1MB chunked response back
                        sock.write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                        )
                        .await
                        .unwrap();
                        let chunk_payload = vec![b'z'; 64 * 1024];
                        let header_line = format!("{:x}\r\n", chunk_payload.len());
                        // 17 * 64KB > 1MB (16 * 64KB)
                        for _ in 0..17 {
                            sock.write_all(header_line.as_bytes()).await.unwrap();
                            sock.write_all(&chunk_payload).await.unwrap();
                            sock.write_all(b"\r\n").await.unwrap();
                        }
                        sock.write_all(b"0\r\n\r\n").await.unwrap();
                    } else {
                        // Return gzipped chunked JSON response
                        let resp_plain = br#"{"status":"draft","ok":false}"#;
                        let mut gz =
                            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
                        gz.write_all(resp_plain).unwrap();
                        let gz_bytes = gz.finish().unwrap();
                        let mid = gz_bytes.len() / 2;
                        let part1 = &gz_bytes[..mid];
                        let part2 = &gz_bytes[mid..];
                        sock.write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Encoding: gzip\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                        )
                        .await
                        .unwrap();
                        sock.write_all(format!("{:x}\r\n", part1.len()).as_bytes())
                            .await
                            .unwrap();
                        sock.write_all(part1).await.unwrap();
                        sock.write_all(b"\r\n").await.unwrap();
                        sock.write_all(format!("{:x}\r\n", part2.len()).as_bytes())
                            .await
                            .unwrap();
                        sock.write_all(part2).await.unwrap();
                        sock.write_all(b"\r\n0\r\n\r\n").await.unwrap();
                    }
                });
            }
        });

        hub.set_rules(
            "json 127.0.0.1/graphql req set /variables/qty 99 op=MutateCart\n\
             replace 127.0.0.1/graphql req old-sku -> new-sku\n\
             replace 127.0.0.1/graphql res draft -> published\n\
             json 127.0.0.1/graphql res set /ok true\n\
             json 127.0.0.1/big req set /touch 1\n\
             json 127.0.0.1/big res set /touch 1\n",
        );

        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_proxy(proxy, hub.clone());

        // Case 1: Chunked <= 1MB GraphQL request matching `op=MutateCart` + `replace` + `json` on both req & gzipped chunked res
        let gql_part1 = br#"{"operationName":"MutateCart","variables":{"sku":"old-sku","qty":1},"#;
        let gql_part2 = br#""query":"mutation MutateCart { updateCart }"}"#;
        let mut client1 = TcpStream::connect(proxy_addr).await.unwrap();
        client1
            .write_all(
                format!(
                    "POST http://{origin_addr}/graphql HTTP/1.1\r\nHost: {origin_addr}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        client1
            .write_all(format!("{:x}\r\n", gql_part1.len()).as_bytes())
            .await
            .unwrap();
        client1.write_all(gql_part1).await.unwrap();
        client1.write_all(b"\r\n").await.unwrap();
        client1
            .write_all(format!("{:x}\r\n", gql_part2.len()).as_bytes())
            .await
            .unwrap();
        client1.write_all(gql_part2).await.unwrap();
        client1.write_all(b"\r\n0\r\n\r\n").await.unwrap();

        let mut raw_resp1 = Vec::new();
        client1.read_to_end(&mut raw_resp1).await.unwrap();
        let split1 = raw_resp1.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let resp_head1 = parse_head(&raw_resp1[..split1]).unwrap();
        assert!(header_value(&resp_head1.headers, "transfer-encoding").is_none());
        assert!(header_value(&resp_head1.headers, "content-encoding").is_none());
        let resp_json1: serde_json::Value = serde_json::from_slice(&raw_resp1[split1..]).unwrap();
        assert_eq!(resp_json1["status"], "published");
        assert_eq!(resp_json1["ok"], true);
        assert_eq!(
            content_length(&resp_head1.headers),
            Some(raw_resp1[split1..].len())
        );

        let (seen_headers1, seen_body1) = seen_reqs.lock().unwrap()[0].clone();
        assert!(header_value(&seen_headers1, "transfer-encoding").is_none());
        assert_eq!(content_length(&seen_headers1), Some(seen_body1.len()));
        let seen_json1: serde_json::Value = serde_json::from_slice(&seen_body1).unwrap();
        assert_eq!(seen_json1["variables"]["sku"], "new-sku");
        assert_eq!(seen_json1["variables"]["qty"], 99);

        // Case 2: Chunked > 1MB request & > 1MB response stream unmodified and record `[rule skipped: body > 1MB]`
        let mut client2 = TcpStream::connect(proxy_addr).await.unwrap();
        client2
            .write_all(
                format!(
                    "POST http://{origin_addr}/big HTTP/1.1\r\nHost: {origin_addr}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let req_chunk = vec![b'x'; 64 * 1024];
        let req_hdr = format!("{:x}\r\n", req_chunk.len());
        for _ in 0..17 {
            client2.write_all(req_hdr.as_bytes()).await.unwrap();
            client2.write_all(&req_chunk).await.unwrap();
            client2.write_all(b"\r\n").await.unwrap();
        }
        client2.write_all(b"0\r\n\r\n").await.unwrap();

        let mut raw_resp2 = Vec::new();
        client2.read_to_end(&mut raw_resp2).await.unwrap();
        // Verify full > 1MB response arrived intact on client
        assert!(raw_resp2.len() > 17 * 64 * 1024);

        // Verify full > 1MB request arrived intact on origin
        let (_, seen_body2) = seen_reqs.lock().unwrap()[1].clone();
        assert_eq!(seen_body2.len(), 17 * 64 * 1024);

        let big_flow = hub.with_session(|s| s.get(2).cloned()).unwrap();
        assert!(
            big_flow.comment.contains("[rule skipped: body > 1MB]"),
            "expected >1MB skip note, got: {}",
            big_flow.comment
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn https_h2_streaming_without_content_length_applies_replace_and_json_rules() {
        struct ChunksBody(std::collections::VecDeque<Bytes>);
        impl Body for ChunksBody {
            type Data = Bytes;
            type Error = std::convert::Infallible;
            fn poll_frame(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
                Poll::Ready(self.0.pop_front().map(|b| Ok(Frame::data(b))))
            }
        }

        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-h2-stream-json-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        hub.set_rules(
            "replace 127.0.0.1/stream req alpha -> beta\n\
             json 127.0.0.1/stream req set /patched true\n\
             replace 127.0.0.1/stream res pending -> done\n\
             json 127.0.0.1/stream res set /count 7\n",
        );
        let hub = Arc::new(hub);

        let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (sock, _) = origin.accept().await.unwrap();
            let tls = acceptor.accept(sock).await.unwrap();
            let service = hyper::service::service_fn(move |req: Request<Incoming>| async move {
                let req_bytes = req.into_body().collect().await.unwrap().to_bytes();
                let req_json: serde_json::Value = serde_json::from_slice(&req_bytes).unwrap();
                assert_eq!(req_json["tag"], "beta");
                assert_eq!(req_json["patched"], true);
                let res_chunks = std::collections::VecDeque::from([
                    Bytes::from_static(br#"{"state":"pending","#),
                    Bytes::from_static(br#""count":0}"#),
                ]);
                Ok::<_, std::convert::Infallible>(
                    Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", "application/json")
                        .body(ChunksBody(res_chunks))
                        .unwrap(),
                )
            });
            let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(tls), service)
                .await;
        });

        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        serve_one(proxy, hub.clone());

        let tls = connect_mitm(proxy_addr, origin_addr, &ca, vec![b"h2".to_vec()]).await;
        let (mut sender, connection) =
            hyper::client::conn::http2::Builder::new(TokioExecutor::new())
                .handshake::<_, ChunksBody>(TokioIo::new(tls))
                .await
                .unwrap();
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let req_chunks = std::collections::VecDeque::from([
            Bytes::from_static(br#"{"tag":"alpha","#),
            Bytes::from_static(br#""patched":false}"#),
        ]);
        let req = Request::builder()
            .method(Method::POST)
            .uri("/stream")
            .header("content-type", "application/json")
            .body(ChunksBody(req_chunks))
            .unwrap();
        let res = timeout(Duration::from_secs(5), sender.send_request(req))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let res_bytes = res.into_body().collect().await.unwrap().to_bytes();
        let res_json: serde_json::Value = serde_json::from_slice(&res_bytes).unwrap();
        assert_eq!(res_json["state"], "done");
        assert_eq!(res_json["count"], 7);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_upstream_supports_optional_scheme_and_basic_auth() {
        assert_eq!(
            parse_upstream("127.0.0.1:7890"),
            Some(Upstream {
                host: "127.0.0.1".into(),
                port: 7890,
                auth_header: None,
            })
        );
        let expected_auth = format!(
            "Basic {}",
            crate::session::base64_encode(b"alice:p@ss:word")
        );
        assert_eq!(
            parse_upstream("http://alice:p%40ss:word@127.0.0.1:7890/"),
            Some(Upstream {
                host: "127.0.0.1".into(),
                port: 7890,
                auth_header: Some(expected_auth.clone()),
            })
        );
        assert_eq!(
            parse_upstream("alice:p@ss:word@[::1]:7890"),
            Some(Upstream {
                host: "::1".into(),
                port: 7890,
                auth_header: Some(expected_auth),
            })
        );
        assert_eq!(parse_upstream("socks5://127.0.0.1:1080"), None);
        assert_eq!(parse_upstream(""), None);
    }

    #[tokio::test]
    async fn proxy_auth_enforces_407_exempts_local_routes_and_chains_authenticated_upstream() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("zniff-proxy-auth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());

        // Plain HTTP origin that asserts client Proxy-Authorization was stripped.
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        let seen_headers: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_clone = seen_headers.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = origin.accept().await {
                let Ok((head_bytes, _)) = read_header_block(&mut sock).await else {
                    break;
                };
                seen_clone
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&head_bytes).into_owned());
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
            }
        });

        // HTTPS origin for CONNECT chaining test.
        let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
        let tls_server = origin_tls_config(
            &origin_ca,
            &origin_leaf,
            &leaf_key,
            vec![b"http/1.1".to_vec()],
        );
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls_server));
        let tls_origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tls_origin_addr = tls_origin.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((sock, _)) = tls_origin.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    if let Ok(mut tls) = acceptor.accept(sock).await {
                        let _ = read_header_block(&mut tls).await;
                        let _ = tls
                            .write_all(
                                b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\ntls-ok",
                            )
                            .await;
                    }
                });
            }
        });

        // Upstream proxy requiring Basic auth `upuser:up@pass`.
        let mut upstream_hub = Hub::new(ca.clone()).unwrap();
        upstream_hub.add_trust_anchor(origin_ca.der().clone());
        upstream_hub.set_proxy_auth("upuser:up@pass");
        let upstream_hub = Arc::new(upstream_hub);
        let upstream_capture =
            Capture::spawn(upstream_hub.clone(), "127.0.0.1:0".parse().unwrap()).unwrap();
        let upstream_addr = upstream_capture.addr;

        // Downstream proxy requiring Basic auth `client:s3cr3t` and chaining to authenticated upstream.
        let mut hub = Hub::new(ca.clone()).unwrap();
        hub.add_trust_anchor(origin_ca.der().clone());
        hub.set_proxy_auth("client:s3cr3t");
        hub.set_upstream(&format!("http://upuser:up%40pass@{upstream_addr}"));
        let hub = Arc::new(hub);
        let capture = Capture::spawn(hub.clone(), "127.0.0.1:0".parse().unwrap()).unwrap();
        let proxy_addr = capture.addr;

        // 1. Missing Proxy-Authorization -> 407, not recorded.
        {
            let mut s = TcpStream::connect(proxy_addr).await.unwrap();
            let req = format!(
                "GET http://{origin_addr}/denied HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
            );
            s.write_all(req.as_bytes()).await.unwrap();
            let mut resp = Vec::new();
            s.read_to_end(&mut resp).await.unwrap();
            let text = String::from_utf8_lossy(&resp);
            assert!(text.starts_with("HTTP/1.1 407"), "{text}");
            assert!(
                text.to_ascii_lowercase()
                    .contains("proxy-authenticate: basic realm=\"zniff\""),
                "{text}"
            );
        }

        // 2. Wrong Proxy-Authorization on CONNECT -> 407.
        {
            let wrong = crate::session::base64_encode(b"client:wrong");
            let mut s = TcpStream::connect(proxy_addr).await.unwrap();
            let req = format!(
                "CONNECT {tls_origin_addr} HTTP/1.1\r\nHost: {tls_origin_addr}\r\nProxy-Authorization: Basic {wrong}\r\n\r\n"
            );
            s.write_all(req.as_bytes()).await.unwrap();
            let mut resp = Vec::new();
            s.read_to_end(&mut resp).await.unwrap();
            let text = String::from_utf8_lossy(&resp);
            assert!(text.starts_with("HTTP/1.1 407"), "{text}");
        }
        assert_eq!(hub.with_session(|s| s.len()), 0);

        // 3. Local routes (/zniff-ca.crt and /zniff) remain accessible without Proxy-Authorization.
        for route in ["/zniff-ca.crt", "/zniff"] {
            let mut s = TcpStream::connect(proxy_addr).await.unwrap();
            let req =
                format!("GET {route} HTTP/1.1\r\nHost: {proxy_addr}\r\nConnection: close\r\n\r\n");
            s.write_all(req.as_bytes()).await.unwrap();
            let mut resp = Vec::new();
            s.read_to_end(&mut resp).await.unwrap();
            let text = String::from_utf8_lossy(&resp);
            assert!(text.starts_with("HTTP/1.1 200"), "{route} -> {text}");
        }

        // 4. Valid client Proxy-Authorization on plain HTTP -> 200 via authenticated upstream,
        //    and neither client nor upstream Proxy-Authorization reaches origin or session headers.
        let valid_client = crate::session::base64_encode(b"client:s3cr3t");
        {
            let mut s = TcpStream::connect(proxy_addr).await.unwrap();
            let req = format!(
                "GET http://{origin_addr}/authed HTTP/1.1\r\nHost: {origin_addr}\r\nProxy-Authorization: Basic {valid_client}\r\nConnection: close\r\n\r\n"
            );
            s.write_all(req.as_bytes()).await.unwrap();
            let mut resp = Vec::new();
            s.read_to_end(&mut resp).await.unwrap();
            let text = String::from_utf8_lossy(&resp);
            assert!(text.starts_with("HTTP/1.1 200"), "{text}");
            assert!(text.ends_with("\r\n\r\nok"), "{text}");
        }
        let origin_req_head = seen_headers.lock().unwrap()[0].to_ascii_lowercase();
        assert!(
            !origin_req_head.contains("proxy-authorization"),
            "origin must never receive Proxy-Authorization: {origin_req_head}"
        );
        let recorded = hub.with_session(|s| s.get(1).cloned()).unwrap();
        assert!(
            recorded
                .request_headers
                .iter()
                .all(|(k, _)| !k.eq_ignore_ascii_case("proxy-authorization"))
        );

        // 5. Replay of plain HTTP flow also authenticates to upstream proxy.
        let replayed_id = replay_once(hub.clone(), Some(recorded), ReplayOverride::default())
            .await
            .unwrap();
        let replayed = hub.with_session(|s| s.get(replayed_id).cloned()).unwrap();
        assert_eq!(replayed.status, Some(200));

        // 6. Valid client Proxy-Authorization on HTTPS CONNECT -> 200 tunnel + MITM via authenticated upstream.
        {
            let mut s = TcpStream::connect(proxy_addr).await.unwrap();
            let connect_req = format!(
                "CONNECT {tls_origin_addr} HTTP/1.1\r\nHost: {tls_origin_addr}\r\nProxy-Authorization: Basic {valid_client}\r\n\r\n"
            );
            s.write_all(connect_req.as_bytes()).await.unwrap();
            let (head, rest) = read_header_block(&mut s).await.unwrap();
            let text = String::from_utf8_lossy(&head);
            assert!(text.starts_with("HTTP/1.1 200"), "{text}");
            assert!(rest.is_empty());

            let mut roots = RootCertStore::empty();
            roots.add(ca.cert_der()).unwrap();
            let mut client_cfg = ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();
            client_cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
            let connector = TlsConnector::from(Arc::new(client_cfg));
            let mut tls = connector
                .connect(server_name("127.0.0.1").unwrap(), s)
                .await
                .unwrap();
            let inner_req = format!(
                "GET /secure HTTP/1.1\r\nHost: {tls_origin_addr}\r\nConnection: close\r\n\r\n"
            );
            tls.write_all(inner_req.as_bytes()).await.unwrap();
            let mut resp = Vec::new();
            tls.read_to_end(&mut resp).await.unwrap();
            let resp_text = String::from_utf8_lossy(&resp);
            assert!(resp_text.starts_with("HTTP/1.1 200"), "{resp_text}");
            assert!(resp_text.ends_with("\r\n\r\ntls-ok"), "{resp_text}");
        }

        capture.stop();
        upstream_capture.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
