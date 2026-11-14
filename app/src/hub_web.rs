//! Web-view stand-in for [`crate::proxy::Hub`].
//!
//! The browser does not capture. Mutations are posted to the server process,
//! which is the only place `Capture` listens. Reads use a session cache that
//! the view fills from `/api/state` and `/api/flows`.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::anyhow;
use serde_json::{Value, json};

use crate::ca::Ca;
use crate::session::{
    CachedFlowDetail, CachedFlowView, ConnInfo, Direction, Message, Protocol, Session,
};

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
    pub status: Option<u16>,
}

#[derive(Debug, Default, Clone)]
pub struct ReplayOverride {
    pub method: Option<String>,
    pub url: Option<String>,
    pub headers: Option<Vec<(String, String)>>,
    pub body: Option<Vec<u8>>,
}

/// Present so the view can name it. The browser never binds the capture port.
pub struct Capture {
    pub addr: SocketAddr,
}

impl Capture {
    pub fn stop(self) {}
}

pub struct Hub {
    session: Mutex<Session>,
    generation: AtomicU64,
    pub ca: Arc<Ca>,
    bypass_text: Mutex<String>,
    rules_text: Mutex<String>,
    rules_rev: AtomicU64,
    upstream_text: Mutex<String>,
    proxy_auth: Mutex<String>,
    rules_on: AtomicBool,
    breaks_on: AtomicBool,
    show_tunnels: AtomicBool,
    paused_queue: Mutex<Vec<(u64, BreakSide)>>,
    share_id: Mutex<Option<String>>,
    relay_accept: AtomicBool,
    relay_token: Mutex<String>,
}

impl Hub {
    pub fn new(ca: Arc<Ca>) -> anyhow::Result<Self> {
        Ok(Self {
            session: Mutex::new(Session::default()),
            generation: AtomicU64::new(1),
            ca,
            bypass_text: Mutex::new(String::new()),
            rules_text: Mutex::new(String::new()),
            rules_rev: AtomicU64::new(0),
            upstream_text: Mutex::new(String::new()),
            proxy_auth: Mutex::new(String::new()),
            rules_on: AtomicBool::new(true),
            breaks_on: AtomicBool::new(true),
            show_tunnels: AtomicBool::new(false),
            paused_queue: Mutex::new(Vec::new()),
            share_id: Mutex::new(None),
            relay_accept: AtomicBool::new(false),
            relay_token: Mutex::new(String::new()),
        })
    }

    pub fn apply_state(&self, value: &Value) {
        if let Some(id) = value.get("share_id").and_then(Value::as_str) {
            *self.share_id.lock().unwrap_or_else(|err| err.into_inner()) = if id.is_empty() {
                None
            } else {
                Some(id.to_string())
            };
        }
        if let Some(queue) = value.get("paused_queue").and_then(Value::as_array) {
            *self
                .paused_queue
                .lock()
                .unwrap_or_else(|err| err.into_inner()) =
                queue.iter().filter_map(parse_paused).collect();
        }
        if let Some(flows) = value.get("flows").and_then(Value::as_u64) {
            self.with_session(|session| session.set_cached_total(Some(flows as usize)));
        }
        if let Some(accept) = value
            .get("prefs")
            .and_then(|prefs| prefs.get("relay_accept"))
            .and_then(Value::as_bool)
        {
            self.relay_accept.store(accept, Ordering::Relaxed);
        }
        if let Some(auth) = value
            .get("prefs")
            .and_then(|prefs| prefs.get("proxy_auth"))
            .and_then(Value::as_str)
        {
            *self
                .proxy_auth
                .lock()
                .unwrap_or_else(|err| err.into_inner()) = auth.trim().to_string();
        }
    }

    pub fn apply_flows(&self, value: &Value) {
        let Some(flows) = value.get("flows").and_then(Value::as_array) else {
            return;
        };
        let session_total = value
            .get("session_total")
            .or_else(|| value.get("total"))
            .and_then(Value::as_u64)
            .map(|count| count as usize);
        let rows = flows.iter().filter_map(parse_view).collect::<Vec<_>>();
        self.with_session(|session| {
            session.replace_cached_views(rows);
            if let Some(total) = session_total {
                session.set_cached_total(Some(total));
            }
        });
        self.bump();
    }

    pub fn fill_detail(&self, value: &Value) -> Option<u64> {
        let detail = parse_detail(value)?;
        let rev = self.with_session(|session| session.fill_cached_detail(detail))?;
        self.bump();
        Some(rev)
    }

    pub fn share_id(&self) -> Option<String> {
        self.share_id
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub fn paused(&self) -> Option<(u64, BreakSide)> {
        self.paused_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .first()
            .copied()
    }

    pub fn paused_queue_len(&self) -> usize {
        self.paused_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .len()
    }

    pub fn paused_queue(&self) -> Vec<(u64, BreakSide)> {
        self.paused_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    pub fn with_session<R>(&self, f: impl FnOnce(&mut Session) -> R) -> R {
        let mut session = self.session.lock().unwrap_or_else(|err| err.into_inner());
        f(&mut session)
    }

    pub fn matching_ids(&self, filter: &str) -> (Vec<(u64, Option<SystemTime>)>, u64) {
        let (plan, rev) = self.with_session(|session| (session.plan_filter(filter), session.rev()));
        (plan.finish(), rev)
    }

    pub fn set_bypass(&self, text: &str) {
        *self
            .bypass_text
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = text.to_string();
        post_op(json!({"op": "set_bypass", "text": text}));
    }

    pub fn set_rules(&self, text: &str) {
        *self
            .rules_text
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = text.to_string();
        self.rules_rev.fetch_add(1, Ordering::Relaxed);
        post_op(json!({"op": "set_rules", "text": text}));
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
        self.rules_on.store(enabled, Ordering::Relaxed);
        post_op(json!({"op": "set_rules_enabled", "on": enabled}));
    }

    pub fn set_breaks_enabled(&self, enabled: bool) {
        self.breaks_on.store(enabled, Ordering::Relaxed);
        post_op(json!({"op": "set_breaks_enabled", "on": enabled}));
    }

    pub fn set_show_tunnel_failures(&self, show: bool) {
        self.show_tunnels.store(show, Ordering::Relaxed);
        post_op(json!({"op": "set_show_tunnel_failures", "on": show}));
    }

    pub fn set_upstream(&self, text: &str) {
        *self
            .upstream_text
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = text.to_string();
        post_op(json!({"op": "set_upstream", "text": text}));
    }

    pub fn set_proxy_auth(&self, auth: &str) {
        let trimmed = auth.trim().to_string();
        *self
            .proxy_auth
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = trimmed.clone();
        post_op(json!({"op": "set_proxy_auth", "text": trimmed}));
    }

    pub fn proxy_auth_text(&self) -> String {
        self.proxy_auth
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub fn set_relay_ingress(&self, accept: bool, token: &str) {
        self.set_relay_token(token);
        self.set_relay_accept(accept);
    }

    pub fn set_relay_token(&self, token: &str) {
        *self
            .relay_token
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = token.trim().to_string();
        post_op(json!({"op": "set_relay_token", "text": token.trim()}));
    }

    pub fn set_relay_accept(&self, accept: bool) {
        self.relay_accept.store(accept, Ordering::Relaxed);
        post_op(json!({"op": "set_relay_accept", "on": accept}));
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

    pub fn set_pinned(&self, id: u64, pinned: bool) -> bool {
        let ok = self.with_session(|session| session.set_pinned(id, pinned));
        if ok {
            self.bump();
            post_op(json!({"op": "set_pinned", "id": id, "on": pinned}));
        }
        ok
    }

    pub fn set_comment(&self, id: u64, comment: impl Into<String>) -> bool {
        let comment = comment.into();
        let ok = self.with_session(|session| session.set_comment(id, comment.clone()));
        if ok {
            self.bump();
            post_op(json!({"op": "set_comment", "id": id, "text": comment}));
        }
        ok
    }

    pub fn resolve_break(&self, decision: BreakDecision) -> bool {
        let mut queue = self
            .paused_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if queue.is_empty() {
            return false;
        }
        queue.remove(0);
        drop(queue);
        self.bump();
        let (body_utf8, body_b64) = encode_body_fields(decision.body.as_deref());
        post_op(json!({
            "op": "resolve_break",
            "drop": decision.drop,
            "status": decision.status,
            "headers": decision.headers,
            "body": body_utf8,
            "body_b64": body_b64,
        }));
        true
    }

    pub fn resolve_all_breaks(&self) -> usize {
        let count = self
            .paused_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .drain(..)
            .count();
        if count > 0 {
            self.bump();
            post_op(json!({"op": "resolve_all_breaks"}));
        }
        count
    }

    pub fn promote_break(&self, id: u64) -> bool {
        let mut queue = self
            .paused_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let Some(index) = queue.iter().position(|(slot, _)| *slot == id) else {
            return false;
        };
        let item = queue.remove(index);
        queue.insert(0, item);
        drop(queue);
        self.bump();
        post_op(json!({"op": "promote_break", "id": id}));
        true
    }

    pub fn touch_break(&self) {
        post_op(json!({"op": "touch_break"}));
    }

    pub fn clear_session(&self) {
        self.with_session(|session| session.clear());
        self.bump();
        post_op(json!({"op": "clear"}));
    }

    pub fn clear_unpinned(&self) -> usize {
        let removed = self.with_session(|session| session.clear_unpinned());
        if removed > 0 {
            self.bump();
            post_op(json!({"op": "clear_unpinned"}));
        }
        removed
    }

    pub fn begin_share_with_redact(
        &self,
        ttl: Duration,
        filter: &str,
        redact: crate::share::RedactOptions,
    ) -> (String, SystemTime) {
        let _ = (ttl, filter, redact);
        (String::new(), crate::clock::now())
    }

    pub fn end_share(&self) {
        *self.share_id.lock().unwrap_or_else(|err| err.into_inner()) = None;
        self.bump();
        post_op(json!({"op": "end_share"}));
    }

    #[allow(dead_code)]
    pub fn export_har_matching_with_redact(
        &self,
        filter: &str,
        redact: &crate::share::RedactOptions,
    ) -> Value {
        let (matched, _) = self.matching_ids(filter);
        let flows = self.with_session(|session| {
            matched
                .into_iter()
                .filter_map(|(id, _)| session.hydrated(id))
                .collect::<Vec<_>>()
        });
        crate::har::export_har_with_redact(&flows, redact)
    }

    #[allow(dead_code)]
    pub fn import_har(&self, value: &Value) -> usize {
        let count = crate::har::import_har(value).len();
        post_op(json!({"op": "import_har", "value": value}));
        count
    }

    pub fn replay_later<F>(self: &Arc<Self>, id: u64, edit: ReplayOverride, on_done: F)
    where
        F: FnOnce(anyhow::Result<u64>) + Send + 'static,
    {
        if let Err(err) = self.replay_guard(Some(id)) {
            on_done(Err(err));
            return;
        }
        let (body_utf8, body_b64) = encode_body_fields(edit.body.as_deref());
        let body = json!({
            "op": "replay",
            "id": id,
            "method": edit.method,
            "url": edit.url,
            "headers": edit.headers,
            "body": body_utf8,
            "body_b64": body_b64,
        });
        spawn_result(body, on_done);
    }

    pub fn replay_batch_later<F>(
        self: &Arc<Self>,
        id: u64,
        count: u32,
        concurrency: u32,
        interval_ms: u64,
        edit: ReplayOverride,
        on_done: F,
    ) where
        F: FnOnce(anyhow::Result<Vec<u64>>) + Send + 'static,
    {
        if let Err(err) = self.replay_guard(Some(id)) {
            on_done(Err(err));
            return;
        }
        let (body_utf8, body_b64) = encode_body_fields(edit.body.as_deref());
        let body = json!({
            "op": "replay_batch",
            "id": id,
            "count": count,
            "concurrency": concurrency,
            "interval_ms": interval_ms,
            "method": edit.method,
            "url": edit.url,
            "headers": edit.headers,
            "body": body_utf8,
            "body_b64": body_b64,
        });
        spawn_batch(body, on_done);
    }

    pub fn send_composed_later<F>(self: &Arc<Self>, edit: ReplayOverride, on_done: F)
    where
        F: FnOnce(anyhow::Result<u64>) + Send + 'static,
    {
        let (body_utf8, body_b64) = encode_body_fields(edit.body.as_deref());
        let body = json!({
            "op": "send_composed",
            "method": edit.method,
            "url": edit.url,
            "headers": edit.headers,
            "body": body_utf8,
            "body_b64": body_b64,
        });
        spawn_result(body, on_done);
    }

    fn replay_guard(&self, id: Option<u64>) -> anyhow::Result<()> {
        let Some(id) = id else {
            return Ok(());
        };
        let flow = self.with_session(|session| session.hydrated(id));
        let Some(flow) = flow else {
            return Err(anyhow!("没有这条记录"));
        };
        if flow.protocol == Protocol::Tunnel || flow.method.eq_ignore_ascii_case("CONNECT") {
            return Err(anyhow!("隧道不能重放"));
        }
        if flow.relay_source.is_some() {
            return Err(anyhow!("中继流量只能查看"));
        }
        Ok(())
    }

    fn bump(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
}

fn encode_body_fields(bytes: Option<&[u8]>) -> (Option<String>, Option<String>) {
    let Some(bytes) = bytes else {
        return (None, None);
    };
    let utf8 = std::str::from_utf8(bytes).ok().map(str::to_string);
    let b64 = Some(crate::session::base64_encode(bytes));
    (utf8, b64)
}

pub fn sync_web_config_diff(before: &crate::config::Config, after: &crate::config::Config) {
    if before.saved_profiles != after.saved_profiles
        || before.profile_history != after.profile_history
        || before.active_profile != after.active_profile
    {
        post_op(json!({
            "op": "sync_profiles",
            "saved_profiles": after.saved_profiles,
            "profile_history": after.profile_history,
            "active_profile": after.active_profile,
        }));
    }
    if before.filter != after.filter {
        post_op(json!({
            "op": "set_filter",
            "text": after.filter,
        }));
    }
    if before.redact_export != after.redact_export
        || before.redact_strict != after.redact_strict
        || before.redact_fields != after.redact_fields
    {
        post_op(json!({
            "op": "set_redact",
            "on": after.redact_export,
            "strict": after.redact_strict,
            "fields": after.redact_fields,
        }));
    }
    if before.relay_url != after.relay_url {
        post_op(json!({
            "op": "set_relay_url",
            "text": after.relay_url,
        }));
    }
}

fn web_base_slot() -> &'static Mutex<String> {
    static BASE: Mutex<String> = Mutex::new(String::new());
    &BASE
}

pub fn note_web_state_url(final_url: &str) {
    let clean = final_url.split(['?', '#']).next().unwrap_or("");
    if let Some(base) = clean.strip_suffix("api/state")
        && (base.starts_with("http://") || base.starts_with("https://"))
        && base.ends_with('/')
    {
        let mut guard = web_base_slot()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if guard.as_str() != base {
            *guard = base.to_string();
        }
    }
}

pub async fn resolve_web_api_url(rel_path: &str) -> Option<String> {
    let rel = rel_path.trim_start_matches('/');
    {
        let guard = web_base_slot()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if !guard.is_empty() {
            return Some(format!("{}{rel}", guard.as_str()));
        }
    }
    if let Some(page) = nsh_core::prelude::app_url().await {
        let clean = page.split(['?', '#']).next().unwrap_or("");
        if clean.starts_with("http://") || clean.starts_with("https://") {
            let base = match clean.rfind('/') {
                Some(idx) if idx > "https://".len() => &clean[..=idx],
                _ => return Some(format!("{clean}/{rel}")),
            };
            return Some(format!("{base}{rel}"));
        }
    }
    None
}

static LAST_SAVE_ERROR: Mutex<Option<String>> = Mutex::new(None);

pub fn take_save_error() -> Option<String> {
    LAST_SAVE_ERROR
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
}

fn post_op(value: Value) {
    let Ok(body) = serde_json::to_vec(&value) else {
        return;
    };
    nsh_core::prelude::spawn(async move {
        let request = nsh_core::prelude::HttpRequest::post("api/hub", body)
            .header("content-type", "application/json");
        if let Ok(response) = nsh_core::prelude::fetch(request).await
            && let Ok(parsed) = serde_json::from_str::<Value>(&response.text())
            && let Some(err) = parsed
                .get("save_error")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
        {
            *LAST_SAVE_ERROR
                .lock()
                .unwrap_or_else(|err| err.into_inner()) = Some(err.to_string());
        }
    });
}

fn spawn_result<F>(body: Value, on_done: F)
where
    F: FnOnce(anyhow::Result<u64>) + Send + 'static,
{
    let Ok(bytes) = serde_json::to_vec(&body) else {
        on_done(Err(anyhow!("请求没有编码")));
        return;
    };
    nsh_core::prelude::spawn(async move {
        on_done(post_id(bytes).await);
    });
}

fn spawn_batch<F>(body: Value, on_done: F)
where
    F: FnOnce(anyhow::Result<Vec<u64>>) + Send + 'static,
{
    let Ok(bytes) = serde_json::to_vec(&body) else {
        on_done(Err(anyhow!("请求没有编码")));
        return;
    };
    nsh_core::prelude::spawn(async move {
        on_done(post_ids(bytes).await);
    });
}

async fn post_id(bytes: Vec<u8>) -> anyhow::Result<u64> {
    let value = post_json(bytes).await?;
    let id = value.get("id").and_then(Value::as_u64);
    id.ok_or_else(|| anyhow!(json_error(&value, "重放失败")))
}

async fn post_ids(bytes: Vec<u8>) -> anyhow::Result<Vec<u64>> {
    let value = post_json(bytes).await?;
    value
        .get("ids")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_u64).collect())
        .ok_or_else(|| anyhow!(json_error(&value, "重放失败")))
}

async fn post_json(bytes: Vec<u8>) -> anyhow::Result<Value> {
    let request = nsh_core::prelude::HttpRequest::post("api/hub", bytes)
        .header("content-type", "application/json");
    let response = nsh_core::prelude::fetch(request)
        .await
        .map_err(|err| anyhow!(err))?;
    let value: Value = serde_json::from_str(&response.text()).unwrap_or(json!({}));
    if !response.ok() {
        return Err(anyhow!(json_error(&value, "请求失败")));
    }
    Ok(value)
}

fn json_error(value: &Value, fallback: &str) -> String {
    value
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

fn parse_paused(value: &Value) -> Option<(u64, BreakSide)> {
    let id = value.get("id").and_then(Value::as_u64)?;
    let side = match value.get("side").and_then(Value::as_str).unwrap_or("") {
        "response" => BreakSide::Response,
        _ => BreakSide::Request,
    };
    Some((id, side))
}

fn parse_conn_info(value: Option<&Value>) -> ConnInfo {
    let Some(obj) = value else {
        return ConnInfo::default();
    };
    ConnInfo {
        dns_ms: obj.get("dns_ms").and_then(Value::as_u64),
        tcp_ms: obj.get("tcp_ms").and_then(Value::as_u64),
        tls_ms: obj.get("tls_ms").and_then(Value::as_u64),
        reused: obj.get("reused").and_then(Value::as_bool).unwrap_or(false),
        server_addr: text_field(obj, "server_addr"),
        tls_version: text_field(obj, "tls_version"),
        client_cert: text_field(obj, "client_cert"),
        dialed_before_start: obj
            .get("dialed_before_start")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

fn parse_view(value: &Value) -> Option<CachedFlowView> {
    Some(CachedFlowView {
        id: value.get("id").and_then(Value::as_u64)?,
        rev: value.get("rev").and_then(Value::as_u64).unwrap_or(0),
        protocol: Protocol::from_label(value.get("protocol").and_then(Value::as_str).unwrap_or("")),
        method: text_field(value, "method"),
        host: text_field(value, "host"),
        path: text_field(value, "path"),
        url: text_field(value, "url"),
        client_ip: text_field(value, "client_ip"),
        conn_info: parse_conn_info(value.get("conn_info")),
        status: value
            .get("status")
            .and_then(Value::as_u64)
            .map(|code| code as u16),
        error: value
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string),
        started: millis_time(value.get("started_ms").and_then(Value::as_u64).unwrap_or(0)),
        response_at: value
            .get("response_ms")
            .and_then(Value::as_u64)
            .map(millis_time),
        finished_at: value
            .get("finished_ms")
            .and_then(Value::as_u64)
            .map(millis_time),
        request_bytes: value
            .get("request_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        response_bytes: value
            .get("response_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        pinned: value
            .get("pinned")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        rule_hit: text_field(value, "rule_hit"),
        comment: text_field(value, "comment"),
        relay_source: value
            .get("relay_source")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_string),
    })
}

fn body_field(value: &Value, text_key: &str, b64_key: &str) -> Vec<u8> {
    if let Some(b64) = value.get(b64_key).and_then(Value::as_str)
        && let Some(bytes) = crate::session::base64_decode(b64)
    {
        return bytes;
    }
    text_field(value, text_key).into_bytes()
}

fn parse_detail(value: &Value) -> Option<CachedFlowDetail> {
    Some(CachedFlowDetail {
        id: value.get("id").and_then(Value::as_u64)?,
        rev: value.get("rev").and_then(Value::as_u64).unwrap_or(0),
        request_headers: pairs(value.get("request_headers")),
        response_headers: pairs(value.get("response_headers")),
        request_body: body_field(value, "request_body", "request_body_b64"),
        response_body: body_field(value, "response_body", "response_body_b64"),
        request_truncated: value
            .get("request_truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        response_truncated: value
            .get("response_truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        request_encoding_error: value
            .get("request_encoding_error")
            .and_then(Value::as_str)
            .map(str::to_string),
        response_encoding_error: value
            .get("response_encoding_error")
            .and_then(Value::as_str)
            .map(str::to_string),
        request_expired: value
            .get("request_expired")
            .and_then(Value::as_u64)
            .map(|len| len as usize),
        response_expired: value
            .get("response_expired")
            .and_then(Value::as_u64)
            .map(|len| len as usize),
        messages: value
            .get("messages")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(parse_message).collect())
            .unwrap_or_default(),
        messages_truncated: value
            .get("messages_truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn parse_message(value: &Value) -> Option<Message> {
    let direction = match value.get("direction").and_then(Value::as_str).unwrap_or("") {
        "response" => Direction::Response,
        _ => Direction::Request,
    };
    Some(Message {
        direction,
        kind: text_field(value, "kind"),
        text: text_field(value, "text"),
    })
}

fn pairs(value: Option<&Value>) -> Vec<(String, String)> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let pair = item.as_array()?;
                    Some((
                        pair.first()
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        pair.get(1)
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn text_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn millis_time(millis: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(millis)
}
