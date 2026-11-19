#[allow(unused_imports)]
use super::*;

pub(super) async fn hold_hyper_response(
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

pub(super) enum Take {
    Whole(Bytes),
    Overflow { prefix: Bytes, rest: Incoming },
}

pub(super) async fn take_limited(mut body: Incoming, cap: usize) -> Result<Take> {
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

pub(super) fn response_from_headers(
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

pub(super) async fn drain_and_record_request(
    hub: &Arc<Hub>,
    id: u64,
    mut body: Incoming,
    encoding: Option<&str>,
) -> Result<()> {
    let mut decoder = Decoder::from_header(encoding);
    let early = decoder.error().map(str::to_string);
    loop {
        match body.frame().await {
            Some(Ok(frame)) => {
                let Ok(data) = frame.into_data() else {
                    continue;
                };
                if data.is_empty() {
                    continue;
                }
                let decoded = decoder.push(&data);
                hub.record_chunk(id, Direction::Request, data.len() as u64, &decoded, None);
            }
            Some(Err(err)) => return Err(anyhow!("读取请求体: {err}")),
            None => {
                let tail = decoder.finish();
                let error = decoder.error().map(str::to_string).or(early);
                hub.record_chunk(id, Direction::Request, 0, &tail, error.as_deref());
                return Ok(());
            }
        }
    }
}

pub(super) async fn collect_frames_capped<B>(mut body: B, cap: usize) -> Result<Bytes>
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: std::fmt::Display,
{
    let mut buf = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|err| anyhow!("读取请求体: {err}"))?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        if buf.len().saturating_add(data.len()) > cap {
            anyhow::bail!("请求体超过 {} MB 缓冲上限", cap / (1024 * 1024));
        }
        buf.extend_from_slice(&data);
    }
    Ok(Bytes::from(buf))
}

pub(super) async fn collect_request_bytes_capped(
    hub: &Arc<Hub>,
    id: u64,
    body: Incoming,
    encoding: Option<&str>,
    cap: usize,
) -> Result<Bytes> {
    let raw = collect_frames_capped(body, cap).await?;
    note_decoded(hub, id, Direction::Request, &raw, encoding);
    Ok(raw)
}

pub(super) async fn drain_incoming(mut body: Incoming) -> Result<()> {
    while let Some(frame) = body.frame().await {
        frame.map_err(|err| anyhow!("丢掉请求体: {err}"))?;
    }
    Ok(())
}

/// Decodes `Content-Encoding`, then applies text replacements. `None` keeps
/// the wire bytes: binary, a failed decode, or a body the pairs do not change.
pub(super) fn current_decoded_request_body(
    hub: &Hub,
    id: u64,
    body_override: Option<&[u8]>,
    override_is_decoded: bool,
) -> Vec<u8> {
    if override_is_decoded {
        return body_override.map(<[u8]>::to_vec).unwrap_or_default();
    }
    let recorded = hub.with_session(|s| {
        s.get(id)
            .map(|f| f.request_body.clone())
            .unwrap_or_default()
    });
    if !recorded.is_empty() {
        recorded
    } else {
        body_override.map(<[u8]>::to_vec).unwrap_or_default()
    }
}

pub(super) fn expand_header_ops_for_flow(
    ops: &[rules::HeaderOp],
    method: &str,
    host: &str,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
    status: Option<u16>,
    now_ms: u64,
    only_req_context: bool,
) -> Vec<rules::HeaderOp> {
    if ops.is_empty() {
        return Vec::new();
    }
    let ctx = crate::rule_vars::VarContext {
        method,
        host,
        target: path,
        headers,
        body,
        status,
        now_ms,
    };
    crate::rule_vars::expand_header_ops(ops, &ctx, only_req_context)
}

pub(super) fn decode_body_for_hook(raw: &[u8], encoding: Option<&str>) -> Option<Vec<u8>> {
    if raw.len() > crate::session::BODY_CAP {
        return None;
    }
    let mut decoder = Decoder::from_header(encoding);
    let mut decoded = decoder.push(raw);
    decoded.extend(decoder.finish());
    decoder.error().is_none().then_some(decoded)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn apply_response_hooks(
    hub: &Hub,
    id: u64,
    specs: &[rules::HookSpec],
    method: &str,
    url: &str,
    raw: &[u8],
    encoding: Option<&str>,
    mut rewritten: Option<Vec<u8>>,
    status: &mut u16,
    resp_headers: &mut Vec<(String, String)>,
) -> (Option<Vec<u8>>, bool) {
    if !specs.iter().any(|h| h.phase.runs_response()) {
        return (rewritten, false);
    }
    let plain_for_hook = match rewritten.as_ref() {
        Some(bytes) => bytes.clone(),
        None => decode_body_for_hook(raw, encoding).unwrap_or_else(|| raw.to_vec()),
    };
    let Some(hook_mut) = crate::webhook::run_response_hooks(
        hub,
        id,
        specs,
        method,
        url,
        *status,
        resp_headers,
        &plain_for_hook,
    )
    .await
    else {
        return (rewritten, false);
    };
    let mut head_changed = false;
    if let Some(next_status) = hook_mut.status {
        *status = next_status;
        head_changed = true;
    }
    if let Some(next_headers) = hook_mut.headers {
        *resp_headers = next_headers;
        head_changed = true;
    }
    if let Some(next_body) = hook_mut.body {
        rewritten = Some(next_body);
    }
    (rewritten, head_changed)
}

pub(super) type BodyVarInfo<'a> = (
    &'a str,
    &'a str,
    &'a str,
    &'a [(String, String)],
    Option<u16>,
    u64,
);

pub(super) fn rewrite_body(
    hub: &Hub,
    id: u64,
    raw: &[u8],
    encoding: Option<&str>,
    pairs: &[(String, String)],
    json_ops: &[rules::JsonOp],
    var_info: Option<BodyVarInfo<'_>>,
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
    let (eff_pairs, eff_json_ops) =
        if let Some((method, host, path, headers, status, now_ms)) = var_info {
            let ctx = crate::rule_vars::VarContext {
                method,
                host,
                target: path,
                headers,
                body: &decoded,
                status,
                now_ms,
            };
            (
                std::borrow::Cow::Owned(crate::rule_vars::expand_replaces(pairs, &ctx)),
                std::borrow::Cow::Owned(crate::rule_vars::expand_json_ops(json_ops, &ctx)),
            )
        } else {
            (
                std::borrow::Cow::Borrowed(pairs),
                std::borrow::Cow::Borrowed(json_ops),
            )
        };
    let mut changed = false;
    if let Some(next) = rules::apply_replaces(&decoded, &eff_pairs) {
        decoded = next;
        changed = true;
    }
    if !eff_json_ops.is_empty() {
        match rules::apply_json_ops(&decoded, &eff_json_ops) {
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

pub(super) fn note_decoded(
    hub: &Hub,
    id: u64,
    direction: Direction,
    raw: &[u8],
    encoding: Option<&str>,
) {
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

pub(super) fn mapped_file_response(
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

pub(super) fn synthetic_response(
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

pub(super) fn synthetic_headers(
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

pub(super) fn replace_header_map(headers: &mut HeaderMap, pairs: &[(String, String)]) {
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
pub(super) async fn fetch_remote(
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
pub(super) async fn fetch_on<IO>(
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
    let upstream = hub.upstream_for_host(host);
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

pub(super) fn is_websocket(req: &Request<Incoming>) -> bool {
    req.headers()
        .get(http::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
        || req.method() == Method::CONNECT
}

pub(super) fn header_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
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

pub(super) fn strip_hop_by_hop(headers: &mut HeaderMap, websocket: bool) {
    let listed = connection_tokens(headers);
    let names: Vec<HeaderName> = headers.keys().cloned().collect();
    for name in names {
        if hop_by_hop(&name, &listed, websocket) {
            headers.remove(&name);
        }
    }
}

pub(super) fn connection_tokens(headers: &HeaderMap) -> Vec<HeaderName> {
    headers
        .get_all(http::header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|token| HeaderName::from_bytes(token.trim().as_bytes()).ok())
        .collect()
}

pub(super) fn hop_by_hop(name: &HeaderName, listed: &[HeaderName], websocket: bool) -> bool {
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

pub(super) fn absolute_uri(scheme: &str, host: &str, path: &str) -> Result<Uri> {
    let path = if path.is_empty() { "/" } else { path };
    format!("{scheme}://{host}{path}")
        .parse()
        .context("组装上游 URI")
}

pub(super) async fn splice_upgrades(
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
