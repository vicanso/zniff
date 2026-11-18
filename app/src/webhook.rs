//! Loopback HTTP webhook execution for custom request/response mutation.
//!
//! Invokes only local HTTP endpoints (`127.0.0.1`, `localhost`, `[::1]`) on a
//! direct TCP + HTTP/1.1 connection (bypassing the zniff proxy) with bounded
//! payloads (`<= BODY_CAP`) and per-hook timeouts, failing open on errors.

use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use http::{Method, Request};
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::proxy::Hub;
use crate::rules::{HookSpec, parse_loopback_hook_url};
use crate::session::{BODY_CAP, base64_decode, base64_encode};

#[derive(Debug, Clone)]
pub struct RequestHookMutation {
    pub path: Option<String>,
    pub headers: Option<Vec<(String, String)>>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct ResponseHookMutation {
    pub status: Option<u16>,
    pub headers: Option<Vec<(String, String)>>,
    pub body: Option<Vec<u8>>,
}

struct LoopbackEndpoint {
    connect_host: String,
    port: u16,
    host_header: String,
    path_and_query: String,
}

fn parse_endpoint(raw_url: &str) -> Result<LoopbackEndpoint> {
    let normalized =
        parse_loopback_hook_url(raw_url).context("hook URL must target loopback http:// host")?;
    let rest = normalized
        .strip_prefix("http://")
        .context("hook URL must start with http://")?;
    let (authority, path_and_query) = match rest.find(['/', '?', '#']) {
        Some(idx) => {
            let p = &rest[idx..];
            let clean = p.split('#').next().unwrap_or(p);
            (
                &rest[..idx],
                if clean.is_empty() {
                    "/".to_string()
                } else {
                    clean.to_string()
                },
            )
        }
        None => (rest, "/".to_string()),
    };
    let (connect_host, port) = if let Some(after) = authority.strip_prefix("[::1]") {
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse::<u16>().context("invalid hook port")?,
            None => 80,
        };
        ("::1".to_string(), port)
    } else if let Some((host, port_str)) = authority.split_once(':') {
        let port = port_str.parse::<u16>().context("invalid hook port")?;
        let connect_host = if host.eq_ignore_ascii_case("localhost") {
            "127.0.0.1"
        } else {
            host
        };
        (connect_host.to_string(), port)
    } else {
        let connect_host = if authority.eq_ignore_ascii_case("localhost") {
            "127.0.0.1"
        } else {
            authority
        };
        (connect_host.to_string(), 80)
    };
    Ok(LoopbackEndpoint {
        connect_host,
        port,
        host_header: authority.to_string(),
        path_and_query,
    })
}

async fn post_hook_json(url: &str, body: Vec<u8>, limit: Duration) -> Result<Option<Value>> {
    let endpoint = parse_endpoint(url)?;
    let raw_resp = timeout(limit, async {
        let stream = TcpStream::connect((endpoint.connect_host.as_str(), endpoint.port))
            .await
            .with_context(|| format!("connect {}:{}", endpoint.connect_host, endpoint.port))?;
        let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .context("HTTP/1.1 handshake")?;
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let req = Request::builder()
            .method(Method::POST)
            .uri(&endpoint.path_and_query)
            .header("Host", &endpoint.host_header)
            .header("Content-Type", "application/json")
            .header("Content-Length", body.len().to_string())
            .header("Connection", "close")
            .body(Full::new(Bytes::from(body)))
            .context("build hook request")?;
        let resp = sender
            .send_request(req)
            .await
            .context("send hook request")?;
        let status = resp.status();
        let collected = resp
            .into_body()
            .collect()
            .await
            .context("read hook response body")?
            .to_bytes();
        if !status.is_success() {
            anyhow::bail!("HTTP {}", status.as_u16());
        }
        if collected.len() > BODY_CAP + 512 * 1024 {
            anyhow::bail!("hook response exceeds size cap");
        }
        Ok::<Vec<u8>, anyhow::Error>(collected.to_vec())
    })
    .await
    .map_err(|_| anyhow::anyhow!("hook timed out after {} ms", limit.as_millis()))??;

    let text = String::from_utf8(raw_resp).context("hook response is not UTF-8 JSON")?;
    if text.trim().is_empty() {
        return Ok(None);
    }
    let val: Value = serde_json::from_str(&text).context("parse hook response JSON")?;
    match val {
        Value::Null => Ok(None),
        Value::Object(map) if map.is_empty() => Ok(None),
        Value::Object(_) => Ok(Some(val)),
        _ => anyhow::bail!("hook response JSON must be an object or null"),
    }
}

fn parse_headers_override(val: &Value) -> Result<Vec<(String, String)>> {
    match val {
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let pair = item
                    .as_array()
                    .filter(|p| p.len() == 2)
                    .context("headers array items must be [name, value]")?;
                let name = pair[0].as_str().context("header name must be string")?;
                let value = pair[1].as_str().context("header value must be string")?;
                if !name.is_empty() {
                    out.push((name.to_string(), value.to_string()));
                }
            }
            Ok(out)
        }
        Value::Object(map) => {
            let mut out = Vec::with_capacity(map.len());
            for (name, v) in map {
                let value = v.as_str().context("header object value must be string")?;
                if !name.is_empty() {
                    out.push((name.clone(), value.to_string()));
                }
            }
            Ok(out)
        }
        _ => anyhow::bail!("headers override must be object or array of pairs"),
    }
}

fn parse_body_override(obj: &serde_json::Map<String, Value>) -> Result<Option<Vec<u8>>> {
    if let Some(b64) = obj.get("body_base64").and_then(Value::as_str) {
        let decoded = base64_decode(b64).context("invalid body_base64 in hook response")?;
        if decoded.len() > BODY_CAP {
            anyhow::bail!("hook body_base64 exceeds 1 MB");
        }
        return Ok(Some(decoded));
    }
    if let Some(text) = obj.get("body_utf8").and_then(Value::as_str) {
        if text.len() > BODY_CAP {
            anyhow::bail!("hook body_utf8 exceeds 1 MB");
        }
        return Ok(Some(text.as_bytes().to_vec()));
    }
    Ok(None)
}

fn encode_body_fields(body: &[u8]) -> (String, Value) {
    let slice = &body[..body.len().min(BODY_CAP)];
    let b64 = base64_encode(slice);
    let utf8 = match std::str::from_utf8(slice) {
        Ok(s) => Value::String(s.to_string()),
        Err(_) => Value::Null,
    };
    (b64, utf8)
}

fn header_pairs_json(headers: &[(String, String)]) -> Value {
    Value::Array(
        headers
            .iter()
            .map(|(k, v)| json!([k, v]))
            .collect::<Vec<_>>(),
    )
}

/// Runs matching request-phase `hook` specs in order, chaining mutations and
/// failing open (appending a flow comment) if any hook fails.
#[allow(clippy::too_many_arguments)]
pub async fn run_request_hooks(
    hub: &Hub,
    id: u64,
    specs: &[HookSpec],
    method: &str,
    scheme: &str,
    host: &str,
    authority: &str,
    initial_path: &str,
    initial_headers: &[(String, String)],
    initial_body: &[u8],
) -> Option<RequestHookMutation> {
    if !specs.iter().any(|s| s.phase.runs_request()) {
        return None;
    }
    let mut cur_path = initial_path.to_string();
    let mut cur_headers = initial_headers.to_vec();
    let mut cur_body = initial_body.to_vec();
    let mut changed_path = false;
    let mut changed_headers = false;
    let mut changed_body = false;
    let mut last_comment = None;

    for spec in specs.iter().filter(|s| s.phase.runs_request()) {
        let (body_b64, body_utf8) = encode_body_fields(&cur_body);
        let payload = json!({
            "id": id,
            "phase": "request",
            "method": method,
            "url": format!("{scheme}://{authority}{cur_path}"),
            "host": host,
            "path": cur_path,
            "headers": header_pairs_json(&cur_headers),
            "body_base64": body_b64,
            "body_utf8": body_utf8,
        });
        let Ok(bytes) = serde_json::to_vec(&payload) else {
            continue;
        };
        match post_hook_json(&spec.url, bytes, spec.timeout).await {
            Ok(Some(Value::Object(map))) => {
                if let Some(next_path) = map.get("path").and_then(Value::as_str)
                    && next_path.starts_with('/')
                {
                    cur_path = next_path.to_string();
                    changed_path = true;
                }
                if let Some(hdr_val) = map.get("headers") {
                    match parse_headers_override(hdr_val) {
                        Ok(next_hdrs) => {
                            cur_headers = next_hdrs;
                            changed_headers = true;
                        }
                        Err(err) => {
                            hub.append_comment(id, &format!("[hook failed: {err}]"));
                        }
                    }
                }
                match parse_body_override(&map) {
                    Ok(Some(next_body)) => {
                        cur_body = next_body;
                        changed_body = true;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        hub.append_comment(id, &format!("[hook failed: {err}]"));
                    }
                }
                if let Some(comment) = map.get("comment").and_then(Value::as_str)
                    && !comment.trim().is_empty()
                {
                    last_comment = Some(comment.trim().to_string());
                }
            }
            Ok(_) => {}
            Err(err) => hub.append_comment(id, &format!("[hook failed: {err}]")),
        }
    }

    if let Some(ref comment) = last_comment {
        hub.append_comment(id, comment);
    }
    if changed_path || changed_headers || changed_body {
        Some(RequestHookMutation {
            path: changed_path.then_some(cur_path),
            headers: changed_headers.then_some(cur_headers),
            body: changed_body.then_some(cur_body),
        })
    } else {
        None
    }
}

/// Runs matching response-phase `hook` specs in order, chaining mutations and
/// failing open (appending a flow comment) if any hook fails.
#[allow(clippy::too_many_arguments)]
pub async fn run_response_hooks(
    hub: &Hub,
    id: u64,
    specs: &[HookSpec],
    method: &str,
    url: &str,
    initial_status: u16,
    initial_headers: &[(String, String)],
    initial_body: &[u8],
) -> Option<ResponseHookMutation> {
    if !specs.iter().any(|s| s.phase.runs_response()) {
        return None;
    }
    let mut cur_status = initial_status;
    let mut cur_headers = initial_headers.to_vec();
    let mut cur_body = initial_body.to_vec();
    let mut changed_status = false;
    let mut changed_headers = false;
    let mut changed_body = false;
    let mut last_comment = None;

    for spec in specs.iter().filter(|s| s.phase.runs_response()) {
        let (body_b64, body_utf8) = encode_body_fields(&cur_body);
        let payload = json!({
            "id": id,
            "phase": "response",
            "method": method,
            "url": url,
            "status": cur_status,
            "headers": header_pairs_json(&cur_headers),
            "body_base64": body_b64,
            "body_utf8": body_utf8,
        });
        let Ok(bytes) = serde_json::to_vec(&payload) else {
            continue;
        };
        match post_hook_json(&spec.url, bytes, spec.timeout).await {
            Ok(Some(Value::Object(map))) => {
                if let Some(code) = map.get("status").and_then(Value::as_u64)
                    && (100..600).contains(&code)
                {
                    cur_status = code as u16;
                    changed_status = true;
                }
                if let Some(hdr_val) = map.get("headers") {
                    match parse_headers_override(hdr_val) {
                        Ok(next_hdrs) => {
                            cur_headers = next_hdrs;
                            changed_headers = true;
                        }
                        Err(err) => {
                            hub.append_comment(id, &format!("[hook failed: {err}]"));
                        }
                    }
                }
                match parse_body_override(&map) {
                    Ok(Some(next_body)) => {
                        cur_body = next_body;
                        changed_body = true;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        hub.append_comment(id, &format!("[hook failed: {err}]"));
                    }
                }
                if let Some(comment) = map.get("comment").and_then(Value::as_str)
                    && !comment.trim().is_empty()
                {
                    last_comment = Some(comment.trim().to_string());
                }
            }
            Ok(_) => {}
            Err(err) => hub.append_comment(id, &format!("[hook failed: {err}]")),
        }
    }

    if let Some(ref comment) = last_comment {
        hub.append_comment(id, comment);
    }
    if changed_status || changed_headers || changed_body {
        Some(ResponseHookMutation {
            status: changed_status.then_some(cur_status),
            headers: changed_headers.then_some(cur_headers),
            body: changed_body.then_some(cur_body),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_endpoint_accepts_loopback_forms_and_rejects_remote() {
        let ep1 = parse_endpoint("http://127.0.0.1:9100/mutate?x=1").unwrap();
        assert_eq!(ep1.connect_host, "127.0.0.1");
        assert_eq!(ep1.port, 9100);
        assert_eq!(ep1.path_and_query, "/mutate?x=1");

        let ep2 = parse_endpoint("http://localhost:8085").unwrap();
        assert_eq!(ep2.connect_host, "127.0.0.1");
        assert_eq!(ep2.port, 8085);
        assert_eq!(ep2.path_and_query, "/");

        let ep3 = parse_endpoint("http://[::1]:9200/hook").unwrap();
        assert_eq!(ep3.connect_host, "::1");
        assert_eq!(ep3.port, 9200);
        assert_eq!(ep3.path_and_query, "/hook");

        assert!(parse_endpoint("http://api.vicanso.dev/hook").is_err());
    }
}
