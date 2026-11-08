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
use http_body_util::{BodyExt, Full};
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
use crate::session::{Direction, FlowDraft, Protocol, Session};
use crate::ws::{self, FrameFeed};

const IO_TIMEOUT: Duration = Duration::from_secs(20);
/// Drop a cached origin HTTP/2 connection before a typical server idle timeout.
const H2_IDLE: Duration = Duration::from_secs(45);
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
    let mut guard = local_app_cache()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    guard.insert(port, (app, Instant::now()));
}

/// Identifies the local process name owning `peer_port` on loopback connections.
pub fn resolve_local_process_name(peer_port: u16) -> Option<String> {
    if peer_port == 0 {
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(name) = resolve_local_process_linux(peer_port) {
            return Some(name);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("lsof")
            .args(["-nP", "-iTCP", "-sTCP:ESTABLISHED", "-Fpcn"])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let needle = format!(":{peer_port}->");
        let mut current_cmd = String::new();
        for line in stdout.lines() {
            if let Some(cmd) = line.strip_prefix('c') {
                current_cmd = cmd.trim().to_string();
            } else if let Some(net) = line.strip_prefix('n')
                && net.contains(&needle)
                && !current_cmd.is_empty()
            {
                return Some(clean_process_name(&current_cmd));
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        let out = std::process::Command::new("netstat")
            .args(["-ano", "-p", "TCP"])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let needle = format!(":{peer_port} ");
        for line in stdout.lines() {
            if line.contains(&needle)
                && let Some(pid_str) = line.split_whitespace().last()
                && let Ok(pid) = pid_str.parse::<u32>()
                && pid > 0
            {
                let tlist = std::process::Command::new("tasklist")
                    .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
                    .output()
                    .ok()?;
                let t_out = String::from_utf8_lossy(&tlist.stdout);
                if let Some(first_col) = t_out.lines().next().and_then(|l| l.split(',').next()) {
                    let name = first_col.trim().trim_matches('"');
                    if !name.is_empty() {
                        return Some(clean_process_name(name));
                    }
                }
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn resolve_local_process_linux(peer_port: u16) -> Option<String> {
    let port_hex = format!(":{peer_port:04X}");
    let mut target_inode = None;
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
            if local_addr.ends_with(&port_hex) {
                let Some(inode) = cols.nth(7) else {
                    continue;
                };
                if inode != "0" {
                    target_inode = Some(inode.to_string());
                    break;
                }
            }
        }
        if target_inode.is_some() {
            break;
        }
    }
    let inode = target_inode?;
    let socket_tag = format!("socket:[{inode}]");
    let entries = std::fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        let fname = entry.file_name();
        let Some(pid_str) = fname.to_str() else {
            continue;
        };
        if !pid_str.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let fd_dir = entry.path().join("fd");
        let Ok(fds) = std::fs::read_dir(fd_dir) else {
            continue;
        };
        for fd in fds.flatten() {
            if let Ok(link) = std::fs::read_link(fd.path())
                && link.to_string_lossy() == socket_tag
            {
                let comm = std::fs::read_to_string(entry.path().join("comm")).ok()?;
                let name = clean_process_name(comm.trim());
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
    }
    None
}

fn clean_process_name(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches(".exe");
    trimmed.replace("\\x20", " ")
}

type H2Sender = hyper::client::conn::http2::SendRequest<RecordingBody<Incoming>>;

struct PooledH2 {
    sender: H2Sender,
    used: Instant,
    generation: u64,
}

pub struct Hub {
    session: Mutex<Session>,
    generation: AtomicU64,
    pub ca: Arc<Ca>,
    roots: RootCertStore,
    bypass: Mutex<BypassRules>,
    rules: Mutex<Vec<Rule>>,
    /// Session-only. Off skips every rule without rewriting the text.
    apply_rules: AtomicBool,
    /// Session-only. Off drops `break` and `break-res` and keeps the other rules.
    apply_breaks: AtomicBool,
    /// Off by default. On records an origin TLS probe that fell back to a tunnel.
    show_tunnel_failures: AtomicBool,
    upstream: Mutex<Option<Upstream>>,
    /// Idle plaintext origin sockets, used only on the proxy runtime.
    pool: tokio::sync::Mutex<HashMap<(String, u16), Vec<TcpStream>>>,
    /// Origin HTTP/2 senders. Each browser connection keeps its own lane.
    h2_pool: Mutex<HashMap<(String, u16), PooledH2>>,
    h2_generation: AtomicU64,
    /// Test-only. The next send on a live browser connection dials again.
    #[cfg(test)]
    refresh_h2: AtomicBool,
    break_queue: Mutex<std::collections::VecDeque<BreakSlot>>,
    /// One live HTML share. A new call replaces the id.
    share: Mutex<Option<crate::share::LiveShare>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    pub host: String,
    pub port: u16,
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
        Ok(Self {
            session: Mutex::new(Session::default()),
            generation: AtomicU64::new(1),
            ca,
            roots,
            bypass: Mutex::new(BypassRules::default()),
            rules: Mutex::new(Vec::new()),
            apply_rules: AtomicBool::new(true),
            apply_breaks: AtomicBool::new(true),
            show_tunnel_failures: AtomicBool::new(false),
            upstream: Mutex::new(None),
            pool: tokio::sync::Mutex::new(HashMap::new()),
            h2_pool: Mutex::new(HashMap::new()),
            h2_generation: AtomicU64::new(1),
            #[cfg(test)]
            refresh_h2: AtomicBool::new(false),
            break_queue: Mutex::new(std::collections::VecDeque::new()),
            share: Mutex::new(None),
        })
    }

    /// Replaces any previous link. The id is unguessable and dies at `ttl`.
    pub fn begin_share(
        &self,
        ttl: std::time::Duration,
        filter: &str,
    ) -> (String, std::time::SystemTime) {
        let id = crate::share::new_id();
        let expires = std::time::SystemTime::now() + ttl;
        *self.share.lock().unwrap_or_else(|err| err.into_inner()) = Some(crate::share::LiveShare {
            id: id.clone(),
            expires,
            filter: filter.to_string(),
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
        drop(slot);
        let (flows, rev) = self
            .with_session(|session| (session.flows_matching_unhydrated(&filter), session.rev()));
        Some(crate::share::document_since(
            &flows, expires, &filter, rev, since,
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
        drop(slot);
        let flow = self.with_session(|session| session.hydrated_matching(flow_id, &filter))?;
        Some(crate::share::flow_body_document(&flow))
    }

    pub fn set_bypass(&self, text: &str) {
        *self.bypass.lock().unwrap_or_else(|err| err.into_inner()) = BypassRules::parse(text);
    }

    pub fn set_rules(&self, text: &str) {
        *self.rules.lock().unwrap_or_else(|err| err.into_inner()) = rules::parse_rules(text);
    }

    pub fn set_rules_enabled(&self, enabled: bool) {
        self.apply_rules.store(enabled, Ordering::Relaxed);
    }

    pub fn set_breaks_enabled(&self, enabled: bool) {
        self.apply_breaks.store(enabled, Ordering::Relaxed);
    }

    pub fn set_show_tunnel_failures(&self, show: bool) {
        self.show_tunnel_failures.store(show, Ordering::Relaxed);
    }

    pub fn set_upstream(&self, text: &str) {
        *self.upstream.lock().unwrap_or_else(|err| err.into_inner()) = parse_upstream(text);
    }

    fn upstream(&self) -> Option<Upstream> {
        self.upstream
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    fn skips_tls(&self, host: &str) -> bool {
        self.bypass
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .skips(host)
    }

    fn plan(&self, host: &str, path: &str) -> rules::Effect {
        if !self.apply_rules.load(Ordering::Relaxed) {
            return rules::Effect::default();
        }
        let rules = self
            .rules
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        let effect = rules::effect(&rules, host, path);
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

    fn note_rules(&self, id: u64, host: &str, path: &str) {
        if !self.apply_rules.load(Ordering::Relaxed) {
            return;
        }
        let rules = self
            .rules
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        let mut hit = rules::hit_summary(&rules, host, path);
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

    fn cached_h2(&self, key: &(String, u16)) -> Option<(H2Sender, u64)> {
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
        Some((entry.sender.clone(), entry.generation))
    }

    /// Returns the generation stored with this sender. Replacing a key does
    /// not wipe the other origins. A full pool drops every entry only when
    /// the key is new.
    fn store_h2(&self, key: (String, u16), sender: H2Sender) -> u64 {
        let generation = self.h2_generation.fetch_add(1, Ordering::Relaxed);
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
            },
        );
        generation
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
        self.pool
            .lock()
            .await
            .get_mut(&key)
            .and_then(|slots| slots.pop())
    }

    async fn park_idle(&self, key: (String, u16), stream: TcpStream) {
        let mut pool = self.pool.lock().await;
        if pool.len() >= 32 && !pool.contains_key(&key) {
            pool.clear();
        }
        let slots = pool.entry(key).or_default();
        if slots.len() >= 4 {
            slots.remove(0);
        }
        slots.push(stream);
    }

    #[cfg(test)]
    pub fn add_trust_anchor(&mut self, der: CertificateDer<'static>) {
        let _ = self.roots.add(der);
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

    fn open(&self, draft: FlowDraft) -> u64 {
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

    fn set_error(&self, id: u64, error: impl Into<String>) {
        self.with_session(|session| session.set_error(id, error));
        self.touch();
    }

    fn append_body(&self, id: u64, direction: Direction, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        self.with_session(|session| session.append_body(id, direction, data));
        self.touch();
    }

    fn add_transfer(&self, id: u64, direction: Direction, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.with_session(|session| session.add_transfer(id, direction, bytes));
        self.touch();
    }

    pub fn mark_finished(&self, id: u64) {
        self.with_session(|session| session.mark_finished(id));
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
    pub fn export_har_matching(&self, filter: &str) -> serde_json::Value {
        let flows = self.with_session(|session| session.flows_matching(filter));
        crate::har::export_har(&flows)
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
            });
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
            } else {
                store_cached_local_app(port, String::new());
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
        Some(LocalRoute::Favicon) => {
            return write_untracked(&mut socket, 204, "No Content", "image/x-icon", b"").await;
        }
        None if self_target => {
            return write_untracked(
                &mut socket,
                404,
                "Not Found",
                "text/plain; charset=utf-8",
                b"Not Found",
            )
            .await;
        }
        None => {}
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
    if offers_h2 && let Some((mut sender, generation)) = hub.cached_h2(&(sni_host.clone(), port)) {
        let ready = timeout(Duration::from_secs(2), sender.ready()).await;
        if matches!(ready, Ok(Ok(()))) {
            return intercept_h2_pooled(
                client, hello.raw, sender, generation, &sni_host, port, hub,
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
    match timeout(IO_TIMEOUT, connect_origin(&sni_host, port, &alpn, &hub)).await {
        Ok(Ok((origin, selected))) => {
            intercept(client, hello.raw, origin, &sni_host, port, selected, hub).await
        }
        Ok(Err(err)) => {
            // The bytes still go through as a tunnel, so the site keeps
            // working and there is no decrypted request. The row is optional
            // only when the fallback tunnel actually carries traffic.
            let reason = origin_tls_reason(&err);
            log::debug!("源站 TLS 失败，改隧道 {sni_host}:{port}: {reason} ({err:#})");
            let recorded = hub.show_tunnel_failures.load(Ordering::Relaxed);
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
) -> Result<(tokio_rustls::client::TlsStream<TcpStream>, Vec<u8>)> {
    let tcp = timeout(IO_TIMEOUT, dial(host, port, hub, true))
        .await
        .context("连接源站超时")??;
    tcp.set_nodelay(true).ok();
    let mut config = ClientConfig::builder()
        .with_root_certificates(hub.roots.clone())
        .with_no_client_auth();
    config.alpn_protocols = alpn.to_vec();
    let connector = TlsConnector::from(Arc::new(config));
    let name = server_name(host)?;
    let tls = connector.connect(name, tcp).await.context("源站 TLS")?;
    let selected = tls
        .get_ref()
        .1
        .alpn_protocol()
        .map(|proto| proto.to_vec())
        .unwrap_or_else(|| b"http/1.1".to_vec());
    Ok((tls, selected))
}

async fn intercept<S>(
    client: S,
    client_hello: Vec<u8>,
    origin: tokio_rustls::client::TlsStream<TcpStream>,
    host: &str,
    port: u16,
    selected: Vec<u8>,
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
        serve_h2(client_tls, origin, host, port, hub).await
    } else {
        serve_h1(client_tls, origin, host, port, hub).await
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

async fn serve_h1<C, O>(client: C, origin: O, host: String, port: u16, hub: Arc<Hub>) -> Result<()>
where
    C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    O: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (sender, connection) = hyper::client::conn::http1::Builder::new()
        .preserve_header_case(true)
        .title_case_headers(true)
        .handshake::<_, RecordingBody<Incoming>>(TokioIo::new(origin))
        .await
        .context("上游 HTTP/1.1")?;
    tokio::spawn(async move {
        if let Err(err) = connection.with_upgrades().await {
            log::debug!("上游 HTTP/1.1 连接结束: {err}");
        }
    });
    // hyper 1.12's HTTP/1 SendRequest is not Clone. One task owns it.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<H1Job>();
    tokio::spawn(async move {
        let mut sender = sender;
        while let Some(job) = rx.recv().await {
            let result = async {
                sender.ready().await.context("上游 HTTP/1.1 未就绪")?;
                sender.send_request(job.req).await.context("上游 HTTP/1.1")
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
        .title_case_headers(true)
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
    hub: &Arc<Hub>,
) -> Result<(H2Sender, u64)>
where
    O: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (sender, connection) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
        .handshake::<_, RecordingBody<Incoming>>(TokioIo::new(origin))
        .await
        .context("上游 HTTP/2")?;
    let generation = hub.store_h2((host.to_string(), port), sender.clone());
    let hub = hub.clone();
    let key = (host.to_string(), port);
    tokio::spawn(async move {
        if let Err(err) = connection.await {
            log::debug!("上游 HTTP/2 连接结束: {err}");
        }
        hub.drop_h2_generation(&key, generation);
    });
    Ok((sender, generation))
}

/// Dials the origin offering only `h2`. A server that picks another protocol
/// is an error: this path does not fall back to HTTP/1.1.
async fn dial_h2(host: &str, port: u16, hub: &Arc<Hub>) -> Result<(H2Sender, u64)> {
    let (origin, selected) = timeout(
        IO_TIMEOUT,
        connect_origin(host, port, &[b"h2".to_vec()], hub),
    )
    .await
    .context("连接源站超时")??;
    if selected != b"h2" {
        anyhow::bail!("源站没有选择 HTTP/2");
    }
    handshake_h2(origin, host, port, hub).await
}

async fn serve_h2<C, O>(client: C, origin: O, host: String, port: u16, hub: Arc<Hub>) -> Result<()>
where
    C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    O: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (sender, generation) = handshake_h2(origin, &host, port, &hub).await?;
    serve_h2_sender(
        client,
        sender,
        generation,
        host,
        port,
        hub,
        TokioExecutor::new(),
    )
    .await
}

async fn intercept_h2_pooled<S>(
    client: S,
    client_hello: Vec<u8>,
    sender: H2Sender,
    generation: u64,
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
        host.to_string(),
        port,
        hub,
        TokioExecutor::new(),
    )
    .await
}

async fn serve_h2_sender<C>(
    client: C,
    sender: H2Sender,
    generation: u64,
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

/// Short reason for a probe that never reached ServerHello. `{err}` alone is
/// only the `context("源站 TLS")` label, so the list would repeat that label.
fn origin_tls_reason(err: &anyhow::Error) -> String {
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
    let (method, host, port, path) = request_target(head)?;
    let id = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: method.clone(),
        host: host.clone(),
        path: path.clone(),
        url: format!("http://{}{path}", display_authority(&host, port, "http")),
        request_headers: head.headers.clone(),
    });
    let mut host = host;
    let mut port = port;
    let mut path = path;
    let mut headers = head.headers.clone();
    let mut body_override: Option<Vec<u8>> = None;
    let mut request_taken = false;
    let mut strip_request_encoding = false;
    let websocket = header_is_websocket(&headers);
    let plan = if websocket {
        rules::Effect::default()
    } else {
        hub.note_rules(id, &host, &path);
        hub.plan(&host, &path)
    };
    if !plan.delay.is_zero() {
        tokio::time::sleep(plan.delay).await;
    }
    if !plan.request_headers.is_empty() {
        rules::apply_header_ops(&mut headers, &plan.request_headers);
        hub.replace_request(id, headers.clone(), Vec::new());
    }
    let response_ops = plan.response_headers.clone();
    let cors = plan.cors;
    let throttle_up_bps = plan.throttle_up_bps;
    let throttle_down_bps = plan.throttle_down_bps;
    let break_response = matches!(plan.action, Some(Action::BreakResponse));
    if cors && method.eq_ignore_ascii_case("OPTIONS") && plan.action.is_none() {
        let _ = drain_request(client, &headers, &hub, id).await;
        let mut resp_headers = Vec::new();
        rules::ensure_cors(&mut resp_headers);
        rules::apply_header_ops(&mut resp_headers, &response_ops);
        write_static(client, &hub, id, 204, "No Content", &resp_headers, &[]).await?;
        return Ok(());
    }
    match &plan.action {
        Some(Action::MapLocal { file, status }) => {
            let _ = drain_request(client, &headers, &hub, id).await;
            return write_mapped_file(
                client,
                &hub,
                id,
                file.as_deref(),
                *status,
                &response_ops,
                cors,
                throttle_down_bps,
            )
            .await;
        }
        Some(Action::BreakRequest) => {
            let decision = hub.wait_break(id, BreakSide::Request).await;
            if decision.drop {
                hub.set_error(id, "断点丢弃");
                let _ = drain_request(client, &headers, &hub, id).await;
                write_static(client, &hub, id, 204, "No Content", &[], &[]).await?;
                return Ok(());
            }
            if let Some(next) = decision.headers {
                hub.replace_request(id, next.clone(), Vec::new());
                headers = next;
            }
            body_override = decision.body;
        }
        Some(Action::MapRemote { url }) => {
            if let Some((scheme, next_host, next_port, next_path)) = parse_http_url(url) {
                host = next_host;
                port = next_port;
                path = next_path;
                hub.retarget(
                    id,
                    host.clone(),
                    path.clone(),
                    format!(
                        "{scheme}://{}{path}",
                        display_authority(&host, port, scheme)
                    ),
                );
                if scheme == "https" {
                    let _ = drain_request(client, &headers, &hub, id).await;
                    let body = body_override.unwrap_or_default();
                    return exchange_tls(
                        client,
                        &hub,
                        id,
                        &method,
                        &host,
                        port,
                        &path,
                        &headers,
                        &body,
                        throttle_up_bps,
                        throttle_down_bps,
                    )
                    .await;
                }
            }
        }
        Some(Action::BreakResponse) | None => {}
    }
    if let Some(next) = rules::apply_query_ops(&path, &plan.query_ops) {
        path = next;
        hub.retarget(
            id,
            host.clone(),
            path.clone(),
            format!("http://{}{path}", display_authority(&host, port, "http")),
        );
    }
    if let Some(next) = rules::replace_target(&path, &plan.request_replaces) {
        path = next;
        hub.retarget(
            id,
            host.clone(),
            path.clone(),
            format!("http://{}{path}", display_authority(&host, port, "http")),
        );
    }
    if body_override.is_none()
        && !plan.request_replaces.is_empty()
        && let Some(len) = content_length(&head.headers)
        && len <= crate::session::BODY_CAP
    {
        let raw = read_exact(client, len).await?;
        request_taken = true;
        let encoding = header_value(&head.headers, "content-encoding");
        if let Some(next) = replaced_body(&raw, encoding.as_deref(), &plan.request_replaces) {
            strip_request_encoding = true;
            body_override = Some(next);
        } else {
            body_override = Some(raw);
        }
    }
    let pool_key = pool_key(&host, port, &hub);
    let mut origin = match hub.take_idle(pool_key.clone()).await {
        Some(stream) => stream,
        None => timeout(IO_TIMEOUT, dial(&host, port, &hub, false))
            .await
            .context("连接源站超时")
            .inspect_err(|err| {
                hub.set_error(id, explain_flow_error(err));
            })??,
    };
    origin.set_nodelay(true).ok();
    let target = if hub.upstream().is_some() {
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
            || (body_override.is_some() && name.eq_ignore_ascii_case("content-length"))
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
    if !has_connection {
        outbound.push_str("Connection: keep-alive\r\n");
    }
    if let Some(body) = &body_override {
        outbound.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    outbound.push_str("\r\n");
    if origin.write_all(outbound.as_bytes()).await.is_err() {
        origin = timeout(IO_TIMEOUT, dial(&host, port, &hub, false))
            .await
            .context("连接源站超时")??;
        origin.set_nodelay(true).ok();
        origin.write_all(outbound.as_bytes()).await?;
    }
    let encoding = header_value(&headers, "content-encoding");
    if let Some(body) = &body_override {
        write_all_throttled(&mut origin, body, throttle_up_bps).await?;
        hub.replace_request(id, headers.clone(), body.clone());
        if !request_taken {
            let _ = discard_request(client, &head.headers).await;
        }
    } else {
        copy_message_body(
            client,
            &mut origin,
            &head.headers,
            &hub,
            id,
            Direction::Request,
            encoding,
            throttle_up_bps,
        )
        .await?;
    }
    let (mut resp_bytes, resp_rest) = read_header_block(&mut origin).await?;
    if websocket {
        resp_bytes = strip_ws_extension(&resp_bytes);
    }
    let resp = parse_head(&resp_bytes)?;
    let (mut status, reason) = status_and_reason(&resp.start);
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
        let copied = copy_with_messages(client, &mut origin, &hub, id, &resp_rest).await;
        hub.mark_finished(id);
        copied?;
        return Ok(());
    }
    let mut origin = Prefixed {
        prefix: resp_rest,
        pos: 0,
        inner: origin,
    };
    if !websocket
        && !break_response
        && !plan.response_replaces.is_empty()
        && let Some(len) = content_length(&resp_headers)
        && len <= crate::session::BODY_CAP
    {
        let raw = read_exact(&mut origin, len).await?;
        let encoding = header_value(&resp_headers, "content-encoding");
        if let Some(next) = replaced_body(&raw, encoding.as_deref(), &plan.response_replaces) {
            resp_headers.retain(|(name, _)| {
                !name.eq_ignore_ascii_case("content-encoding")
                    && !name.eq_ignore_ascii_case("transfer-encoding")
            });
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
        let head = render_response_head(status, &reason, &resp_headers);
        client.write_all(head.as_bytes()).await?;
        write_all_throttled(client, &raw, throttle_down_bps).await?;
        note_decoded(&hub, id, Direction::Response, &raw, encoding.as_deref());
        hub.mark_finished(id);
        return Ok(());
    }
    if break_response {
        match read_capped_body(&mut origin, &resp_headers, crate::session::BODY_CAP).await {
            Ok(Some(body)) => {
                let encoding = header_value(&resp_headers, "content-encoding");
                note_decoded(&hub, id, Direction::Response, &body, encoding.as_deref());
                let decision = hub.wait_break(id, BreakSide::Response).await;
                if decision.drop {
                    hub.set_error(id, "断点丢弃");
                    hub.replace_response(id, 204, Vec::new(), Vec::new());
                    write_bytes(client, 204, "No Content", &[], &[], None).await?;
                    hub.mark_finished(id);
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
        client,
        &resp_headers,
        &hub,
        id,
        Direction::Response,
        encoding,
        throttle_down_bps,
    )
    .await?;
    hub.mark_finished(id);
    let response_close = header_value(&resp.headers, "connection")
        .is_some_and(|value| value.eq_ignore_ascii_case("close"));
    if reusable
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
    if let Some(upstream) = hub.upstream() {
        let mut stream = TcpStream::connect((upstream.host.as_str(), upstream.port))
            .await
            .context("连接上游代理")?;
        stream.set_nodelay(true).ok();
        if tunnel {
            let req = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n\r\n");
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
        return Ok(stream);
    }
    let stream = TcpStream::connect((host, port)).await?;
    stream.set_nodelay(true).ok();
    Ok(stream)
}

fn parse_upstream(text: &str) -> Option<Upstream> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let (host, port) = split_host_port(text, 80).ok()?;
    Some(Upstream { host, port })
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
    )
    .await?;
    Ok(())
}

async fn discard_request<S>(client: &mut S, headers: &[(String, String)]) -> Result<()>
where
    S: AsyncRead + Unpin,
{
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
    write_all_throttled(client, body, throttle_bps).await?;
    let mut recorded = headers.to_vec();
    if !recorded
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-length"))
    {
        recorded.push(("Content-Length".into(), body.len().to_string()));
    }
    hub.set_response(id, status, recorded, None);
    hub.append_body(id, Direction::Response, body);
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
    throttle_up_bps: Option<u64>,
    throttle_down_bps: Option<u64>,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let (mut origin, _) = connect_origin(host, port, &[b"http/1.1".to_vec()], hub).await?;
    let mut outbound = format!("{method} {path} HTTP/1.1\r\n");
    let mut has_host = false;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("proxy-connection")
            || name.eq_ignore_ascii_case("content-length")
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
    let (resp_bytes, resp_rest) = read_header_block(&mut origin).await?;
    client.write_all(&resp_bytes).await?;
    let resp = parse_head(&resp_bytes)?;
    let status = resp
        .start
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    hub.set_response(id, status, resp.headers.clone(), None);
    let mut origin = Prefixed {
        prefix: resp_rest,
        pos: 0,
        inner: origin,
    };
    let _ = copy_message_body(
        &mut origin,
        client,
        &resp.headers,
        hub,
        id,
        Direction::Response,
        header_value(&resp.headers, "content-encoding"),
        throttle_down_bps,
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
    Favicon,
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
    if !method.eq_ignore_ascii_case("GET") && !method.eq_ignore_ascii_case("HEAD") {
        return None;
    }
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
    let zh = crate::share::prefers_zh(header_value(&head.headers, "accept-language").as_deref());
    match kind {
        crate::share::SharePath::Data { since } => {
            let Some(json) = hub.share_document_since(id, since) else {
                let body = crate::share::expired_page(zh);
                return write_untracked(
                    socket,
                    404,
                    "Not Found",
                    "text/html; charset=utf-8",
                    body.as_bytes(),
                )
                .await;
            };
            write_untracked(
                socket,
                200,
                "OK",
                "application/json; charset=utf-8",
                json.as_bytes(),
            )
            .await
        }
        crate::share::SharePath::FlowBody(flow_id) => {
            let Some(json) = hub.share_flow_body(id, flow_id) else {
                return write_untracked(
                    socket,
                    404,
                    "Not Found",
                    "application/json; charset=utf-8",
                    b"{}",
                )
                .await;
            };
            write_untracked(
                socket,
                200,
                "OK",
                "application/json; charset=utf-8",
                json.as_bytes(),
            )
            .await
        }
        crate::share::SharePath::Page => {
            let Some(json) = hub.share_document_since(id, None) else {
                let body = crate::share::expired_page(zh);
                return write_untracked(
                    socket,
                    404,
                    "Not Found",
                    "text/html; charset=utf-8",
                    body.as_bytes(),
                )
                .await;
            };
            let live = format!("/s/{id}/flows.json");
            let body = crate::share::page(&json, &live, zh);
            write_untracked(
                socket,
                200,
                "OK",
                "text/html; charset=utf-8",
                body.as_bytes(),
            )
            .await
        }
    }
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
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    );
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(body).await?;
    Ok(())
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
  code { font-family: ui-monospace, Menlo, monospace; }
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
  code { font-family: ui-monospace, Menlo, monospace; }
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
    let host = header_value(&head.headers, "host").unwrap_or_else(|| "zniff".into());
    let id = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: "GET".into(),
        host: host.clone(),
        path: "/zniff".into(),
        url: format!("http://{host}/zniff"),
        request_headers: head.headers.clone(),
    });
    let body =
        install_page_for(header_value(&head.headers, "accept-language").as_deref()).as_bytes();
    write_static(
        socket,
        hub,
        id,
        200,
        "OK",
        &[("Content-Type".into(), "text/html; charset=utf-8".into())],
        body,
    )
    .await
}

async fn serve_ca_cert<S>(socket: &mut S, hub: &Hub, head: &Head) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let pem = std::fs::read(hub.ca.cert_path()).unwrap_or_default();
    let host = header_value(&head.headers, "host").unwrap_or_else(|| "zniff".into());
    let id = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: "GET".into(),
        host: host.clone(),
        path: "/zniff-ca.crt".into(),
        url: format!("http://{host}/zniff-ca.crt"),
        request_headers: head.headers.clone(),
    });
    write_static(
        socket,
        hub,
        id,
        200,
        "OK",
        &[("Content-Type".into(), "application/x-x509-ca-cert".into())],
        &pem,
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

async fn replay_once(
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
    hub.replace_request(id, headers.clone(), body.clone());
    let sent = if scheme == "https" {
        replay_tls(&hub, id, &method, &host, port, &path, &headers, &body).await
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
    let mut origin = timeout(IO_TIMEOUT, dial(host, port, hub, false))
        .await
        .context("连接源站超时")??;
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
) -> Result<()> {
    let (origin, _) = connect_origin(host, port, &[b"http/1.1".to_vec()], hub).await?;
    write_replay(
        hub, id, origin, method, host, port, "https", path, headers, body,
    )
    .await
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
    outbound.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    origin.write_all(outbound.as_bytes()).await?;
    origin.write_all(body).await?;
    if !body.is_empty() {
        hub.add_transfer(id, Direction::Request, body.len() as u64);
    }
    let (resp_bytes, resp_rest) = read_header_block(&mut origin).await?;
    let resp = parse_head(&resp_bytes)?;
    let status = resp
        .start
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
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
    )
    .await?;
    hub.mark_finished(id);
    Ok(())
}

fn file_type(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" | "log" => "text/plain",
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
) -> Result<bool>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
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
            hub.add_transfer(id, direction, n as u64);
            let decoded = decoder.push(&buf[..n]);
            hub.append_body(id, direction, &decoded);
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
            hub.add_transfer(id, direction, n as u64);
            let decoded = decoder.push(&buf[..n]);
            hub.append_body(id, direction, &decoded);
        }
        false
    } else {
        true
    };
    let tail = decoder.finish();
    hub.append_body(id, direction, &tail);
    if let Some(error) = decoder.error() {
        hub.set_encoding_error(id, direction, error.to_string());
    }
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
            hub.add_transfer(id, direction, n as u64);
            let decoded = decoder.push(&buf[..n]);
            hub.append_body(id, direction, &decoded);
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
) -> Result<Option<Vec<u8>>>
where
    R: AsyncRead + Unpin,
{
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
    let mut buf = Vec::new();
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

struct Prefixed<S> {
    prefix: Vec<u8>,
    pos: usize,
    inner: S,
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

    fn record_bytes(&mut self, bytes: &[u8]) {
        let Some(record) = self.record.as_mut() else {
            return;
        };
        record
            .hub
            .add_transfer(record.id, record.direction, bytes.len() as u64);
        let decoded = record.decoder.push(bytes);
        record
            .hub
            .append_body(record.id, record.direction, &decoded);
        if let Some(error) = record.decoder.error() {
            record
                .hub
                .set_encoding_error(record.id, record.direction, error.to_string());
        }
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
    reply: tokio::sync::oneshot::Sender<Result<Response<Incoming>>>,
}

#[derive(Clone)]
struct H1Client {
    tx: tokio::sync::mpsc::UnboundedSender<H1Job>,
}

trait HttpSender: Send {
    fn send(
        &mut self,
        req: Request<RecordingBody<Incoming>>,
    ) -> impl std::future::Future<Output = Result<Response<Incoming>>> + Send;
}

impl HttpSender for H1Client {
    fn send(
        &mut self,
        req: Request<RecordingBody<Incoming>>,
    ) -> impl std::future::Future<Output = Result<Response<Incoming>>> + Send {
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
    ) -> impl std::future::Future<Output = Result<Response<Incoming>>> + Send {
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
                let (mut sender, generation) = {
                    let mut guard = lane.lock().await;
                    let stale = guard.broken
                        || guard.sender.is_closed()
                        || guard.last_ok.elapsed() > H2_IDLE;
                    #[cfg(test)]
                    let stale = stale || hub.take_h2_refresh();
                    if stale {
                        let old = guard.generation;
                        hub.drop_h2_generation(&key, old);
                        match dial_h2(&host, port, &hub).await {
                            Ok((next, generation)) => {
                                guard.sender = next;
                                guard.generation = generation;
                                guard.broken = false;
                                guard.last_ok = Instant::now();
                            }
                            Err(err) => {
                                guard.broken = true;
                                return Err(err.context("HTTP/2 上游"));
                            }
                        }
                    }
                    (guard.sender.clone(), guard.generation)
                };
                // `try_send_request` gives the request back only when it never
                // left. Retrying after the body was written would repeat a POST.
                // A later failure only retires this generation, so another
                // stream's replacement stays in the pool.
                match sender.try_send_request(req).await {
                    Ok(response) => {
                        let mut guard = lane.lock().await;
                        if guard.generation == generation {
                            guard.last_ok = Instant::now();
                            guard.broken = false;
                        }
                        drop(guard);
                        hub.touch_h2(&key, generation);
                        return Ok(response);
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
    let mut body_override: Option<Vec<u8>> = None;
    let mut strip_request_encoding = false;
    let mut headers_replaced = false;
    let mut break_response = false;
    let mut response_ops = Vec::new();
    let mut response_replaces = Vec::new();
    let mut cors = false;
    let mut throttle_up_bps = None;
    let mut throttle_down_bps = None;
    if !websocket {
        hub.note_rules(id, &host, &path);
        let plan = hub.plan(&host, &path);
        if !plan.delay.is_zero() {
            tokio::time::sleep(plan.delay).await;
        }
        response_ops = plan.response_headers.clone();
        response_replaces = plan.response_replaces.clone();
        cors = plan.cors;
        throttle_up_bps = plan.throttle_up_bps;
        throttle_down_bps = plan.throttle_down_bps;
        if !plan.request_headers.is_empty() {
            let mut pairs = header_pairs(&parts.headers);
            rules::apply_header_ops(&mut pairs, &plan.request_headers);
            replace_header_map(&mut parts.headers, &pairs);
            headers_replaced = true;
        }
        if cors && parts.method == Method::OPTIONS && plan.action.is_none() {
            drain_incoming(incoming.take().expect("request body")).await?;
            let mut resp_headers = Vec::new();
            rules::ensure_cors(&mut resp_headers);
            rules::apply_header_ops(&mut resp_headers, &response_ops);
            resp_headers.push(("Content-Length".into(), "0".into()));
            return synthetic_headers(&hub, id, StatusCode::NO_CONTENT, resp_headers, Vec::new());
        }
        match &plan.action {
            Some(Action::MapLocal { file, status }) => {
                record_request_bytes(
                    &hub,
                    id,
                    incoming.take().expect("request body"),
                    encoding.as_deref(),
                )
                .await?;
                return mapped_file_response(
                    &hub,
                    id,
                    file.as_deref(),
                    *status,
                    &response_ops,
                    cors,
                )
                .map(|res| res.map(|b| b.with_throttle(throttle_down_bps)));
            }
            Some(Action::BreakRequest) => {
                let decision = hub.wait_break(id, BreakSide::Request).await;
                if decision.drop {
                    hub.set_error(id, "断点丢弃");
                    drain_incoming(incoming.take().expect("request body")).await?;
                    return empty_status(&hub, id, StatusCode::NO_CONTENT);
                }
                if let Some(next) = decision.headers {
                    replace_header_map(&mut parts.headers, &next);
                    headers_replaced = true;
                }
                body_override = decision.body;
            }
            Some(Action::MapRemote { url }) => {
                if let Some((next_scheme, next_host, next_port, next_path)) = parse_http_url(url) {
                    let same = next_scheme == scheme && next_host == host && next_port == port;
                    host = next_host;
                    port = next_port;
                    path = next_path;
                    scheme = next_scheme.to_string();
                    let authority = display_authority(&host, port, &scheme);
                    hub.retarget(
                        id,
                        host.clone(),
                        path.clone(),
                        format!("{scheme}://{authority}{path}"),
                    );
                    if !same {
                        let raw = record_request_bytes(
                            &hub,
                            id,
                            incoming.take().expect("request body"),
                            encoding.as_deref(),
                        )
                        .await?;
                        return fetch_remote(
                            &hub,
                            id,
                            parts.method.clone(),
                            &scheme,
                            &host,
                            port,
                            &path,
                            &parts.headers,
                            raw,
                        )
                        .await
                        .map(|res| res.map(|b| b.with_throttle(throttle_down_bps)));
                    }
                }
            }
            Some(Action::BreakResponse) => break_response = true,
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
        if body_override.is_none() && !plan.request_replaces.is_empty() {
            let known = parts
                .headers
                .get(http::header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|text| text.parse::<usize>().ok());
            if known.is_some_and(|len| len <= crate::session::BODY_CAP) {
                let raw = record_request_bytes(
                    &hub,
                    id,
                    incoming.take().expect("request body"),
                    encoding.as_deref(),
                )
                .await?;
                if let Some(next) = replaced_body(&raw, encoding.as_deref(), &plan.request_replaces)
                {
                    strip_request_encoding = true;
                    body_override = Some(next);
                } else {
                    body_override = Some(raw.to_vec());
                }
            }
        }
    }
    let outbound_body = if let Some(bytes) = body_override {
        parts.headers.remove(http::header::CONTENT_LENGTH);
        if strip_request_encoding {
            parts.headers.remove(http::header::CONTENT_ENCODING);
        }
        if let Ok(value) = http::HeaderValue::from_str(&bytes.len().to_string()) {
            parts.headers.insert(http::header::CONTENT_LENGTH, value);
        }
        if let Some(left) = incoming.take() {
            drain_incoming(left).await?;
        }
        hub.replace_request(id, header_pairs(&parts.headers), bytes.clone());
        RecordingBody::from_bytes(Bytes::from(bytes)).with_throttle(throttle_up_bps)
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
    let mut response = match sender.send(outbound).await {
        Ok(response) => response,
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
    } else {
        let mut pairs = header_pairs(&parts.headers);
        rules::apply_header_ops(&mut pairs, &response_ops);
        if cors {
            rules::ensure_cors(&mut pairs);
        }
        replace_header_map(&mut parts.headers, &pairs);
    }
    if !websocket && !break_response && !response_replaces.is_empty() {
        let known = parts
            .headers
            .get(http::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|text| text.parse::<usize>().ok());
        if known.is_some_and(|len| len <= crate::session::BODY_CAP) {
            let collected = body
                .collect()
                .await
                .map_err(|err| anyhow!("读取响应体: {err}"))?;
            let raw = collected.to_bytes();
            if let Some(next) = replaced_body(&raw, resp_encoding.as_deref(), &response_replaces) {
                parts.headers.remove(http::header::CONTENT_ENCODING);
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
                hub.set_error(id, "断点丢弃");
                hub.replace_response(id, 204, Vec::new(), Vec::new());
                hub.mark_finished(id);
                return response_from_headers(204, &[], &[]);
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
fn replaced_body(
    raw: &[u8],
    encoding: Option<&str>,
    pairs: &[(String, String)],
) -> Option<Vec<u8>> {
    if pairs.is_empty() || raw.len() > crate::session::BODY_CAP {
        return None;
    }
    let mut decoder = Decoder::from_header(encoding);
    let mut decoded = decoder.push(raw);
    decoded.extend(decoder.finish());
    if decoder.error().is_some() {
        return None;
    }
    rules::apply_replaces(&decoded, pairs)
}

fn note_decoded(hub: &Hub, id: u64, direction: Direction, raw: &[u8], encoding: Option<&str>) {
    hub.add_transfer(id, direction, raw.len() as u64);
    let mut decoder = Decoder::from_header(encoding);
    if let Some(error) = decoder.error() {
        hub.set_encoding_error(id, direction, error.to_string());
    }
    let decoded = decoder.push(raw);
    hub.append_body(id, direction, &decoded);
    let tail = decoder.finish();
    hub.append_body(id, direction, &tail);
    if let Some(error) = decoder.error() {
        hub.set_encoding_error(id, direction, error.to_string());
    }
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

fn empty_status(
    hub: &Arc<Hub>,
    id: u64,
    status: StatusCode,
) -> Result<Response<RecordingBody<Incoming>>> {
    synthetic_response(hub, id, status, "text/plain", Vec::new())
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
    id: u64,
    method: Method,
    scheme: &str,
    host: &str,
    port: u16,
    path: &str,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<Response<RecordingBody<Incoming>>> {
    if scheme == "https" {
        let (tls, _) = connect_origin(host, port, &[b"http/1.1".to_vec()], hub).await?;
        fetch_on(
            tls, hub, id, method, scheme, host, port, path, headers, body,
        )
        .await
    } else {
        let tcp = dial(host, port, hub, false).await?;
        fetch_on(
            tcp, hub, id, method, scheme, host, port, path, headers, body,
        )
        .await
    }
}

#[allow(clippy::too_many_arguments)]
async fn fetch_on<IO>(
    io: IO,
    hub: &Arc<Hub>,
    id: u64,
    method: Method,
    scheme: &str,
    host: &str,
    port: u16,
    path: &str,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<Response<RecordingBody<Incoming>>>
where
    IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let authority = display_authority(host, port, scheme);
    let mut outbound = Request::builder()
        .method(method)
        .uri(absolute_uri(scheme, &authority, path)?)
        .body(Full::new(body.clone()))
        .context("组装映射请求")?;
    *outbound.headers_mut() = headers.clone();
    strip_hop_by_hop(outbound.headers_mut(), false);
    outbound.headers_mut().remove(http::header::CONTENT_LENGTH);
    outbound.headers_mut().insert(
        http::header::CONTENT_LENGTH,
        http::HeaderValue::from_str(&body.len().to_string())
            .unwrap_or(http::HeaderValue::from_static("0")),
    );
    if !outbound.headers().contains_key(http::header::HOST)
        && let Ok(value) = http::HeaderValue::from_str(&authority)
    {
        outbound.headers_mut().insert(http::header::HOST, value);
    }
    let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
        .preserve_header_case(true)
        .title_case_headers(true)
        .handshake::<_, Full<Bytes>>(TokioIo::new(io))
        .await
        .context("映射连接")?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    sender.ready().await.context("映射连接未就绪")?;
    let response = sender.send_request(outbound).await.context("映射请求")?;
    let encoding = response
        .headers()
        .get(http::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let (parts, resp_body) = response.into_parts();
    hub.set_response(
        id,
        parts.status.as_u16(),
        header_pairs(&parts.headers),
        None,
    );
    let body = RecordingBody::new(
        resp_body,
        hub.clone(),
        id,
        Direction::Response,
        encoding.as_deref(),
    );
    Ok(Response::from_parts(parts, body))
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
    async fn breakpoint_drop_returns_204() {
        let dir = std::env::temp_dir().join(format!("zniff-break-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        hub.set_rules("break 127.0.0.1/hold");
        let proxy = listen().await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let hub_serve = hub.clone();
        tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let _ = handle_conn(socket, hub_serve).await;
        });
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
        timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
            .await
            .unwrap()
            .unwrap();
        waiter.await.unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("204"), "{text}");
        let flow = hub
            .with_session(|session| session.get(1).cloned())
            .expect("flow");
        assert_eq!(flow.error.as_deref(), Some("断点丢弃"));
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
        assert!(origin_text.contains("X-Edit: yes"), "{origin_text}");
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
        let (status, body) = h2_get(&ca, proxy_addr, origin_addr, "/hold").await;
        waiter.await.unwrap();
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(body.is_empty());
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
        let flow = hub
            .with_session(|session| session.flows().into_iter().next())
            .unwrap();
        assert_eq!(flow.error.as_deref(), Some("断点丢弃"));
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
}
