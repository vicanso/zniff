#[allow(unused_imports)]
use super::*;

pub(super) async fn mitm_connect<S>(
    mut client: S,
    host: &str,
    port: u16,
    hub: Arc<Hub>,
) -> Result<()>
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
    if !hub.host_allowed(&sni_host) {
        return Ok(());
    }
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

pub(super) async fn connect_origin(
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

pub(super) async fn fail_client_cert_connect<S>(
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
    let acceptor = tokio_rustls::TlsAcceptor::from(config);
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
                let _ = drain_and_record_request(&hub, id, body, encoding.as_deref()).await;
                hub.note_rules_with_ops(id, &host, &path, &[]);
                if hub.ca.has_client_cert_in_dir(&host) {
                    hub.with_session(|session| {
                        session.append_rule_hit(
                            id,
                            crate::session::RuleHit {
                                group_name: None,
                                line_no: 0,
                                verb: "client-cert".to_string(),
                            },
                        );
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
pub(super) async fn intercept<S>(
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
    let acceptor = tokio_rustls::TlsAcceptor::from(config);
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

pub(super) fn client_tls_reason(err: &io::Error, host: &str) -> String {
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

pub(super) fn client_tls_alert(err: &io::Error) -> Option<rustls::AlertDescription> {
    let tls = err.get_ref()?.downcast_ref::<rustls::Error>()?;
    match tls {
        rustls::Error::AlertReceived(alert) => Some(*alert),
        _ => None,
    }
}

/// Connection closed with no TLS alert. A pinned host is the usual case;
/// otherwise the CA is the first thing to check.
pub(super) fn reset_guess(host: &str) -> String {
    if likely_pinned(host) {
        pinned_guess()
    } else {
        "证书被拒绝。猜测：先确认系统已信任 zniff CA。若已信任，就是这个客户端自带证书库或做了钉扎，把主机加入直通名单。".into()
    }
}

pub(super) fn pinned_guess() -> String {
    "证书被拒绝。猜测：该客户端多半做了证书钉扎，不接受代理签发的证书。把主机加入直通名单。若还没安装 zniff CA，先安装。".into()
}

pub(super) fn likely_pinned(host: &str) -> bool {
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

pub(super) fn host_ends_with(host: &str, suffix: &str) -> bool {
    host.len() > suffix.len()
        && host.as_bytes().get(host.len() - suffix.len() - 1) == Some(&b'.')
        && host[host.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

pub(super) type H1Sender = hyper::client::conn::http1::SendRequest<RecordingBody<Incoming>>;

pub(super) async fn handshake_h1<O>(origin: O) -> Result<H1Sender>
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

pub(super) async fn dial_h1(
    host: &str,
    port: u16,
    hub: &Hub,
) -> Result<(H1Sender, crate::session::ConnInfo)> {
    let (origin, _, conn_info) = timeout(
        IO_TIMEOUT,
        connect_origin(host, port, &[b"http/1.1".to_vec()], hub),
    )
    .await
    .context("连接源站超时")??;
    let sender = handshake_h1(origin).await?;
    Ok((sender, conn_info))
}

pub(super) async fn serve_h1<C, O>(
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

pub(super) async fn handshake_h2<O>(
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
pub(super) async fn dial_h2(
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

pub(super) async fn serve_h2<C, O>(
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
pub(super) async fn intercept_h2_pooled<S>(
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
    let acceptor = tokio_rustls::TlsAcceptor::from(config);
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
pub(super) async fn serve_h2_sender<C>(
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

pub(super) fn server_name(host: &str) -> Result<ServerName<'static>> {
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        Ok(ServerName::IpAddress(ip.into()))
    } else {
        ServerName::try_from(host.to_string()).context("SNI")
    }
}

/// Helper to distinguish local client certificate configuration / loading errors
/// from upstream TLS handshake rejections.
pub(super) fn is_client_cert_error(err: &anyhow::Error) -> bool {
    err.chain().any(|c| {
        let s = c.to_string();
        s.contains("客户端证书")
            || s.contains("mTLS")
            || s.contains("client certificate")
            || s.contains("client-cert")
    })
}

pub(super) fn client_cert_load_reason(err: &anyhow::Error) -> String {
    format!("客户端证书加载失败: {err:#}")
}

/// Short reason for a probe that never reached ServerHello. `{err}` alone is
/// only the `context("源站 TLS")` label, so the list would repeat that label.
pub(super) fn origin_tls_reason(err: &anyhow::Error) -> String {
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
pub(super) fn explain_flow_error(err: &anyhow::Error) -> String {
    let label = err.to_string();
    if label.is_empty() || label.contains('。') || label.contains("猜测") {
        return label;
    }
    match flow_error_guess(err, &label) {
        Some(guess) => format!("{label}。猜测：{guess}"),
        None => label,
    }
}

pub(super) fn flow_error_guess(err: &anyhow::Error, label: &str) -> Option<&'static str> {
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
    if text.contains("socks5") {
        if text.contains("认证") {
            return Some("上游 SOCKS5 代理认证失败，请检查用户名或密码。");
        }
        return Some("上游 SOCKS5 代理握手或转发失败，请检查代理地址、协议或网络连通性。");
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

pub(super) fn canceled_guess(label: &str) -> &'static str {
    if label.contains("HTTP/2") {
        "复用的 HTTP/2 连接已经关掉，请求还没送到源站。下一次会重新连接。"
    } else if label.contains("HTTP/1.1") {
        "到源站的 HTTP/1.1 连接已经结束，这条请求没送出去。"
    } else {
        "连接已经关掉，请求没送出去。"
    }
}

pub(super) fn is_flow_timeout(
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

pub(super) fn timeout_guess(label: &str) -> &'static str {
    if label.contains("请求头") {
        "客户端迟迟没有把请求头发完。"
    } else if label.contains("源站") || label.contains("隧道") {
        "源站没有在时限内接受连接。可能是网络不通、地址解析慢，或源站没有响应。"
    } else {
        "这一步在时限内没有完成。"
    }
}

pub(super) fn is_flow_canceled(label: &str, text: &str, hyper: Option<&hyper::Error>) -> bool {
    hyper.is_some_and(|err| err.is_canceled())
        || text.contains("connection was not ready")
        || text.contains("operation was canceled")
        || label == "上游 HTTP/1.1 未就绪"
        || label == "上游 HTTP/1.1 已经关闭"
}

pub(super) fn is_flow_reset(text: &str, io: Option<&std::io::Error>) -> bool {
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

pub(super) fn bare_step_guess(label: &str) -> Option<&'static str> {
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

pub(super) fn rustls_error(err: &anyhow::Error) -> Option<&rustls::Error> {
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

pub(super) fn record_tunnel(hub: &Hub, host: &str, port: u16, reason: String) {
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

pub(super) fn record_note(hub: &Hub, host: &str, port: u16, protocol: Protocol, reason: String) {
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

pub(super) fn display_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

pub(super) fn display_authority(host: &str, port: u16, scheme: &str) -> String {
    let host = display_host(host);
    let default_port = if scheme == "https" { 443 } else { 80 };
    if port == default_port {
        host
    } else {
        format!("{host}:{port}")
    }
}

pub(super) async fn tunnel_after_hello<S>(
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
