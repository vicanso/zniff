//! HTTP proxy: CONNECT with protocol following, and plain HTTP.
//!
//! The origin is contacted directly, never through the system proxy. A side
//! copy of each body is decompressed and capped; forwarding does not wait
//! for the UI.

use std::collections::{HashMap, HashSet};
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

use crate::allow_hosts::AllowHostRules;
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
const H2_POOL_CAP: usize = 128;
const HTTP1_POOL_CAP: usize = 128;
const MAX_BUFFERED_REQUEST_BODY: usize = 64 * 1024 * 1024;
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

pub use crate::local_proc::resolve_local_process_name;
use crate::local_proc::{cached_local_app, store_cached_local_app};

type H2Sender = hyper::client::conn::http2::SendRequest<RecordingBody<Incoming>>;

mod hub;
mod hub_session;
mod hyper_forward;
mod hyper_helpers;
mod local_routes;
mod plain_http;
mod replay;
mod socks5;
mod tls_mitm;
mod upstream_io;
mod wire;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use self::hub::spilled_flow_ids;
pub use self::hub::{BreakDecision, BreakSide, Hub, ReplayOverride, Upstream, UpstreamKind};
use self::hub::{BreakSlot, PooledH2};
pub(crate) use self::hub::{ShareExport, SharePoll};
use self::hyper_forward::*;
use self::hyper_helpers::*;
use self::local_routes::*;
use self::plain_http::*;
pub use self::replay::replay_batch;
pub(crate) use self::replay::replay_once;
use self::tls_mitm::*;
pub(crate) use self::upstream_io::verify_basic_auth;
use self::upstream_io::*;
use self::wire::*;

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
        Some(LocalRoute::Pac) => return serve_pac(&mut socket, local_addr, &hub, &head).await,
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
        if !is_tls_probe_host(&host) && !hub.host_allowed(&host) {
            hub.record_allow_denied(&host);
            return write_host_not_allowed(&mut socket, &host, false).await;
        }
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
