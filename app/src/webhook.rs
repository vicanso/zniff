//! Loopback HTTP webhook execution for custom request/response mutation.
//!
//! Invokes only local HTTP endpoints (`127.0.0.1`, `localhost`, `[::1]`) on a
//! direct TCP + HTTP/1.1 connection (bypassing the zniff proxy) with bounded
//! payloads (`<= BODY_CAP`) and per-hook timeouts, failing open on errors.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use http::{Method, Request};
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::hook_bridge::HookSessionHost;
use crate::hook_runtime::{
    HookPreviewInputOwned, HookRequestInputOwned, HookResponseInputOwned, ScriptRequestAction,
    eval_preview_hook_async, eval_request_hook_async, eval_response_hook_async,
    resolve_script_hook,
};
use crate::hook_spec::{PreviewCardNode, ScriptHookConfig};
use crate::proxy::Hub;
use crate::rules::{HookMode, HookSpec, HookTarget, parse_loopback_hook_url};
use crate::session::{BODY_CAP, base64_decode, base64_encode};

#[derive(Debug, Clone)]
pub struct RequestHookMutation {
    pub method: Option<String>,
    pub path: Option<String>,
    pub headers: Option<Vec<(String, String)>>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct SyntheticHookResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum RequestHookOutcome {
    Mutate(RequestHookMutation),
    RespondWith(SyntheticHookResponse),
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

/// Runs matching request-phase `hook` specs in order, chaining mutations or
/// returning a short-circuit `RespondWith` response, failing open if any hook fails.
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
) -> Option<RequestHookOutcome> {
    if !specs.iter().any(|s| s.phase.runs_request()) {
        return None;
    }
    let mut cur_method = method.to_string();
    let mut cur_path = initial_path.to_string();
    let mut cur_headers = initial_headers.to_vec();
    let mut cur_body = initial_body.to_vec();
    let mut changed_method = false;
    let mut changed_path = false;
    let mut changed_headers = false;
    let mut changed_body = false;
    let mut last_comment = None;
    let configured_hooks = hub.script_hooks();
    let data_dir = hub.ca.dir().to_path_buf();

    for spec in specs.iter().filter(|s| s.phase.runs_request()) {
        match &spec.target {
            HookTarget::Http(url) => {
                let (body_b64, body_utf8) = encode_body_fields(&cur_body);
                let payload = json!({
                    "id": id,
                    "phase": "request",
                    "method": cur_method,
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
                match post_hook_json(url, bytes, spec.timeout).await {
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
            HookTarget::Script { id_or_file, vars } => {
                let timeout_ms = spec.timeout.as_millis().max(10) as u64;
                let resolved = match resolve_script_hook(
                    &data_dir,
                    &configured_hooks,
                    id_or_file,
                    vars,
                    timeout_ms,
                ) {
                    Ok(r) => r,
                    Err(err) => {
                        hub.append_comment(id, &format!("[script hook failed: {err}]"));
                        continue;
                    }
                };
                let url = format!("{scheme}://{authority}{cur_path}");
                let input = HookRequestInputOwned {
                    flow_id: id,
                    method: cur_method.clone(),
                    scheme: scheme.to_string(),
                    host: host.to_string(),
                    url,
                    path: cur_path.clone(),
                    headers: cur_headers.clone(),
                    body: cur_body.clone(),
                };
                match eval_request_hook_async(hub.hook_session_host(), resolved, input).await {
                    Ok(ScriptRequestAction::Pass { comment }) => {
                        if let Some(c) = comment {
                            last_comment = Some(c);
                        }
                    }
                    Ok(ScriptRequestAction::RespondWith {
                        status,
                        headers,
                        body,
                        comment,
                    }) => {
                        if let Some(c) = comment.or(last_comment) {
                            hub.append_comment(id, &c);
                        }
                        return Some(RequestHookOutcome::RespondWith(SyntheticHookResponse {
                            status,
                            headers,
                            body,
                        }));
                    }
                    Ok(ScriptRequestAction::Mutate {
                        method: next_method,
                        path: next_path,
                        headers: next_headers,
                        body: next_body,
                        comment,
                    }) => {
                        if let Some(m) = next_method {
                            cur_method = m;
                            changed_method = true;
                        }
                        if let Some(p) = next_path {
                            cur_path = p;
                            changed_path = true;
                        }
                        if let Some(h) = next_headers {
                            cur_headers = h;
                            changed_headers = true;
                        }
                        if let Some(b) = next_body {
                            cur_body = b;
                            changed_body = true;
                        }
                        if let Some(c) = comment {
                            last_comment = Some(c);
                        }
                    }
                    Err(err) => hub.append_comment(id, &format!("[script hook failed: {err}]")),
                }
            }
        }
    }

    if let Some(ref comment) = last_comment {
        hub.append_comment(id, comment);
    }
    if changed_method || changed_path || changed_headers || changed_body {
        Some(RequestHookOutcome::Mutate(RequestHookMutation {
            method: changed_method.then_some(cur_method),
            path: changed_path.then_some(cur_path),
            headers: changed_headers.then_some(cur_headers),
            body: changed_body.then_some(cur_body),
        }))
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
    let configured_hooks = hub.script_hooks();
    let data_dir = hub.ca.dir().to_path_buf();
    let (host, path) = split_url_host_path(url);

    for spec in specs.iter().filter(|s| s.phase.runs_response()) {
        match &spec.target {
            HookTarget::Http(hook_url) => {
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
                match post_hook_json(hook_url, bytes, spec.timeout).await {
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
            HookTarget::Script { id_or_file, vars } => {
                let timeout_ms = spec.timeout.as_millis().max(10) as u64;
                let resolved = match resolve_script_hook(
                    &data_dir,
                    &configured_hooks,
                    id_or_file,
                    vars,
                    timeout_ms,
                ) {
                    Ok(r) => r,
                    Err(err) => {
                        hub.append_comment(id, &format!("[script hook failed: {err}]"));
                        continue;
                    }
                };
                let input = HookResponseInputOwned {
                    flow_id: id,
                    method: method.to_string(),
                    host: host.clone(),
                    path: path.clone(),
                    url: url.to_string(),
                    status: cur_status,
                    headers: cur_headers.clone(),
                    body: cur_body.clone(),
                };
                match eval_response_hook_async(hub.hook_session_host(), resolved, input).await {
                    Ok(res) => {
                        if let Some(code) = res.status {
                            cur_status = code;
                            changed_status = true;
                        }
                        if let Some(hdrs) = res.headers {
                            cur_headers = hdrs;
                            changed_headers = true;
                        }
                        if let Some(body) = res.body {
                            cur_body = body;
                            changed_body = true;
                        }
                        if let Some(c) = res.comment {
                            last_comment = Some(c);
                        }
                    }
                    Err(err) => hub.append_comment(id, &format!("[script hook failed: {err}]")),
                }
            }
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

fn split_url_host_path(url: &str) -> (String, String) {
    let rest = url.split_once("://").map(|(_, after)| after).unwrap_or(url);
    match rest.find(['/', '?']) {
        Some(idx) => {
            let auth = &rest[..idx];
            let host = auth
                .strip_prefix('[')
                .and_then(|s| s.split_once(']'))
                .map(|(h, _)| h)
                .unwrap_or_else(|| auth.split(':').next().unwrap_or(auth));
            (host.to_string(), rest[idx..].to_string())
        }
        None => {
            let host = rest.split(':').next().unwrap_or(rest);
            (host.to_string(), "/".to_string())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewHookResult {
    pub body: Vec<u8>,
    pub content_type: Option<String>,
    pub note: Option<String>,
    pub cards: Vec<PreviewCardNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewLookup {
    Pending,
    Ready(Option<PreviewHookResult>),
}

const PREVIEW_CACHE_CAP: usize = 64;

type PreviewCacheKey = (u64, bool, u64, usize);

fn preview_cache() -> &'static Mutex<Vec<(PreviewCacheKey, PreviewLookup)>> {
    static CACHE: OnceLock<Mutex<Vec<(PreviewCacheKey, PreviewLookup)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

pub(crate) fn clear_preview_cache() {
    preview_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}

pub(crate) struct PreviewSpawnInput {
    pub cache_key: PreviewCacheKey,
    pub specs: Vec<HookSpec>,
    pub flow_id: u64,
    pub is_response: bool,
    pub method: String,
    pub host: String,
    pub path: String,
    pub url: String,
    pub status: Option<u16>,
    pub req_headers: Vec<(String, String)>,
    pub req_body: Vec<u8>,
    pub res_headers: Vec<(String, String)>,
    pub res_body: Vec<u8>,
    pub data_dir: PathBuf,
    pub configured_hooks: Vec<ScriptHookConfig>,
    pub host_session: HookSessionHost,
}

fn preview_runtime() -> Option<&'static tokio::runtime::Runtime> {
    static RT: OnceLock<Option<tokio::runtime::Runtime>> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("zniff-hook-preview")
            .enable_all()
            .build()
            .ok()
    })
    .as_ref()
}

fn parse_preview_body_override(
    map: &serde_json::Map<String, Value>,
) -> Result<(Option<Vec<u8>>, Option<String>)> {
    let explicit_ct = map
        .get("content_type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if let Some(b64) = map.get("body_b64").and_then(Value::as_str) {
        let decoded = base64_decode(b64).context("invalid base64 in body_b64")?;
        if decoded.len() > BODY_CAP {
            anyhow::bail!("hook body_b64 exceeds 1 MB limit");
        }
        return Ok((Some(decoded), explicit_ct));
    }
    if let Some(body_val) = map.get("body") {
        if let Some(text) = body_val.as_str() {
            let bytes = text.as_bytes().to_vec();
            if bytes.len() > BODY_CAP {
                anyhow::bail!("hook body exceeds 1 MB limit");
            }
            return Ok((Some(bytes), explicit_ct));
        }
        if body_val.is_object() || body_val.is_array() {
            let bytes = serde_json::to_vec_pretty(body_val).context("serialize JSON body")?;
            if bytes.len() > BODY_CAP {
                anyhow::bail!("hook body exceeds 1 MB limit");
            }
            return Ok((
                Some(bytes),
                explicit_ct.or_else(|| Some("application/json".to_string())),
            ));
        }
    }
    Ok((None, explicit_ct))
}

pub(crate) struct PreviewScriptEnv<'a> {
    pub data_dir: &'a Path,
    pub configured_hooks: &'a [ScriptHookConfig],
    pub host_session: &'a HookSessionHost,
    pub host: &'a str,
    pub path: &'a str,
    pub req_headers: &'a [(String, String)],
    pub req_body: &'a [u8],
    pub res_headers: &'a [(String, String)],
    pub res_body: &'a [u8],
}

/// Runs all matching read-only `mode=preview` hooks sequentially without mutating proxied traffic.
pub(crate) async fn run_preview_hooks(
    specs: &[HookSpec],
    id: u64,
    is_response: bool,
    method: &str,
    url: &str,
    status: Option<u16>,
    headers: &[(String, String)],
    body: &[u8],
    script_env: Option<PreviewScriptEnv<'_>>,
) -> Option<PreviewHookResult> {
    let mut cur_body = body.to_vec();
    let mut cur_ct: Option<String> = None;
    let mut last_note: Option<String> = None;
    let mut cards: Vec<PreviewCardNode> = Vec::new();
    let mut changed_body = false;

    for spec in specs {
        if spec.mode != HookMode::Preview {
            continue;
        }
        let phase_ok = if is_response {
            spec.phase.runs_response()
        } else {
            spec.phase.runs_request()
        };
        if !phase_ok {
            continue;
        }
        match &spec.target {
            HookTarget::Http(hook_url) => {
                let payload = json!({
                    "id": id,
                    "phase": if is_response { "res" } else { "req" },
                    "mode": "preview",
                    "method": method,
                    "url": url,
                    "status": status,
                    "headers": headers.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
                    "body_b64": base64_encode(&cur_body),
                });
                let Ok(bytes) = serde_json::to_vec(&payload) else {
                    continue;
                };
                if let Ok(Some(Value::Object(map))) =
                    post_hook_json(hook_url, bytes, spec.timeout).await
                {
                    if let Ok((Some(next_body), next_ct)) = parse_preview_body_override(&map) {
                        cur_body = next_body;
                        if next_ct.is_some() {
                            cur_ct = next_ct;
                        }
                        changed_body = true;
                    }
                    if let Some(note) = map
                        .get("note")
                        .or_else(|| map.get("comment"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        last_note = Some(note.to_string());
                    }
                }
            }
            HookTarget::Script { id_or_file, vars } => {
                let Some(ref env) = script_env else {
                    continue;
                };
                let timeout_ms = spec.timeout.as_millis().max(200) as u64;
                let Ok(resolved) = resolve_script_hook(
                    env.data_dir,
                    env.configured_hooks,
                    id_or_file,
                    vars,
                    timeout_ms,
                ) else {
                    continue;
                };
                let input = HookPreviewInputOwned {
                    flow_id: id,
                    is_response,
                    method: method.to_string(),
                    host: env.host.to_string(),
                    path: env.path.to_string(),
                    url: url.to_string(),
                    status,
                    req_headers: env.req_headers.to_vec(),
                    req_body: if is_response {
                        env.req_body.to_vec()
                    } else {
                        cur_body.clone()
                    },
                    res_headers: env.res_headers.to_vec(),
                    res_body: if is_response {
                        cur_body.clone()
                    } else {
                        env.res_body.to_vec()
                    },
                };
                if let Ok(res) =
                    eval_preview_hook_async(env.host_session.clone(), resolved, input).await
                {
                    if let Some(next_body) = res.side.body {
                        cur_body = next_body;
                        changed_body = true;
                    }
                    if let Some(next_ct) = res.side.content_type {
                        cur_ct = Some(next_ct);
                    }
                    if let Some(note) = res.side.note {
                        last_note = Some(note);
                    }
                    for card in res.cards {
                        if !cards.contains(&card) {
                            cards.push(card);
                        }
                    }
                }
            }
        }
    }

    if changed_body || !cards.is_empty() {
        Some(PreviewHookResult {
            body: if changed_body { cur_body } else { Vec::new() },
            content_type: cur_ct,
            note: last_note,
            cards,
        })
    } else {
        None
    }
}

/// Looks up cached preview-hook output or spawns a background request on the preview runtime.
pub(crate) fn lookup_or_spawn_preview(
    input: PreviewSpawnInput,
    on_done: impl FnOnce() + Send + 'static,
) -> PreviewLookup {
    let cache_key = input.cache_key;
    {
        let mut cache = preview_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, entry)) = cache.iter().find(|(k, _)| *k == cache_key) {
            return entry.clone();
        }
        if cache.len() >= PREVIEW_CACHE_CAP {
            cache.remove(0);
        }
        cache.push((cache_key, PreviewLookup::Pending));
    }

    let Some(rt) = preview_runtime() else {
        return PreviewLookup::Ready(None);
    };
    rt.spawn(async move {
        let headers = if input.is_response {
            &input.res_headers
        } else {
            &input.req_headers
        };
        let body = if input.is_response {
            &input.res_body
        } else {
            &input.req_body
        };
        let env = PreviewScriptEnv {
            data_dir: &input.data_dir,
            configured_hooks: &input.configured_hooks,
            host_session: &input.host_session,
            host: &input.host,
            path: &input.path,
            req_headers: &input.req_headers,
            req_body: &input.req_body,
            res_headers: &input.res_headers,
            res_body: &input.res_body,
        };
        let result = run_preview_hooks(
            &input.specs,
            input.flow_id,
            input.is_response,
            &input.method,
            &input.url,
            input.status,
            headers,
            body,
            Some(env),
        )
        .await;
        {
            let mut cache = preview_cache().lock().unwrap_or_else(|e| e.into_inner());
            if let Some((_, entry)) = cache.iter_mut().find(|(k, _)| *k == cache_key) {
                *entry = PreviewLookup::Ready(result);
            } else {
                if cache.len() >= PREVIEW_CACHE_CAP {
                    cache.remove(0);
                }
                cache.push((cache_key, PreviewLookup::Ready(result)));
            }
        }
        on_done();
    });
    PreviewLookup::Pending
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

    #[tokio::test]
    async fn preview_hook_decodes_json_object_without_mutating_origin() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let resp_body =
                    r#"{"body":{"decrypted":true,"host":"api.vicanso.dev"},"note":"AES-256-GCM"}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp_body.len(),
                    resp_body
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            }
        });

        let spec = HookSpec {
            target: HookTarget::Http(format!("http://127.0.0.1:{port}/preview")),
            phase: crate::rules::HookPhase::Response,
            mode: HookMode::Preview,
            timeout: Duration::from_secs(2),
        };
        let decoded = run_preview_hooks(
            &[spec],
            7,
            true,
            "POST",
            "https://api.vicanso.dev/v1/secret",
            Some(200),
            &[("Content-Type".into(), "application/octet-stream".into())],
            b"\x01\x02\x03ciphertext",
            None,
        )
        .await
        .expect("preview hook should return decoded body");
        assert_eq!(decoded.content_type.as_deref(), Some("application/json"));
        assert_eq!(decoded.note.as_deref(), Some("AES-256-GCM"));
        let parsed: Value = serde_json::from_slice(&decoded.body).unwrap();
        assert_eq!(parsed["decrypted"], true);
        assert_eq!(parsed["host"], "api.vicanso.dev");
    }
}
