#[allow(unused_imports)]
use super::*;

pub(super) fn pool_key(host: &str, port: u16, hub: &Hub) -> (String, u16) {
    if let Some((target_host, port_override)) = hub.resolve_for_host(host) {
        return (
            format!("{}@{target_host}", host.to_ascii_lowercase()),
            port_override.unwrap_or(port),
        );
    }
    match hub.upstream() {
        Some(upstream) if upstream.is_http_proxy() => (upstream.host, upstream.port),
        Some(_) => (format!("socks5@{host}"), port),
        None => (host.to_string(), port),
    }
}

pub(super) async fn dial(host: &str, port: u16, hub: &Hub, tunnel: bool) -> Result<TcpStream> {
    Ok(dial_timed(host, port, hub, tunnel).await?.0)
}

pub(super) async fn dial_timed(
    host: &str,
    port: u16,
    hub: &Hub,
    tunnel: bool,
) -> Result<(TcpStream, crate::session::ConnInfo)> {
    let mut info = crate::session::ConnInfo::default();
    let resolved = hub.resolve_for_host(host);
    if resolved.is_none()
        && let Some(upstream) = hub.upstream()
    {
        let tcp_start = Instant::now();
        let mut stream = TcpStream::connect((upstream.host.as_str(), upstream.port))
            .await
            .context("连接上游代理")?;
        stream.set_nodelay(true).ok();
        if let Ok(peer) = stream.peer_addr() {
            info.server_addr = peer.to_string();
        }
        match upstream.kind {
            UpstreamKind::Http if tunnel => {
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
            UpstreamKind::Http => {}
            UpstreamKind::Socks5 { remote_dns } => {
                super::socks5::socks5_handshake(
                    &mut stream,
                    host,
                    port,
                    remote_dns,
                    upstream.socks5_auth.as_ref(),
                )
                .await?;
            }
        }
        info.tcp_ms = Some(tcp_start.elapsed().as_millis() as u64);
        return Ok((stream, info));
    }
    let (dial_host, dial_port) = match resolved {
        Some((target_host, port_override)) => (target_host, port_override.unwrap_or(port)),
        None => (host.to_string(), port),
    };
    let clean_host = dial_host.trim().trim_matches(|ch| ch == '[' || ch == ']');
    let stream = if let Ok(ip) = clean_host.parse::<std::net::IpAddr>() {
        let tcp_start = Instant::now();
        let s = TcpStream::connect((ip, dial_port)).await?;
        info.tcp_ms = Some(tcp_start.elapsed().as_millis() as u64);
        s
    } else {
        let dns_start = Instant::now();
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((clean_host, dial_port))
            .await?
            .collect();
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

pub(super) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
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

pub(super) fn decode_percent_userinfo(raw: &str) -> String {
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

pub(super) fn parse_upstream(text: &str) -> Option<Upstream> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        return None;
    }
    let (kind, rest, default_port) = if let Some((scheme, after)) = text.split_once("://") {
        match scheme.to_ascii_lowercase().as_str() {
            "http" => (UpstreamKind::Http, after, 8080),
            "socks5" => (UpstreamKind::Socks5 { remote_dns: false }, after, 1080),
            "socks5h" => (UpstreamKind::Socks5 { remote_dns: true }, after, 1080),
            _ => return None,
        }
    } else {
        (UpstreamKind::Http, text, 80)
    };
    let authority = rest.split('/').next().unwrap_or(rest);
    if authority.is_empty() || authority.contains('?') || authority.contains('#') {
        return None;
    }
    let (auth_header, socks5_auth, host_port) = match authority.rsplit_once('@') {
        Some((userinfo, hp)) if !userinfo.is_empty() => {
            let (u, p) = userinfo.split_once(':').unwrap_or((userinfo, ""));
            let user = decode_percent_userinfo(u);
            let pass = decode_percent_userinfo(p);
            match kind {
                UpstreamKind::Http => {
                    let creds = format!("{user}:{pass}");
                    let encoded = crate::session::base64_encode(creds.as_bytes());
                    (Some(format!("Basic {encoded}")), None, hp)
                }
                UpstreamKind::Socks5 { .. } => (None, Some((user, pass)), hp),
            }
        }
        Some((_, hp)) => (None, None, hp),
        None => (None, None, authority),
    };
    if !host_port.starts_with('[') && host_port.matches(':').count() > 1 {
        return None;
    }
    let (host, port) = split_host_port(host_port, default_port).ok()?;
    if host.is_empty() || port == 0 {
        return None;
    }
    Some(Upstream {
        kind,
        host,
        port,
        auth_header,
        socks5_auth,
    })
}

pub(super) fn origin_dial_rules(rules: &[rules::Rule]) -> Vec<&rules::Rule> {
    rules
        .iter()
        .filter(|r| {
            matches!(
                r,
                rules::Rule::ClientCert { .. } | rules::Rule::Resolve { .. }
            )
        })
        .collect()
}

pub(super) async fn read_client_request_body<S>(
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

pub(super) async fn drain_request<S>(
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

pub(super) async fn discard_request<S>(client: &mut S, headers: &[(String, String)]) -> Result<()>
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
pub(super) async fn write_mapped_file<S>(
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

pub(super) async fn write_static<S>(
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
pub(super) async fn write_static_throttled<S>(
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
pub(super) async fn write_static_for_method<S>(
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
pub(super) async fn exchange_tls<S>(
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
    flow_now_ms: u64,
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
    let expanded_res = expand_header_ops_for_flow(
        &plan.response_headers,
        method,
        host,
        path,
        &resp_headers,
        &[],
        Some(status),
        flow_now_ms,
        false,
    );
    rules::apply_header_ops(&mut resp_headers, &expanded_res);
    if plan.cors {
        rules::ensure_cors(&mut resp_headers);
    }
    hub.set_response(id, status, resp_headers.clone(), None);
    let has_resp_hooks = plan.hooks.iter().any(|h| h.phase.runs_response());
    let no_resp_body = response_has_no_body(method, status);
    let mut origin = Prefixed {
        prefix: resp_rest,
        pos: 0,
        inner: origin,
    };
    if !break_response
        && (has_resp_hooks
            || (!no_resp_body
                && (!plan.response_replaces.is_empty() || !plan.response_json_ops.is_empty())))
    {
        let buffered_resp = if no_resp_body {
            Some(Vec::new())
        } else if let Some(len) = content_length(&resp_headers) {
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
            let mut next_body = if !raw.is_empty() {
                rewrite_body(
                    hub,
                    id,
                    &raw,
                    encoding.as_deref(),
                    &plan.response_replaces,
                    &plan.response_json_ops,
                    Some((method, host, path, &resp_headers, Some(status), flow_now_ms)),
                )
            } else {
                None
            };
            if has_resp_hooks {
                let flow_url = format!("https://{}{path}", display_authority(host, port, "https"));
                let (mutated_body, head_changed) = apply_response_hooks(
                    hub,
                    id,
                    &plan.hooks,
                    method,
                    &flow_url,
                    &raw,
                    encoding.as_deref(),
                    next_body,
                    &mut status,
                    &mut resp_headers,
                )
                .await;
                next_body = mutated_body;
                if head_changed {
                    reason = reason_phrase(status).to_string();
                }
            }
            if let Some(next) = next_body {
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
