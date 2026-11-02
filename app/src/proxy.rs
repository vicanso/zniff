//! HTTP proxy: CONNECT with protocol following, and plain HTTP.
//!
//! The origin is contacted directly, never through the system proxy. A side
//! copy of each body is decompressed and capped; forwarding does not wait
//! for the UI.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{anyhow, Context as _, Result};
use bytes::Bytes;
use http::{HeaderMap, HeaderName, Method, Request, Response, StatusCode, Uri, Version};
use http_body::Body;
use http_body::Frame;
use hyper::body::Incoming;
use hyper::upgrade::Upgraded;
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::client::ClientConfig;
use rustls::pki_types::ServerName;
#[cfg(test)]
use rustls::pki_types::CertificateDer;
use rustls::RootCertStore;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use crate::ca::Ca;
use crate::client_hello::{read_client_hello, ReadHello};
use crate::decompress::Decoder;
use crate::session::{Direction, FlowDraft, Protocol, Session};

const IO_TIMEOUT: Duration = Duration::from_secs(20);

pub struct Hub {
    session: Mutex<Session>,
    generation: AtomicU64,
    pub ca: Arc<Ca>,
    roots: RootCertStore,
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
        })
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
        let id = self.with_session(|session| session.open(draft));
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

    fn set_encoding_error(&self, id: u64, direction: Direction, error: impl Into<String>) {
        self.with_session(|session| session.set_encoding_error(id, direction, error));
        self.touch();
    }

    fn mark_protocol(&self, id: u64, protocol: Protocol) {
        self.with_session(|session| session.set_protocol(id, protocol));
        self.touch();
    }
}

pub async fn serve(
    listener: TcpListener,
    hub: Arc<Hub>,
    mut shutdown: watch::Receiver<bool>,
) {
    log::info!("代理监听 {}", listener.local_addr().map(|a| a.to_string()).unwrap_or_default());
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
    pub fn spawn(hub: Arc<Hub>) -> Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel();
        let (shutdown, shutdown_rx) = watch::channel(false);
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            runtime.block_on(async move {
                let listener = match TcpListener::bind("127.0.0.1:0").await {
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

async fn handle_conn(mut socket: TcpStream, hub: Arc<Hub>) -> Result<()> {
    socket.set_nodelay(true).ok();
    let (head_bytes, rest) = read_header_block(&mut socket).await?;
    let head = parse_head(&head_bytes)?;
    let mut parts = head.start.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
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
        return tunnel_after_hello(&mut client, raw, host, port).await;
    };
    let sni_host = hello.sni.clone().unwrap_or_else(|| host.to_string());
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
            log::debug!("源站 TLS 失败，改隧道 {sni_host}:{port}: {err:#}");
            record_tunnel(
                &hub,
                &sni_host,
                port,
                format!("源站 TLS 失败，已改为隧道: {err}"),
            );
            tunnel_after_hello(&mut client, hello.raw, host, port).await
        }
        Err(_) => {
            record_tunnel(&hub, &sni_host, port, "连接源站超时，已改为隧道".into());
            tunnel_after_hello(&mut client, hello.raw, host, port).await
        }
    }
}

async fn connect_origin(
    host: &str,
    port: u16,
    alpn: &[Vec<u8>],
    hub: &Hub,
) -> Result<(tokio_rustls::client::TlsStream<TcpStream>, Vec<u8>)> {
    let tcp = timeout(IO_TIMEOUT, TcpStream::connect((host, port)))
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
        .ca
        .server_config(host, &selected)
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
            let reason = client_tls_reason(&err);
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

fn client_tls_reason(err: &io::Error) -> String {
    match err.kind() {
        io::ErrorKind::UnexpectedEof
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::BrokenPipe => "证书被拒绝".into(),
        _ => {
            let text = err.to_string();
            let lower = text.to_ascii_lowercase();
            if lower.contains("certificate") || lower.contains("alert") || lower.contains("close_notify")
            {
                "证书被拒绝".into()
            } else {
                format!("客户端 TLS 失败: {text}")
            }
        }
    }
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
        if let Err(err) = connection.await {
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
    let service = hyper::service::service_fn(move |req| {
        let mut sender = shared.clone();
        let hub = hub.clone();
        let host = host_for_service.clone();
        async move {
            forward_request(&mut sender, req, hub, &host, port, "https", Protocol::Http11)
                .await
                .map_err(|err| io::Error::other(err.to_string()))
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

async fn serve_h2<C, O>(
    client: C,
    origin: O,
    host: String,
    port: u16,
    hub: Arc<Hub>,
) -> Result<()>
where
    C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    O: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let executor = TokioExecutor::new();
    let (sender, connection) = hyper::client::conn::http2::Builder::new(executor.clone())
        .handshake::<_, RecordingBody<Incoming>>(TokioIo::new(origin))
        .await
        .context("上游 HTTP/2")?;
    tokio::spawn(async move {
        if let Err(err) = connection.await {
            log::debug!("上游 HTTP/2 连接结束: {err}");
        }
    });
    let host_for_service = host.clone();
    let service = hyper::service::service_fn(move |req| {
        let mut sender = sender.clone();
        let hub = hub.clone();
        let host = host_for_service.clone();
        async move {
            forward_request(&mut sender, req, hub, &host, port, "https", Protocol::Http2)
                .await
                .map_err(|err| io::Error::other(err.to_string()))
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
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut origin = timeout(IO_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .context("隧道连接源站超时")??;
    origin.set_nodelay(true).ok();
    origin.write_all(&hello).await?;
    copy_bidirectional(client, &mut origin).await?;
    Ok(())
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
    let mut origin = timeout(IO_TIMEOUT, TcpStream::connect((host.as_str(), port)))
        .await
        .context("连接源站超时")
        .map_err(|err| {
            hub.set_error(id, err.to_string());
            err
        })??;
    origin.set_nodelay(true).ok();
    let mut outbound = format!("{method} {path} HTTP/1.1\r\n");
    let mut has_host = false;
    let mut has_connection = false;
    for (name, value) in &head.headers {
        if name.eq_ignore_ascii_case("proxy-connection") {
            continue;
        }
        if name.eq_ignore_ascii_case("host") {
            has_host = true;
        }
        if name.eq_ignore_ascii_case("connection") {
            has_connection = true;
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
        outbound.push_str("Connection: close\r\n");
    }
    outbound.push_str("\r\n");
    origin.write_all(outbound.as_bytes()).await?;
    let encoding = header_value(&head.headers, "content-encoding");
    copy_message_body(client, &mut origin, &head.headers, &hub, id, Direction::Request, encoding)
        .await?;
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
    if status == 101 && header_is_websocket(&head.headers) {
        hub.mark_protocol(id, Protocol::WebSocket);
        if !resp_rest.is_empty() {
            client.write_all(&resp_rest).await?;
        }
        copy_bidirectional(client, &mut origin).await?;
        return Ok(());
    }
    let mut origin = Prefixed {
        prefix: resp_rest,
        pos: 0,
        inner: origin,
    };
    let encoding = header_value(&resp.headers, "content-encoding");
    copy_message_body(
        &mut origin,
        client,
        &resp.headers,
        &hub,
        id,
        Direction::Response,
        encoding,
    )
    .await?;
    Ok(())
}

async fn copy_message_body<R, W>(
    from: &mut R,
    to: &mut W,
    headers: &[(String, String)],
    hub: &Hub,
    id: u64,
    direction: Direction,
    encoding: Option<String>,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut decoder = Decoder::from_header(encoding.as_deref());
    if let Some(error) = decoder.error() {
        hub.set_encoding_error(id, direction, error.to_string());
    }
    if let Some(length) = content_length(headers) {
        let mut left = length;
        let mut buf = vec![0u8; 16 * 1024];
        while left > 0 {
            let want = left.min(buf.len());
            let n = from.read(&mut buf[..want]).await?;
            if n == 0 {
                break;
            }
            to.write_all(&buf[..n]).await?;
            let decoded = decoder.push(&buf[..n]);
            hub.append_body(id, direction, &decoded);
            left -= n;
        }
    } else if is_chunked(headers) {
        copy_chunked(from, to, hub, id, direction, &mut decoder).await?;
    } else if direction == Direction::Response {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = from.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            to.write_all(&buf[..n]).await?;
            let decoded = decoder.push(&buf[..n]);
            hub.append_body(id, direction, &decoded);
        }
    }
    let tail = decoder.finish();
    hub.append_body(id, direction, &tail);
    if let Some(error) = decoder.error() {
        hub.set_encoding_error(id, direction, error.to_string());
    }
    Ok(())
}

async fn copy_chunked<R, W>(
    from: &mut R,
    to: &mut W,
    hub: &Hub,
    id: u64,
    direction: Direction,
    decoder: &mut Decoder,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
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
        let mut buf = vec![0u8; 16 * 1024];
        while left > 0 {
            let want = left.min(buf.len());
            let n = from.read(&mut buf[..want]).await?;
            if n == 0 {
                anyhow::bail!("分块 body 提前结束");
            }
            to.write_all(&buf[..n]).await?;
            let decoded = decoder.push(&buf[..n]);
            hub.append_body(id, direction, &decoded);
            left -= n;
        }
        let crlf = read_exact(from, 2).await?;
        to.write_all(&crlf).await?;
    }
    Ok(())
}

fn content_length(headers: &[(String, String)]) -> Option<usize> {
    header_value(headers, "content-length")?.parse().ok()
}

fn is_chunked(headers: &[(String, String)]) -> bool {
    header_value(headers, "transfer-encoding")
        .map(|value| value.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false)
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
        let default_port = if target.starts_with("https://") { 443 } else { 80 };
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
    if let Some((host, port)) = authority.rsplit_once(':') {
        if let Ok(port) = port.parse::<u16>() {
            return Ok((host.to_string(), port));
        }
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
    buf.windows(4).position(|window| window == b"\r\n\r\n").map(|index| index + 4)
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

async fn copy_bidirectional<A, B>(a: &mut A, b: &mut B) -> Result<()>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    tokio::io::copy_bidirectional(a, b).await?;
    Ok(())
}

/// Records a decompressed side copy while the original frames pass through.
struct RecordingBody<B> {
    inner: B,
    record: Option<Record>,
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
            inner,
            record: Some(Record {
                hub,
                id,
                direction,
                decoder,
                finished: false,
            }),
        }
    }

    fn passthrough(inner: B) -> Self {
        Self {
            inner,
            record: None,
        }
    }

    fn record_bytes(&mut self, bytes: &[u8]) {
        let Some(record) = self.record.as_mut() else {
            return;
        };
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
        let decoded = record.decoder.finish();
        record
            .hub
            .append_body(record.id, record.direction, &decoded);
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
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    let bytes = data.clone();
                    this.record_bytes(&bytes);
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(None) => {
                this.finish_record();
                Poll::Ready(None)
            }
            other => other,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
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

impl HttpSender for hyper::client::conn::http2::SendRequest<RecordingBody<Incoming>> {
    fn send(
        &mut self,
        req: Request<RecordingBody<Incoming>>,
    ) -> impl std::future::Future<Output = Result<Response<Incoming>>> + Send {
        let fut = self.send_request(req);
        async move { fut.await.context("HTTP/2 上游") }
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
    let path = parts
        .uri
        .path_and_query()
        .map(|path| path.as_str().to_string())
        .unwrap_or_else(|| "/".into());
    let authority = display_authority(host, port, scheme);
    let id = hub.open(FlowDraft {
        protocol,
        method: parts.method.to_string(),
        host: host.to_string(),
        path: path.clone(),
        url: format!("{scheme}://{authority}{path}"),
        request_headers: header_pairs(&parts.headers),
    });
    let body = if websocket {
        RecordingBody::passthrough(body)
    } else {
        RecordingBody::new(body, hub.clone(), id, Direction::Request, encoding.as_deref())
    };
    parts.version = match protocol {
        Protocol::Http2 => Version::HTTP_2,
        _ => Version::HTTP_11,
    };
    parts.uri = absolute_uri(scheme, &authority, &path)?;
    strip_hop_by_hop(&mut parts.headers, websocket);
    let outbound = Request::from_parts(parts, body);
    let mut response = sender.send(outbound).await?;
    if websocket && response.status() == StatusCode::SWITCHING_PROTOCOLS {
        hub.mark_protocol(id, Protocol::WebSocket);
        if let Some(upgrade) = upgrade {
            let server_up = hyper::upgrade::on(&mut response);
            tokio::spawn(async move {
                splice_upgrades(upgrade, server_up).await;
            });
        }
    }
    let resp_encoding = response
        .headers()
        .get(http::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let (parts, body) = response.into_parts();
    hub.set_response(id, parts.status.as_u16(), header_pairs(&parts.headers), None);
    let body = if websocket {
        RecordingBody::passthrough(body)
    } else {
        RecordingBody::new(
            body,
            hub,
            id,
            Direction::Response,
            resp_encoding.as_deref(),
        )
    };
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
) {
    let (client, server) = tokio::join!(client, server);
    let Ok(client) = client else { return };
    let Ok(server) = server else { return };
    let mut client = TokioIo::new(client);
    let mut server = TokioIo::new(server);
    let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
}



#[cfg(test)]
mod tests {
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
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let mut leaf_params = rcgen::CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
        leaf_params.is_ca = rcgen::IsCa::NoCa;
        leaf_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        leaf_params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        let origin_leaf = leaf_params
            .signed_by(&leaf_key, &origin_ca, &origin_ca_key)
            .unwrap();
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
        assert_eq!(tls.get_ref().1.alpn_protocol(), Some(b"http/1.1".as_slice()));

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

        let (mut sender, connection) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
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
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let mut leaf_params = rcgen::CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
        leaf_params.is_ca = rcgen::IsCa::NoCa;
        leaf_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        leaf_params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        let origin_leaf = leaf_params
            .signed_by(&leaf_key, &origin_ca, &origin_ca_key)
            .unwrap();
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
}
