#[allow(unused_imports)]
use super::*;

pub(super) fn parse_http_url(url: &str) -> Option<(&str, String, u16, String)> {
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

pub(super) enum LocalRoute {
    Cert,
    Install,
    Pac,
    Share(String, crate::share::SharePath),
    Relay(String, crate::share::SharePath, RelayVerb),
    Favicon,
    Mcp,
}

pub(super) enum RelayVerb {
    Put,
    Delete,
}

pub(super) fn targets_self(
    local_addr: Option<SocketAddr>,
    head: &Head,
    method: &str,
    target: &str,
) -> bool {
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
pub(super) fn local_route(method: &str, target: &str) -> Option<LocalRoute> {
    local_route_for(method, target, false)
}

pub(super) fn local_route_for(method: &str, target: &str, self_target: bool) -> Option<LocalRoute> {
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
    if direct && (path == "/proxy.pac" || path == "/zniff.pac") {
        return Some(LocalRoute::Pac);
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

pub(super) fn is_loopback_host(authority: &str) -> bool {
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

pub(super) async fn serve_mcp(
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

pub(super) async fn serve_relay(
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

pub(super) async fn read_relay_body(
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

pub(super) fn relay_drafts(doc: &crate::share::InboundShare) -> Vec<RelayDraft> {
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

pub(super) fn millis(ms: u128) -> Duration {
    Duration::from_millis(u64::try_from(ms).unwrap_or(u64::MAX))
}

pub(super) fn protocol_from_label(label: &str) -> Protocol {
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

pub(super) fn shared_messages(lines: &[String]) -> Vec<Message> {
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

pub(super) async fn serve_share<S>(
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

/// Builds the plain-text body returned for disallowed hosts without control characters.
pub(super) fn host_not_allowed_body(host: &str) -> String {
    let clean_host: String = host
        .trim()
        .chars()
        .filter(|ch| !ch.is_control() && !ch.is_whitespace())
        .collect();
    format!("zniff: host not allowed: {clean_host}")
}

/// Sends an HTTP 403 Forbidden response when the requested host violates `allow_hosts`.
pub(super) async fn write_host_not_allowed<S>(
    socket: &mut S,
    host: &str,
    omit_body: bool,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let body = host_not_allowed_body(host);
    let head = format!(
        "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    );
    socket.write_all(head.as_bytes()).await?;
    if !omit_body {
        socket.write_all(body.as_bytes()).await?;
    }
    Ok(())
}

pub(super) async fn write_proxy_auth_required<S>(socket: &mut S, omit_body: bool) -> Result<()>
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

pub(super) async fn write_untracked<S>(
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

pub(super) async fn write_untracked_encoded_for_method<S>(
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

pub(super) fn head_method(head: &Head) -> String {
    head.start
        .split_whitespace()
        .next()
        .unwrap_or("GET")
        .to_string()
}

pub(super) fn accepts_gzip(header: Option<&str>) -> bool {
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

pub(super) fn gzip_bytes(body: &[u8]) -> Option<Vec<u8>> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(body).ok()?;
    encoder.finish().ok()
}

const INSTALL_PAGE_STYLE: &str = r#"  body { margin: 0; background: #0d1017; color: #f1f5f9; font: 16px/1.55 -apple-system, BlinkMacSystemFont, sans-serif; }
  main { max-width: 32rem; margin: 0 auto; padding: 1.5rem 1.25rem 2.5rem; }
  h1 { font-size: 1.35rem; margin: 0 0 0.6rem; }
  h2 { font-size: 1rem; margin: 1.4rem 0 0.4rem; }
  p { margin: 0.4rem 0; }
  a.btn { display: block; margin: 1rem 0 0.4rem; padding: 0.85rem 1rem; border-radius: 10px; background: #2563eb; color: #fff; font-weight: 600; text-align: center; text-decoration: none; }
  ol { margin: 0.3rem 0; padding-left: 1.2rem; }
  code { font-family: "JetBrains Mono", "JetBrains Mono NL", ui-monospace, Menlo, monospace; font-size: 0.88em; }
  .note { color: #94a3b8; font-size: 0.9rem; }
  details.card { margin: 0.9rem 0; padding: 0.75rem 0.9rem; border-radius: 10px; border: 1px solid #1e293b; background: #131926; }
  details.card > summary { cursor: pointer; font-weight: 600; font-size: 0.95rem; }
  details.sub-card { margin-top: 0.75rem; padding-top: 0.65rem; border-top: 1px solid #1e293b; }
  details.sub-card > summary { cursor: pointer; font-size: 0.9rem; color: #cbd5e1; }
  .snippet { margin: 0.65rem 0; border-radius: 8px; border: 1px solid #1e293b; background: #0b0f19; overflow: hidden; }
  .snippet-head { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; padding: 0.4rem 0.65rem; border-bottom: 1px solid #1e293b; font-size: 0.82rem; color: #cbd5e1; }
  .copy-btn { border: 1px solid #334155; background: #1e293b; color: #f8fafc; border-radius: 6px; padding: 0.25rem 0.6rem; font-size: 0.78rem; cursor: pointer; flex-shrink: 0; }
  .copy-btn:active { opacity: 0.75; }
  pre { margin: 0; padding: 0.65rem; overflow-x: auto; font-size: 0.8rem; line-height: 1.45; color: #e2e8f0; }"#;

fn build_install_page(zh: bool) -> String {
    let android_section = crate::android_guide::install_page_android_section(zh);
    let copy_script = crate::android_guide::INSTALL_PAGE_COPY_SCRIPT;
    if zh {
        format!(
            r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="icon" href="data:,">
<title>安装 zniff 根证书</title>
<style>
{INSTALL_PAGE_STYLE}
</style>
</head>
<body>
<main>
  <h1>安装 zniff 根证书</h1>
  <p>手机和电脑在同一局域网。先下载证书，再按系统步骤信任它。</p>
  <a class="btn" href="/zniff-ca.crt">下载根证书</a>
  <h2>iPhone / iPad</h2>
  <ol>
    <li>用 <strong>Safari</strong> 打开本页，点上面的「下载根证书」并允许下载描述文件（微信或第三方浏览器无法唤起描述文件安装）。</li>
    <li>打开「设置」→「已下载描述文件」（或「设置」→「通用」→「VPN 与设备管理」），安装 zniff CA。</li>
    <li>安装完成后，才会在「设置」→「通用」→「关于本机」→「证书信任设置」里看到 zniff CA，打开它的完全信任。</li>
  </ol>
  <h2>Android</h2>
  <ol>
    <li>点下载，保存 <code>zniff-ca.crt</code>。</li>
    <li>打开「设置」→「安全」→「加密与凭据」→「安装证书」→「CA 证书」。</li>
  </ol>
{android_section}
  <p class="note">装好后，把手机 Wi-Fi 的代理指到这台电脑的地址和端口（或自动代理 PAC URL <code>/proxy.pac</code>），才能抓到手机的 HTTP(S)。</p>
</main>
{copy_script}
</body>
</html>
"#
        )
    } else {
        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="icon" href="data:,">
<title>Install the zniff root certificate</title>
<style>
{INSTALL_PAGE_STYLE}
</style>
</head>
<body>
<main>
  <h1>Install the zniff root certificate</h1>
  <p>Keep the phone and the computer on the same network. Download the certificate, then trust it.</p>
  <a class="btn" href="/zniff-ca.crt">Download the root certificate</a>
  <h2>iPhone / iPad</h2>
  <ol>
    <li>Open this page in <strong>Safari</strong>, tap Download the root certificate above, and allow the configuration profile (other browsers only save the file).</li>
    <li>Open Settings → Profile Downloaded (or Settings → General → VPN &amp; Device Management) and install zniff CA.</li>
    <li>Once installed, zniff CA appears in Settings → General → About → Certificate Trust Settings. Enable full trust for it.</li>
  </ol>
  <h2>Android</h2>
  <ol>
    <li>Tap the download and save <code>zniff-ca.crt</code>.</li>
    <li>Open Settings → Security → Encryption &amp; credentials → Install a certificate → CA certificate.</li>
  </ol>
{android_section}
  <p class="note">After it is installed, point the phone's Wi-Fi HTTP proxy (or Auto Proxy PAC URL <code>/proxy.pac</code>) at this computer's address and port to capture HTTP(S).</p>
</main>
{copy_script}
</body>
</html>
"#
        )
    }
}

pub(super) fn install_page_for(accept_language: Option<&str>) -> &'static str {
    static PAGE_ZH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    static PAGE_EN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
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
        PAGE_ZH.get_or_init(|| build_install_page(true)).as_str()
    } else {
        PAGE_EN.get_or_init(|| build_install_page(false)).as_str()
    }
}

pub(super) async fn serve_install_page<S>(socket: &mut S, hub: &Hub, head: &Head) -> Result<()>
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

pub(super) async fn serve_ca_cert<S>(socket: &mut S, hub: &Hub, head: &Head) -> Result<()>
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

pub(super) async fn serve_pac<S>(
    socket: &mut S,
    local_addr: Option<SocketAddr>,
    hub: &Hub,
    head: &Head,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let is_head = head_method(head).eq_ignore_ascii_case("HEAD");
    let authority = header_value(&head.headers, "host")
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .or_else(|| local_addr.map(|addr| addr.to_string()))
        .unwrap_or_else(|| "127.0.0.1:8877".to_string());
    let script = hub.pac_script(&authority);
    write_untracked_encoded_for_method(
        socket,
        200,
        "OK",
        "application/x-ns-proxy-autoconfig; charset=utf-8",
        None,
        script.as_bytes(),
        is_head,
    )
    .await
}
