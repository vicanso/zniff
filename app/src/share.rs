//! A snapshot HTML file, and one live share link.
//!
//! The link is a random id on the proxy port. It expires, and a new share
//! replaces the previous id. Requests for it are not recorded as flows.

use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(not(target_arch = "wasm32"))]
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};

use crate::session::{Direction, Flow};

/// Newest flows kept in one share document.
const FLOW_CAP: usize = 400;
/// Characters of one body included in the share.
const BODY_CHARS: usize = 16_000;
/// Maximum WebSocket frames or SSE events included per shared flow.
const MESSAGE_CAP: usize = 200;
/// Raw bytes encoded as base64 for 16/32-byte hex dump preview.
const HEX_PREVIEW_CAP: usize = 4_096;
/// Maximum raster image byte size embedded inline as a `data:` URL.
const IMAGE_INLINE_CAP: usize = 256 * 1024;

/// Placeholder written in place of unstructured or binary bodies when `RedactOptions::strict` is on.
pub const STRICT_REDACTED_BODY: &[u8] =
    b"[REDACTED: unstructured or binary body omitted in strict mode]";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RedactOptions {
    pub enabled: bool,
    pub strict: bool,
    pub extra_fields: Vec<String>,
}

impl RedactOptions {
    pub fn new(enabled: bool, extra_raw: &str) -> Self {
        let mut strict = false;
        let extra_fields = extra_raw
            .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .filter_map(|s| {
                if s.eq_ignore_ascii_case("@strict") {
                    strict = true;
                    None
                } else {
                    Some(s.to_ascii_lowercase())
                }
            })
            .collect();
        Self {
            enabled,
            strict,
            extra_fields,
        }
    }

    pub const fn with_strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn to_wire_fields(&self) -> String {
        let mut items = self.extra_fields.clone();
        if self.strict {
            items.push("@strict".to_string());
        }
        items.join(", ")
    }

    pub fn is_sensitive_header(&self, name: &str) -> bool {
        let lower = name.trim().to_ascii_lowercase();
        let compact: String = lower.chars().filter(|&c| c != '-' && c != '_').collect();
        const COMPACT_DEFAULT_HEADERS: &[&str] = &[
            "authorization",
            "proxyauthorization",
            "cookie",
            "setcookie",
            "xapikey",
            "xauthtoken",
            "xaccesstoken",
            "xcsrftoken",
            "xxsrftoken",
            "apikey",
        ];
        COMPACT_DEFAULT_HEADERS.contains(&compact.as_str())
            || self.extra_fields.iter().any(|extra| {
                let extra_compact: String =
                    extra.chars().filter(|&c| c != '-' && c != '_').collect();
                extra == &lower || extra_compact == compact
            })
    }

    pub fn is_sensitive_field(&self, name: &str) -> bool {
        let lower = name.trim().to_ascii_lowercase();
        let compact: String = lower.chars().filter(|&c| c != '-' && c != '_').collect();
        const COMPACT_DEFAULT_FIELDS: &[&str] = &[
            "password",
            "passwd",
            "pass",
            "pwd",
            "secret",
            "clientsecret",
            "token",
            "accesstoken",
            "refreshtoken",
            "idtoken",
            "authtoken",
            "apikey",
            "privatekey",
            "sessionid",
            "sid",
            "session",
            "jwt",
            "authorization",
            "proxyauthorization",
            "credential",
            "credentials",
            "cvv",
            "cvc",
            "ssn",
            "pin",
            "cookie",
            "cookies",
            "setcookie",
            "xapikey",
            "xauthtoken",
            "xaccesstoken",
            "xcsrftoken",
            "xxsrftoken",
            "csrftoken",
            "xsrftoken",
            "csrf",
            "xsrf",
        ];
        COMPACT_DEFAULT_FIELDS.contains(&compact.as_str())
            || self.is_sensitive_header(name)
            || self.extra_fields.iter().any(|extra| {
                let extra_compact: String =
                    extra.chars().filter(|&c| c != '-' && c != '_').collect();
                extra == &lower || extra_compact == compact
            })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RedactReport {
    pub redacted_headers: usize,
    pub redacted_query_params: usize,
    pub redacted_body_fields: usize,
    pub unsanitizable_req_body: bool,
    pub unsanitizable_res_body: bool,
    pub strict_masked_req_body: bool,
    pub strict_masked_res_body: bool,
}

impl RedactReport {
    pub fn total_redacted(&self) -> usize {
        self.redacted_headers + self.redacted_query_params + self.redacted_body_fields
    }

    pub fn has_unsanitizable_body(&self) -> bool {
        self.unsanitizable_req_body || self.unsanitizable_res_body
    }

    pub fn has_strict_masked_body(&self) -> bool {
        self.strict_masked_req_body || self.strict_masked_res_body
    }

    pub fn note(&self) -> Option<String> {
        let total = self.total_redacted();
        match (
            self.unsanitizable_req_body,
            self.unsanitizable_res_body,
            self.strict_masked_req_body,
            self.strict_masked_res_body,
        ) {
            (true, true, _, _) => Some("[REDACTED; req/res body format not sanitizable]".into()),
            (true, false, _, _) => Some("[REDACTED; req body format not sanitizable]".into()),
            (false, true, _, _) => Some("[REDACTED; res body format not sanitizable]".into()),
            (false, false, true, true) => {
                Some("[REDACTED; req/res body omitted in strict mode]".into())
            }
            (false, false, true, false) => {
                Some("[REDACTED; req body omitted in strict mode]".into())
            }
            (false, false, false, true) => {
                Some("[REDACTED; res body omitted in strict mode]".into())
            }
            (false, false, false, false) if total > 0 => Some("[REDACTED]".into()),
            (false, false, false, false) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveShare {
    pub id: String,
    pub expires: SystemTime,
    /// Filter text from the box at the moment the link was created.
    /// Later polls keep using it, including on flows that arrive after that.
    pub filter: String,
    pub redact: RedactOptions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharePath {
    Page,
    Data { since: Option<u64> },
    FlowBody(u64),
}

pub fn new_id() -> String {
    let mut bytes = [0_u8; 16];
    #[cfg(not(target_arch = "wasm32"))]
    let filled = SystemRandom::new().fill(&mut bytes).is_ok();
    #[cfg(target_arch = "wasm32")]
    let filled = false;
    if !filled {
        let nanos = crate::clock::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        bytes.copy_from_slice(&nanos.to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

#[derive(Deserialize, Default)]
pub struct InboundShare {
    #[serde(default)]
    pub flows: Vec<InboundFlow>,
}

#[derive(Deserialize, Default)]
pub struct InboundFlow {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub request_headers: Vec<(String, String)>,
    #[serde(default)]
    pub response_headers: Vec<(String, String)>,
    #[serde(default)]
    pub request_body: String,
    #[serde(default)]
    pub response_body: String,
    #[serde(default)]
    pub request_bytes: u64,
    #[serde(default)]
    pub response_bytes: u64,
    #[serde(default)]
    pub ttfb_ms: Option<u128>,
    #[serde(default)]
    pub total_ms: Option<u128>,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub rule_hit: String,
    #[serde(default)]
    pub messages: Vec<String>,
}

#[derive(Deserialize, Default)]
pub struct InboundBody {
    #[serde(default)]
    pub request_body: String,
    #[serde(default)]
    pub response_body: String,
    #[serde(default)]
    pub messages: Vec<String>,
}

pub fn parse_inbound_share(bytes: &[u8]) -> Option<InboundShare> {
    serde_json::from_slice(bytes).ok()
}

pub fn parse_inbound_body(bytes: &[u8]) -> Option<InboundBody> {
    serde_json::from_slice(bytes).ok()
}

pub fn valid_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 8 | 13 | 18 | 23) || byte.is_ascii_hexdigit())
}

/// Origin-form `/s/{id}`, `/s/{id}/flows.json[?since=N]`, or `/s/{id}/flows/{flow_id}.json`.
pub fn parse_share_path(path_and_query: &str) -> Option<(String, SharePath)> {
    let (path, query) = path_and_query
        .split_once('?')
        .map_or((path_and_query, ""), |(p, q)| (p, q));
    let rest = path.strip_prefix("/s/")?;
    if let Some(id) = rest.strip_suffix("/flows.json") {
        if !valid_id(id) {
            return None;
        }
        let since = query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == "since").then(|| v.parse::<u64>().ok()).flatten()
        });
        return Some((id.to_string(), SharePath::Data { since }));
    }
    if let Some((id, tail)) = rest.split_once("/flows/") {
        if !valid_id(id) {
            return None;
        }
        let flow_id = tail.strip_suffix(".json")?.parse::<u64>().ok()?;
        return Some((id.to_string(), SharePath::FlowBody(flow_id)));
    }
    valid_id(rest).then(|| (rest.to_string(), SharePath::Page))
}

pub fn grant_ok(share: &LiveShare, id: &str, now: SystemTime) -> bool {
    share.id == id && now < share.expires
}

#[derive(Serialize)]
struct ShareDoc {
    expires: u64,
    filter: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    redacted: bool,
    omitted: usize,
    rev: u64,
    incremental: bool,
    ids: Vec<u64>,
    flows: Vec<ShareFlow>,
}

fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

fn is_zero_usize(v: &usize) -> bool {
    *v == 0
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ShareConnInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reused: Option<bool>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    tls_version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    client_cert: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    server_addr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dns_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tcp_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tls_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    wait_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    download_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ShareMessage {
    dir: String,
    kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    label: String,
    text: String,
}

#[derive(Default, Serialize)]
struct ShareFlowBody {
    id: u64,
    request_body: String,
    request_body_pretty: String,
    response_body: String,
    response_body_pretty: String,
    messages: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    graphql_op: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    request_body_kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    response_body_kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    request_form_rows: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    request_hex_b64: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    response_hex_b64: String,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    request_raw_len: usize,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    response_raw_len: usize,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    request_truncated: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    response_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_expired: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response_expired: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_encoding_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response_encoding_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response_image_data_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response_image_dims: Option<(u32, u32)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    message_items: Vec<ShareMessage>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    messages_truncated: bool,
}

#[derive(Default, Serialize)]
struct ShareFlow {
    id: u64,
    method: String,
    status: Option<u16>,
    error: Option<String>,
    protocol: String,
    host: String,
    path: String,
    url: String,
    started: String,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    started_ms: u64,
    gap: String,
    elapsed: String,
    slow: bool,
    size: String,
    request_bytes: u64,
    response_bytes: u64,
    ttfb_ms: Option<u128>,
    total_ms: Option<u128>,
    pinned: bool,
    rule_hit: String,
    comment: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    client_ip: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    relay_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    graphql_op: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    conn_info: Option<ShareConnInfo>,
    query: Vec<(String, String)>,
    request_headers: Vec<(String, String)>,
    response_headers: Vec<(String, String)>,
    request_body_spilled: bool,
    response_body_spilled: bool,
    request_body: String,
    request_body_pretty: String,
    response_body: String,
    response_body_pretty: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    request_body_kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    response_body_kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    request_form_rows: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    request_hex_b64: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    response_hex_b64: String,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    request_raw_len: usize,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    response_raw_len: usize,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    request_truncated: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    response_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_expired: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response_expired: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_encoding_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response_encoding_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response_image_data_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response_image_dims: Option<(u32, u32)>,
    messages: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    message_items: Vec<ShareMessage>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    messages_truncated: bool,
}

#[allow(dead_code)]
pub fn export_html(flows: &[Flow], filter: &str, redact: &RedactOptions, zh: bool) -> String {
    let doc = document_with_redact(flows, crate::clock::now(), filter, redact);
    page(&doc, "", zh)
}

pub fn document_with_redact(
    flows: &[Flow],
    expires: SystemTime,
    filter: &str,
    redact: &RedactOptions,
) -> String {
    document_since_with_redact(flows, expires, filter, 0, None, redact)
}

pub fn document_since_with_redact(
    flows: &[Flow],
    expires: SystemTime,
    filter: &str,
    session_rev: u64,
    since: Option<u64>,
    redact: &RedactOptions,
) -> String {
    let omitted = flows.len().saturating_sub(FLOW_CAP);
    let window = &flows[omitted..];
    let ids: Vec<u64> = window.iter().map(|flow| flow.id).collect();
    let rev = session_rev.max(window.iter().map(|flow| flow.rev).max().unwrap_or(0));
    let mut previous = None;
    let mut out = Vec::new();
    for flow in window {
        let prev = previous;
        previous = Some(flow.started);
        if since.is_none_or(|s| flow.rev > s) {
            if redact.enabled {
                let (sanitized, _) = redact_flow(flow, redact);
                out.push(share_flow(&sanitized, prev, Some(redact)));
            } else {
                out.push(share_flow(flow, prev, None));
            }
        }
    }
    let doc = ShareDoc {
        expires: expires
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0),
        filter: filter.to_string(),
        redacted: redact.enabled,
        omitted,
        rev,
        incremental: since.is_some(),
        ids,
        flows: out,
    };
    serde_json::to_string(&doc).unwrap_or_else(|_| "{}".into())
}

pub fn flow_body_document_with_redact(flow: &Flow, redact: &RedactOptions) -> String {
    let owned;
    let (target, redact_ref) = if redact.enabled {
        let (sanitized, _) = redact_flow(flow, redact);
        owned = sanitized;
        (&owned, Some(redact))
    } else {
        (flow, None)
    };
    let req_ct = crate::present::content_type(&target.request_headers);
    let res_ct = crate::present::content_type(&target.response_headers);
    let (request_body, request_body_pretty) = share_body_pair(
        &target.request_body,
        target.request_truncated,
        target.request_expired,
        req_ct,
        redact_ref,
    );
    let (response_body, response_body_pretty) = share_body_pair(
        &target.response_body,
        target.response_truncated,
        target.response_expired,
        res_ct,
        redact_ref,
    );
    let (messages, message_items, messages_truncated) = extract_messages_bundle(target, res_ct);
    let request_body_kind = classify_body_kind(req_ct, &target.request_body);
    let response_body_kind = classify_body_kind(res_ct, &target.response_body);
    let request_form_rows = extract_form_rows(req_ct, &target.request_body);
    let request_hex_b64 = extract_hex_b64(&target.request_body);
    let response_hex_b64 = extract_hex_b64(&target.response_body);
    let (response_image_data_url, response_image_dims) =
        extract_safe_image_preview(res_ct, &target.response_body);
    let graphql_op = if target.request_body == STRICT_REDACTED_BODY {
        crate::present::graphql_operation_label(&target.path, &[])
    } else {
        crate::present::graphql_operation_label(&target.path, &target.request_body)
    };
    let doc = ShareFlowBody {
        id: target.id,
        request_body,
        request_body_pretty,
        response_body,
        response_body_pretty,
        messages,
        graphql_op,
        request_body_kind,
        response_body_kind,
        request_form_rows,
        request_hex_b64,
        response_hex_b64,
        request_raw_len: if target.request_body == STRICT_REDACTED_BODY {
            0
        } else {
            target.request_body.len()
        },
        response_raw_len: if target.response_body == STRICT_REDACTED_BODY {
            0
        } else {
            target.response_body.len()
        },
        request_truncated: target.request_truncated,
        response_truncated: target.response_truncated,
        request_expired: target.request_expired,
        response_expired: target.response_expired,
        request_encoding_error: target.request_encoding_error.clone(),
        response_encoding_error: target.response_encoding_error.clone(),
        response_image_data_url,
        response_image_dims,
        message_items,
        messages_truncated,
    };
    serde_json::to_string(&doc).unwrap_or_else(|_| "{}".into())
}

pub fn redact_flow(flow: &Flow, opts: &RedactOptions) -> (Flow, RedactReport) {
    let mut out = flow.clone();
    if !opts.enabled {
        return (out, RedactReport::default());
    }
    let mut report = RedactReport::default();

    for (name, value) in &mut out.request_headers {
        if let Some(new_val) = redact_header_value(name, value, opts)
            && *value != new_val
        {
            *value = new_val;
            report.redacted_headers += 1;
        }
    }
    for (name, value) in &mut out.response_headers {
        if let Some(new_val) = redact_header_value(name, value, opts)
            && *value != new_val
        {
            *value = new_val;
            report.redacted_headers += 1;
        }
    }

    let (new_url, url_q_count) = redact_url_query(&out.url, opts);
    out.url = new_url;
    let (new_path, path_q_count) = redact_url_query(&out.path, opts);
    out.path = new_path;
    report.redacted_query_params += url_q_count.max(path_q_count);

    let req_ct = crate::present::content_type(&out.request_headers)
        .unwrap_or("")
        .to_string();
    if !out.request_body.is_empty() {
        let (new_body, count, unsanitizable, strict_masked) =
            redact_body_bytes(&out.request_body, &req_ct, opts);
        out.request_body = new_body;
        report.redacted_body_fields += count;
        report.unsanitizable_req_body = unsanitizable;
        report.strict_masked_req_body = strict_masked;
    }

    let res_ct = crate::present::content_type(&out.response_headers)
        .unwrap_or("")
        .to_string();
    if !out.response_body.is_empty() {
        let (new_body, count, unsanitizable, strict_masked) =
            redact_body_bytes(&out.response_body, &res_ct, opts);
        out.response_body = new_body;
        report.redacted_body_fields += count;
        report.unsanitizable_res_body = unsanitizable;
        report.strict_masked_res_body = strict_masked;
    }

    for msg in &mut out.messages {
        let is_binary_frame = msg.kind == "二进制" || msg.kind.eq_ignore_ascii_case("binary");
        if opts.strict && is_binary_frame {
            let strict_str = String::from_utf8_lossy(STRICT_REDACTED_BODY).into_owned();
            if msg.text != strict_str {
                msg.text = strict_str;
                report.redacted_body_fields += 1;
            }
        } else if let Ok(mut json_val) = serde_json::from_str::<serde_json::Value>(&msg.text) {
            let count = redact_json_value(&mut json_val, opts);
            if count > 0
                && let Ok(serialized) = serde_json::to_string(&json_val)
            {
                msg.text = serialized;
                report.redacted_body_fields += count;
            }
        } else {
            let count = redact_string_value(&mut msg.text, opts, 0);
            report.redacted_body_fields += count;
        }
    }

    if let Some(note) = report.note() {
        if out.comment.is_empty() {
            out.comment = note;
        } else if !out.comment.contains("[REDACTED") {
            out.comment = format!("{} {note}", out.comment);
        }
    }

    (out, report)
}

fn redact_header_value(name: &str, value: &str, opts: &RedactOptions) -> Option<String> {
    if !opts.is_sensitive_header(name) {
        return None;
    }
    let lower = name.trim().to_ascii_lowercase();
    if lower == "authorization" || lower == "proxy-authorization" {
        let trimmed = value.trim();
        if let Some((scheme, rest)) = trimmed.split_once(char::is_whitespace)
            && !rest.trim().is_empty()
        {
            return Some(format!("{scheme} [REDACTED]"));
        }
        return Some("[REDACTED]".into());
    }
    if lower == "cookie" {
        let parts: Vec<String> = value
            .split(';')
            .map(|seg| {
                let s = seg.trim();
                if let Some((k, _)) = s.split_once('=') {
                    format!("{}=[REDACTED]", k.trim())
                } else if s.is_empty() {
                    String::new()
                } else {
                    "[REDACTED]".into()
                }
            })
            .filter(|s| !s.is_empty())
            .collect();
        return Some(parts.join("; "));
    }
    if lower == "set-cookie" {
        let mut iter = value.split(';');
        let first = iter.next().unwrap_or("").trim();
        let redacted_first = if let Some((k, _)) = first.split_once('=') {
            format!("{}=[REDACTED]", k.trim())
        } else {
            "[REDACTED]".into()
        };
        let mut out = vec![redacted_first];
        for attr in iter {
            let attr_trimmed = attr.trim();
            if !attr_trimmed.is_empty() {
                out.push(attr_trimmed.to_string());
            }
        }
        return Some(out.join("; "));
    }
    Some("[REDACTED]".into())
}

fn redact_url_query(url_or_path: &str, opts: &RedactOptions) -> (String, usize) {
    let Some((before_q, after_q)) = url_or_path.split_once('?') else {
        return (url_or_path.to_string(), 0);
    };
    let (query, frag) = match after_q.split_once('#') {
        Some((q, f)) => (q, Some(f)),
        None => (after_q, None),
    };
    let (new_query, count) = redact_urlencoded_pairs(query, opts);
    if count == 0 {
        return (url_or_path.to_string(), 0);
    }
    let mut out = format!("{before_q}?{new_query}");
    if let Some(f) = frag {
        out.push('#');
        out.push_str(f);
    }
    (out, count)
}

fn percent_encode_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() * 3);
    for b in raw.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

fn redact_urlencoded_pairs(query: &str, opts: &RedactOptions) -> (String, usize) {
    let mut count = 0;
    let mut parts = Vec::new();
    for seg in query.split('&') {
        if let Some((k, v)) = seg.split_once('=') {
            let decoded_k = percent_decode_key(k);
            if opts.is_sensitive_field(&decoded_k) && v != "[REDACTED]" && v != "%5BREDACTED%5D" {
                parts.push(format!("{k}=[REDACTED]"));
                count += 1;
            } else if v.contains('?') {
                let (new_v, v_count) = redact_url_query(v, opts);
                if v_count > 0 {
                    parts.push(format!("{k}={new_v}"));
                    count += v_count;
                } else {
                    parts.push(seg.to_string());
                }
            } else if v.contains("%3F")
                || v.contains("%3f")
                || ((v.contains("%3D") || v.contains("%3d"))
                    && (v.contains("%26")
                        || percent_decode_key(v)
                            .split_once('=')
                            .is_some_and(|(sub_k, _)| {
                                opts.is_sensitive_field(&percent_decode_key(sub_k))
                            })))
            {
                let decoded_v = percent_decode_key(v);
                let (new_decoded_v, v_count) = if decoded_v.contains('?') {
                    redact_url_query(&decoded_v, opts)
                } else {
                    redact_urlencoded_pairs(&decoded_v, opts)
                };
                if v_count > 0 {
                    let re_encoded = percent_encode_component(&new_decoded_v);
                    parts.push(format!("{k}={re_encoded}"));
                    count += v_count;
                } else {
                    parts.push(seg.to_string());
                }
            } else {
                parts.push(seg.to_string());
            }
        } else {
            parts.push(seg.to_string());
        }
    }
    (parts.join("&"), count)
}

fn percent_decode_key(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3])
            && let Ok(val) = u8::from_str_radix(hex, 16)
        {
            out.push(val);
            i += 3;
            continue;
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn redact_sse_payload(payload: &str, opts: &RedactOptions) -> (String, usize) {
    let trimmed = payload.trim();
    if !trimmed.is_empty()
        && ((trimmed.starts_with('{') && trimmed.ends_with('}'))
            || (trimmed.starts_with('[') && trimmed.ends_with(']')))
        && let Ok(mut val) = serde_json::from_str::<serde_json::Value>(trimmed)
    {
        let count = redact_json_value(&mut val, opts);
        if count > 0
            && let Ok(serialized) = serde_json::to_string(&val)
        {
            return (serialized, count);
        }
        return (payload.to_string(), 0);
    }
    let mut s = payload.to_string();
    let count = redact_string_value(&mut s, opts, 0);
    (s, count)
}

fn redact_sse_event_block(lines: &[(&str, &str)], opts: &RedactOptions) -> (String, usize) {
    // Check if multiple `data:` lines in one SSE event combine into a single JSON document.
    let mut data_parts: Vec<&str> = Vec::new();
    for &(line, _) in lines {
        if let Some(rest) = line.strip_prefix("data:") {
            data_parts.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if data_parts.len() > 1 {
        let joined = data_parts.join("\n");
        let trimmed = joined.trim();
        if ((trimmed.starts_with('{') && trimmed.ends_with('}'))
            || (trimmed.starts_with('[') && trimmed.ends_with(']')))
            && let Ok(mut val) = serde_json::from_str::<serde_json::Value>(trimmed)
        {
            let count = redact_json_value(&mut val, opts);
            if count > 0
                && let Ok(serialized) = serde_json::to_string(&val)
            {
                let mut out = String::new();
                let mut emitted_data = false;
                for &(line, ending) in lines {
                    if line.starts_with("data:") {
                        if !emitted_data {
                            out.push_str("data: ");
                            out.push_str(&serialized);
                            out.push_str(ending);
                            emitted_data = true;
                        }
                    } else {
                        out.push_str(line);
                        out.push_str(ending);
                    }
                }
                return (out, count);
            }
            let mut unchanged = String::new();
            for &(line, ending) in lines {
                unchanged.push_str(line);
                unchanged.push_str(ending);
            }
            return (unchanged, 0);
        }
    }

    let mut out = String::new();
    let mut total = 0;
    for &(line, ending) in lines {
        if let Some(rest) = line.strip_prefix("data:") {
            let (prefix, payload) = match rest.strip_prefix(' ') {
                Some(p) => ("data: ", p),
                None => ("data:", rest),
            };
            let (new_payload, count) = redact_sse_payload(payload, opts);
            if count > 0 {
                out.push_str(prefix);
                out.push_str(&new_payload);
                out.push_str(ending);
                total += count;
            } else {
                out.push_str(line);
                out.push_str(ending);
            }
        } else {
            out.push_str(line);
            out.push_str(ending);
        }
    }
    (out, total)
}

fn redact_event_stream(text: &str, opts: &RedactOptions) -> (String, usize) {
    let mut out = String::with_capacity(text.len());
    let mut total_count = 0;
    let mut event_lines: Vec<(&str, &str)> = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let (line, ending, next) = match rest.find('\n') {
            Some(nl) => {
                if nl > 0 && rest.as_bytes()[nl - 1] == b'\r' {
                    (&rest[..nl - 1], "\r\n", &rest[nl + 1..])
                } else {
                    (&rest[..nl], "\n", &rest[nl + 1..])
                }
            }
            None => (rest, "", ""),
        };
        rest = next;
        if line.is_empty() {
            if !event_lines.is_empty() {
                let (block_out, c) = redact_sse_event_block(&event_lines, opts);
                out.push_str(&block_out);
                total_count += c;
                event_lines.clear();
            }
            out.push_str(ending);
        } else {
            event_lines.push((line, ending));
        }
    }
    if !event_lines.is_empty() {
        let (block_out, c) = redact_sse_event_block(&event_lines, opts);
        out.push_str(&block_out);
        total_count += c;
    }
    (out, total_count)
}

fn extract_multipart_boundary_param(content_type: &str) -> Option<String> {
    let mut parts = content_type.split(';');
    let mime = parts.next()?.trim();
    if !mime.eq_ignore_ascii_case("multipart/form-data") {
        return None;
    }
    for param in parts {
        let param = param.trim();
        if let Some((k, v)) = param.split_once('=')
            && k.trim().eq_ignore_ascii_case("boundary")
        {
            let val = v.trim().trim_matches('"').trim_matches('\'');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

fn extract_disposition_param(disposition: &str, target: &str) -> Option<String> {
    for seg in disposition.split(';').skip(1) {
        let seg = seg.trim();
        let Some((k, v)) = seg.split_once('=') else {
            continue;
        };
        let key = k.trim();
        if key.eq_ignore_ascii_case(target) {
            let val = v.trim().trim_matches('"').trim_matches('\'');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        } else if key.eq_ignore_ascii_case(&format!("{target}*")) {
            let raw = v.trim().trim_matches('"');
            let encoded = raw.split_once("''").map(|(_, rest)| rest).unwrap_or(raw);
            let decoded = percent_decode_key(encoded);
            if !decoded.is_empty() {
                return Some(decoded);
            }
        }
    }
    None
}

fn filename_matches_sensitive(filename: &str, opts: &RedactOptions) -> bool {
    let base = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    opts.is_sensitive_field(base)
        || base
            .rsplit_once('.')
            .is_some_and(|(stem, _)| !stem.is_empty() && opts.is_sensitive_field(stem))
}

fn find_boundary_positions(body: &[u8], delim: &[u8]) -> Vec<usize> {
    if delim.is_empty() || body.len() < delim.len() {
        return Vec::new();
    }
    let mut positions = Vec::new();
    let mut i = 0;
    while i + delim.len() <= body.len() {
        if &body[i..i + delim.len()] == delim && (i == 0 || body[i - 1] == b'\n') {
            positions.push(i);
            i += delim.len();
        } else {
            i += 1;
        }
    }
    positions
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn redact_multipart_body(
    body: &[u8],
    boundary: &str,
    opts: &RedactOptions,
) -> Option<(Vec<u8>, usize, bool, bool)> {
    let delim = format!("--{boundary}");
    let delim_bytes = delim.as_bytes();
    let positions = find_boundary_positions(body, delim_bytes);
    if positions.len() < 2 {
        return None;
    }

    let mut out = Vec::with_capacity(body.len());
    out.extend_from_slice(&body[..positions[0]]);

    let mut total_count = 0usize;
    let mut any_unsanitizable = false;
    let mut any_strict_masked = false;
    let mut changed = false;
    let mut saw_closing = false;

    for (idx, &pos) in positions.iter().enumerate() {
        let after_delim = pos + delim_bytes.len();
        if body[after_delim..].starts_with(b"--") {
            out.extend_from_slice(&body[pos..]);
            saw_closing = true;
            break;
        }
        let after_nl_len = if body[after_delim..].starts_with(b"\r\n") {
            2
        } else if body[after_delim..].starts_with(b"\n") {
            1
        } else {
            return None;
        };
        let part_start = after_delim + after_nl_len;
        let &next_pos = positions.get(idx + 1)?;
        let (part_end, pre_next_nl): (usize, &[u8]) =
            if next_pos >= 2 && &body[next_pos - 2..next_pos] == b"\r\n" {
                (next_pos - 2, b"\r\n")
            } else if next_pos >= 1 && body[next_pos - 1] == b'\n' {
                (next_pos - 1, b"\n")
            } else {
                return None;
            };
        if part_end < part_start {
            return None;
        }
        let part = &body[part_start..part_end];
        let (sep_pos, sep_len) = if let Some(p) = find_bytes(part, b"\r\n\r\n") {
            (p, 4)
        } else {
            let p = find_bytes(part, b"\n\n")?;
            (p, 2)
        };
        let raw_headers = &part[..sep_pos];
        let sep = &part[sep_pos..sep_pos + sep_len];
        let payload = &part[sep_pos + sep_len..];
        let headers_str = std::str::from_utf8(raw_headers).ok()?;

        let mut disp_name: Option<String> = None;
        let mut disp_filename: Option<String> = None;
        let mut part_ct: Option<String> = None;
        for line in headers_str.lines() {
            let line = line.trim_end_matches('\r');
            let Some((h_name, h_val)) = line.split_once(':') else {
                continue;
            };
            let h_name = h_name.trim();
            let h_val = h_val.trim();
            if h_name.eq_ignore_ascii_case("content-disposition") {
                disp_name = extract_disposition_param(h_val, "name");
                disp_filename = extract_disposition_param(h_val, "filename");
            } else if h_name.eq_ignore_ascii_case("content-type") {
                part_ct = Some(h_val.to_string());
            }
        }

        let field_sensitive = disp_name
            .as_deref()
            .is_some_and(|n| opts.is_sensitive_field(n))
            || disp_filename
                .as_deref()
                .is_some_and(|f| filename_matches_sensitive(f, opts));

        let new_payload: Vec<u8> = if field_sensitive {
            if payload == b"[REDACTED]" || payload == STRICT_REDACTED_BODY {
                payload.to_vec()
            } else {
                total_count += 1;
                changed = true;
                b"[REDACTED]".to_vec()
            }
        } else if payload.is_empty() || payload == STRICT_REDACTED_BODY {
            payload.to_vec()
        } else {
            let ct_str = part_ct.as_deref().unwrap_or("");
            let ct_lower = ct_str.to_ascii_lowercase();
            let is_plain_form_value = disp_filename.is_none()
                && (ct_lower.is_empty() || ct_lower.starts_with("text/plain"));
            if is_plain_form_value {
                match std::str::from_utf8(payload) {
                    Ok(text) => {
                        let trimmed = text.trim();
                        if ((trimmed.starts_with('{') && trimmed.ends_with('}'))
                            || (trimmed.starts_with('[') && trimmed.ends_with(']')))
                            && serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
                        {
                            let (sub_out, sub_cnt, _, _) =
                                redact_body_bytes(payload, "application/json", opts);
                            if sub_cnt > 0 {
                                total_count += sub_cnt;
                                changed = true;
                            }
                            sub_out
                        } else {
                            let mut val_str = text.to_string();
                            let sub_cnt = redact_string_value(&mut val_str, opts, 0);
                            if sub_cnt > 0 {
                                total_count += sub_cnt;
                                changed = true;
                                val_str.into_bytes()
                            } else {
                                payload.to_vec()
                            }
                        }
                    }
                    Err(_) => {
                        if opts.strict {
                            total_count += 1;
                            any_strict_masked = true;
                            changed = true;
                            STRICT_REDACTED_BODY.to_vec()
                        } else {
                            any_unsanitizable = true;
                            payload.to_vec()
                        }
                    }
                }
            } else {
                let (sub_out, sub_cnt, sub_unsan, sub_strict) =
                    redact_body_bytes(payload, ct_str, opts);
                if sub_cnt > 0 || sub_strict {
                    changed = true;
                }
                total_count += sub_cnt;
                any_unsanitizable |= sub_unsan;
                any_strict_masked |= sub_strict;
                sub_out
            }
        };

        out.extend_from_slice(&body[pos..part_start]);
        out.extend_from_slice(raw_headers);
        out.extend_from_slice(sep);
        out.extend_from_slice(&new_payload);
        out.extend_from_slice(pre_next_nl);
    }

    if !saw_closing {
        return None;
    }
    if !changed {
        return Some((body.to_vec(), 0, any_unsanitizable, any_strict_masked));
    }
    Some((out, total_count, any_unsanitizable, any_strict_masked))
}

fn finish_unsanitizable_body(body: &[u8], opts: &RedactOptions) -> (Vec<u8>, usize, bool, bool) {
    if opts.strict {
        (STRICT_REDACTED_BODY.to_vec(), 1, false, true)
    } else {
        (body.to_vec(), 0, true, false)
    }
}

fn redact_body_bytes(
    body: &[u8],
    content_type: &str,
    opts: &RedactOptions,
) -> (Vec<u8>, usize, bool, bool) {
    if body.is_empty() || body == STRICT_REDACTED_BODY {
        return (body.to_vec(), 0, false, false);
    }
    let ct_lower = content_type.to_ascii_lowercase();
    if ct_lower.contains("multipart/form-data") {
        if let Some(boundary) = extract_multipart_boundary_param(content_type)
            && let Some(res) = redact_multipart_body(body, &boundary, opts)
        {
            return res;
        }
        return finish_unsanitizable_body(body, opts);
    }
    let Ok(text) = std::str::from_utf8(body) else {
        return finish_unsanitizable_body(body, opts);
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return (body.to_vec(), 0, false, false);
    }
    if ct_lower.contains("event-stream") {
        let (redacted_sse, count) = redact_event_stream(text, opts);
        if count > 0 {
            return (redacted_sse.into_bytes(), count, false, false);
        }
        return (body.to_vec(), 0, false, false);
    }
    let looks_json = ct_lower.contains("json")
        || (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'));
    if looks_json {
        if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let count = redact_json_value(&mut val, opts);
            if count > 0
                && let Ok(out) = serde_json::to_vec(&val)
            {
                return (out, count, false, false);
            }
            return (body.to_vec(), 0, false, false);
        }
        if ct_lower.contains("json") {
            return finish_unsanitizable_body(body, opts);
        }
    }
    if ct_lower.contains("application/x-www-form-urlencoded") {
        let (redacted_form, count) = redact_urlencoded_pairs(text, opts);
        if count > 0 {
            return (redacted_form.into_bytes(), count, false, false);
        }
        return (body.to_vec(), 0, false, false);
    }
    finish_unsanitizable_body(body, opts)
}

fn redact_json_value(val: &mut serde_json::Value, opts: &RedactOptions) -> usize {
    redact_json_value_depth(val, opts, 0)
}

fn redact_json_value_depth(
    val: &mut serde_json::Value,
    opts: &RedactOptions,
    depth: usize,
) -> usize {
    if depth > 16 {
        return 0;
    }
    match val {
        serde_json::Value::Object(map) => {
            let mut count = 0;
            for (k, v) in map.iter_mut() {
                if opts.is_sensitive_field(k) && !v.is_null() {
                    if v.as_str() != Some("[REDACTED]") {
                        *v = serde_json::Value::String("[REDACTED]".into());
                        count += 1;
                    }
                } else {
                    count += redact_json_value_depth(v, opts, depth);
                }
            }
            count
        }
        serde_json::Value::Array(arr) => arr
            .iter_mut()
            .map(|item| redact_json_value_depth(item, opts, depth))
            .sum(),
        serde_json::Value::String(s) => redact_string_value(s, opts, depth),
        _ => 0,
    }
}

fn redact_string_value(s: &mut String, opts: &RedactOptions, depth: usize) -> usize {
    if s == "[REDACTED]" {
        return 0;
    }
    let trimmed = s.trim();
    // 1. Unpack stringified JSON if it starts with { or [
    if ((trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']')))
        && let Ok(mut nested) = serde_json::from_str::<serde_json::Value>(trimmed)
    {
        let count = redact_json_value_depth(&mut nested, opts, depth + 1);
        if count > 0
            && let Ok(serialized) = serde_json::to_string(&nested)
        {
            *s = serialized;
            return count;
        }
        return 0;
    }

    // 2. Redact URL query parameters if it contains '?'
    if s.contains('?') {
        let (new_s, count) = redact_url_query(s, opts);
        if count > 0 {
            *s = new_s;
        }
        return count;
    }

    // 2b. Redact percent-encoded URL query parameters if it contains %3F or %3f
    if s.contains("%3F") || s.contains("%3f") {
        let decoded = percent_decode_key(s);
        if decoded.contains('?') {
            let (new_decoded, count) = redact_url_query(&decoded, opts);
            if count > 0 {
                *s = percent_encode_component(&new_decoded);
            }
            return count;
        }
    }

    // 3. Redact standalone urlencoded query string (e.g. "token=xyz&secret=123" or single pair "password=123")
    if s.contains('=')
        && (s.contains('&')
            || s.split_once('=')
                .is_some_and(|(k, _)| opts.is_sensitive_field(&percent_decode_key(k))))
    {
        let (new_s, count) = redact_urlencoded_pairs(s, opts);
        if count > 0 {
            *s = new_s;
            return count;
        }
    }

    0
}

pub fn format_redact_preview(
    flow: &Flow,
    opts: &RedactOptions,
    zh: bool,
) -> (String, String, RedactReport) {
    let mut enabled_opts = opts.clone();
    enabled_opts.enabled = true;
    let (sanitized, report) = redact_flow(flow, &enabled_opts);
    (
        format_flow_snapshot(flow, zh),
        format_flow_snapshot(&sanitized, zh),
        report,
    )
}

fn format_flow_snapshot(flow: &Flow, zh: bool) -> String {
    let mut lines = Vec::new();
    lines.push(format!("{} {}", flow.method, flow.url));
    if !flow.comment.is_empty() {
        let label = if zh { "备注" } else { "Note" };
        lines.push(format!("# {label}: {}", flow.comment));
    }
    lines.push(String::new());
    lines.push(if zh {
        "--- 请求头 ---".into()
    } else {
        "--- Request Headers ---".into()
    });
    for (k, v) in &flow.request_headers {
        lines.push(format!("{k}: {v}"));
    }
    if !flow.request_body.is_empty() {
        lines.push(String::new());
        lines.push(if zh {
            "--- 请求 Body ---".into()
        } else {
            "--- Request Body ---".into()
        });
        let req_ct = crate::present::content_type(&flow.request_headers);
        if let Ok(text) = std::str::from_utf8(&flow.request_body) {
            lines.push(clip_chars(
                &crate::present::pretty_body(req_ct, text),
                4_000,
            ));
        } else {
            lines.push(format!("[binary {} bytes]", flow.request_body.len()));
        }
    }
    if !flow.response_headers.is_empty() || flow.status.is_some() {
        lines.push(String::new());
        lines.push(if zh {
            format!("--- 响应头 ({}) ---", flow.status.unwrap_or(0))
        } else {
            format!("--- Response Headers ({}) ---", flow.status.unwrap_or(0))
        });
        for (k, v) in &flow.response_headers {
            lines.push(format!("{k}: {v}"));
        }
    }
    if !flow.response_body.is_empty() {
        lines.push(String::new());
        lines.push(if zh {
            "--- 响应 Body ---".into()
        } else {
            "--- Response Body ---".into()
        });
        let res_ct = crate::present::content_type(&flow.response_headers);
        if let Ok(text) = std::str::from_utf8(&flow.response_body) {
            lines.push(clip_chars(
                &crate::present::pretty_body(res_ct, text),
                4_000,
            ));
        } else {
            lines.push(format!("[binary {} bytes]", flow.response_body.len()));
        }
    }
    lines.join("\n")
}

fn format_bytes_short(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if (bytes as f64) < MB {
        format!("{:.1} KB", bytes as f64 / KB)
    } else {
        format!("{:.2} MB", bytes as f64 / MB)
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    if bytes.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn safe_raster_image_mime(content_type: Option<&str>) -> Option<&'static str> {
    let base = content_type?.split(';').next()?.trim().to_ascii_lowercase();
    match base.as_str() {
        "image/png" => Some("image/png"),
        "image/jpeg" | "image/jpg" => Some("image/jpeg"),
        "image/gif" => Some("image/gif"),
        "image/webp" => Some("image/webp"),
        "image/bmp" | "image/x-ms-bmp" => Some("image/bmp"),
        "image/x-icon" | "image/vnd.microsoft.icon" | "image/ico" | "image/icon" => {
            Some("image/x-icon")
        }
        "image/avif" => Some("image/avif"),
        _ => None,
    }
}

fn extract_safe_image_preview(
    content_type: Option<&str>,
    bytes: &[u8],
) -> (Option<String>, Option<(u32, u32)>) {
    if bytes.is_empty() || bytes == STRICT_REDACTED_BODY || bytes.len() > IMAGE_INLINE_CAP {
        return (None, None);
    }
    let Some(mime) = safe_raster_image_mime(content_type) else {
        return (None, None);
    };
    let dims = crate::present::image_pixel_size(bytes);
    let url = format!("data:{mime};base64,{}", base64_encode(bytes));
    (Some(url), dims)
}

fn classify_body_kind(content_type: Option<&str>, bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    if bytes == STRICT_REDACTED_BODY {
        return "text".into();
    }
    let ct = content_type.unwrap_or("").to_ascii_lowercase();
    if safe_raster_image_mime(content_type).is_some()
        || (ct.starts_with("image/") && !ct.contains("svg"))
    {
        return "image".into();
    }
    if crate::present::is_protobuf(content_type) {
        return "protobuf".into();
    }
    if crate::present::is_msgpack(content_type) {
        return "msgpack".into();
    }
    if ct.contains("multipart/form-data") {
        return "multipart".into();
    }
    if ct.contains("x-www-form-urlencoded") {
        return "form".into();
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        let trimmed = text.trim();
        if (ct.contains("json")
            || ct.contains("graphql")
            || trimmed.starts_with('{')
            || trimmed.starts_with('['))
            && crate::present::pretty_graphql_json(text).is_some()
        {
            return "graphql".into();
        }
        if ct.contains("graphql") {
            return "graphql".into();
        }
        if (ct.contains("json")
            || (trimmed.starts_with('{') && trimmed.ends_with('}'))
            || (trimmed.starts_with('[') && trimmed.ends_with(']')))
            && serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
        {
            return "json".into();
        }
        if ct.contains("html") {
            return "html".into();
        }
        if ct.contains("markdown") || ct.contains("text/md") || ct.contains("text/x-markdown") {
            return "markdown".into();
        }
        if ct.contains("xml") || ct.contains("svg") {
            return "xml".into();
        }
        "text".into()
    } else {
        "binary".into()
    }
}

fn extract_form_rows(content_type: Option<&str>, bytes: &[u8]) -> Vec<(String, String)> {
    if bytes.is_empty() || bytes == STRICT_REDACTED_BODY {
        return Vec::new();
    }
    if let Some(rows) = crate::present::multipart_pairs(content_type, bytes) {
        return rows;
    }
    let ct = content_type.unwrap_or("").to_ascii_lowercase();
    if ct.contains("x-www-form-urlencoded")
        && let Ok(text) = std::str::from_utf8(bytes)
    {
        return crate::present::form_pairs(text);
    }
    Vec::new()
}

fn extract_hex_b64(bytes: &[u8]) -> String {
    if bytes.is_empty() || bytes == STRICT_REDACTED_BODY {
        return String::new();
    }
    base64_encode(&bytes[..bytes.len().min(HEX_PREVIEW_CAP)])
}

fn normalize_ws_kind(raw_kind: &str) -> (&'static str, String) {
    let lower = raw_kind.trim().to_lowercase();
    match lower.as_str() {
        "文本" | "text" => ("text", String::new()),
        "续帧" | "continuation" => ("text", raw_kind.to_string()),
        "二进制" | "binary" => ("binary", String::new()),
        "关闭" | "close" => ("control", "Close".into()),
        "ping" => ("control", "Ping".into()),
        "pong" => ("control", "Pong".into()),
        _ => ("text", raw_kind.to_string()),
    }
}

fn extract_messages_bundle(
    flow: &Flow,
    res_ct: Option<&str>,
) -> (Vec<String>, Vec<ShareMessage>, bool) {
    if !flow.messages.is_empty() {
        let truncated = flow.messages_truncated || flow.messages.len() > MESSAGE_CAP;
        let mut legacy = Vec::with_capacity(flow.messages.len().min(MESSAGE_CAP));
        let mut items = Vec::with_capacity(flow.messages.len().min(MESSAGE_CAP));
        for message in flow.messages.iter().take(MESSAGE_CAP) {
            let (side, dir) = match message.direction {
                Direction::Request => ("→", "up"),
                Direction::Response => ("←", "down"),
            };
            let text = clip_chars(&message.text, 4_000);
            legacy.push(format!("{side} {} {text}", message.kind));
            let (kind, label) = normalize_ws_kind(&message.kind);
            items.push(ShareMessage {
                dir: dir.into(),
                kind: kind.into(),
                label,
                text,
            });
        }
        (legacy, items, truncated)
    } else if res_ct.is_some_and(|ct| ct.to_ascii_lowercase().contains("event-stream"))
        && flow.response_body != STRICT_REDACTED_BODY
        && let Ok(text) = std::str::from_utf8(&flow.response_body)
    {
        let all_events = crate::present::sse_events(text);
        let truncated = flow.response_truncated || all_events.len() > MESSAGE_CAP;
        let mut legacy = Vec::with_capacity(all_events.len().min(MESSAGE_CAP));
        let mut items = Vec::with_capacity(all_events.len().min(MESSAGE_CAP));
        for ev in all_events.into_iter().take(MESSAGE_CAP) {
            let data = clip_chars(&ev.data, 4_000);
            legacy.push(format!("← [{}] {data}", ev.event));
            items.push(ShareMessage {
                dir: "down".into(),
                kind: "sse".into(),
                label: ev.event,
                text: data,
            });
        }
        (legacy, items, truncated)
    } else {
        (Vec::new(), Vec::new(), flow.messages_truncated)
    }
}

fn share_conn_info(flow: &Flow) -> Option<ShareConnInfo> {
    let tb = crate::present::flow_timing_breakdown(flow);
    if !tb.has_content() {
        return None;
    }
    let reused = if flow.conn_info.is_empty() {
        None
    } else {
        Some(tb.reused)
    };
    Some(ShareConnInfo {
        reused,
        tls_version: tb.tls_version,
        client_cert: tb.client_cert,
        server_addr: tb.server_addr,
        dns_ms: tb.dns_ms,
        tcp_ms: tb.tcp_ms,
        tls_ms: tb.tls_ms,
        wait_ms: tb.wait_ms,
        download_ms: tb.download_ms,
    })
}

fn share_flow(
    flow: &Flow,
    previous: Option<SystemTime>,
    redact: Option<&RedactOptions>,
) -> ShareFlow {
    let req_ct = crate::present::content_type(&flow.request_headers);
    let res_ct = crate::present::content_type(&flow.response_headers);
    let request_body_spilled = flow.request_spill.is_some()
        && flow.request_body.is_empty()
        && flow.request_expired.is_none();
    let response_body_spilled = flow.response_spill.is_some()
        && flow.response_body.is_empty()
        && flow.response_expired.is_none();
    let (
        request_body,
        request_body_pretty,
        request_body_kind,
        request_form_rows,
        request_hex_b64,
        request_raw_len,
    ) = if request_body_spilled {
        (
            String::new(),
            String::new(),
            classify_body_kind(req_ct, b" ").trim().to_string(),
            Vec::new(),
            String::new(),
            0,
        )
    } else {
        let (rb, rbp) = share_body_pair(
            &flow.request_body,
            flow.request_truncated,
            flow.request_expired,
            req_ct,
            redact,
        );
        let raw_len = if flow.request_body == STRICT_REDACTED_BODY {
            0
        } else {
            flow.request_body.len()
        };
        (
            rb,
            rbp,
            classify_body_kind(req_ct, &flow.request_body),
            extract_form_rows(req_ct, &flow.request_body),
            extract_hex_b64(&flow.request_body),
            raw_len,
        )
    };
    let (
        response_body,
        response_body_pretty,
        response_body_kind,
        response_hex_b64,
        response_raw_len,
        response_image_data_url,
        response_image_dims,
    ) = if response_body_spilled {
        (
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            0,
            None,
            None,
        )
    } else {
        let (rb, rbp) = share_body_pair(
            &flow.response_body,
            flow.response_truncated,
            flow.response_expired,
            res_ct,
            redact,
        );
        let raw_len = if flow.response_body == STRICT_REDACTED_BODY {
            0
        } else {
            flow.response_body.len()
        };
        let (img_url, img_dims) = extract_safe_image_preview(res_ct, &flow.response_body);
        (
            rb,
            rbp,
            classify_body_kind(res_ct, &flow.response_body),
            extract_hex_b64(&flow.response_body),
            raw_len,
            img_url,
            img_dims,
        )
    };
    let ttfb_ms = flow
        .response_at
        .and_then(|at| at.duration_since(flow.started).ok())
        .map(|d| d.as_millis());
    let total_ms = flow
        .finished_at
        .and_then(|at| at.duration_since(flow.started).ok())
        .map(|d| d.as_millis());
    let started_ms = flow
        .started
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let (messages, message_items, messages_truncated) = extract_messages_bundle(flow, res_ct);
    let req_bytes = flow.req_bytes() as u64;
    let res_bytes = flow.res_bytes() as u64;
    let client_ip = if !flow.client_ip.is_empty() {
        flow.client_ip.clone()
    } else if let Some(src) = flow.relay_source.as_deref() {
        crate::session::relay_label(src)
    } else {
        String::new()
    };
    let graphql_op = if request_body_spilled || flow.request_body == STRICT_REDACTED_BODY {
        crate::present::graphql_operation_label(&flow.path, &[])
    } else {
        crate::present::graphql_operation_label(&flow.path, &flow.request_body)
    };

    ShareFlow {
        id: flow.id,
        method: flow.method.clone(),
        status: flow.status,
        error: flow.error.clone(),
        protocol: flow.protocol.label().to_string(),
        host: flow.host.clone(),
        path: flow.path.clone(),
        url: flow.url.clone(),
        started: crate::present::format_clock(flow.started),
        started_ms,
        gap: crate::present::format_gap(previous, flow.started),
        elapsed: crate::present::format_elapsed(flow.started, flow.finished_at),
        slow: flow.finished_at.is_some()
            && crate::present::is_slow(
                flow.started,
                flow.finished_at,
                flow.finished_at.unwrap_or(flow.started),
            ),
        size: format_bytes_short(res_bytes),
        request_bytes: req_bytes,
        response_bytes: res_bytes,
        ttfb_ms,
        total_ms,
        pinned: flow.pinned,
        rule_hit: flow.rule_hit.clone(),
        comment: flow.comment.clone(),
        client_ip,
        relay_source: flow.relay_source.clone(),
        graphql_op,
        conn_info: share_conn_info(flow),
        query: crate::present::query_pairs(&flow.url),
        request_headers: flow.request_headers.clone(),
        response_headers: flow.response_headers.clone(),
        request_body_spilled,
        response_body_spilled,
        request_body,
        request_body_pretty,
        response_body,
        response_body_pretty,
        request_body_kind: if request_body_spilled {
            String::new()
        } else {
            request_body_kind
        },
        response_body_kind,
        request_form_rows,
        request_hex_b64,
        response_hex_b64,
        request_raw_len,
        response_raw_len,
        request_truncated: flow.request_truncated,
        response_truncated: flow.response_truncated,
        request_expired: flow.request_expired,
        response_expired: flow.response_expired,
        request_encoding_error: flow.request_encoding_error.clone(),
        response_encoding_error: flow.response_encoding_error.clone(),
        response_image_data_url,
        response_image_dims,
        messages,
        message_items,
        messages_truncated,
    }
}

fn share_body_pair(
    bytes: &[u8],
    truncated: bool,
    expired: Option<usize>,
    content_type: Option<&str>,
    redact: Option<&RedactOptions>,
) -> (String, String) {
    if bytes.is_empty() {
        if let Some(exp) = expired {
            let msg = format!(
                "[large body ({}) reclaimed]",
                format_bytes_short(exp as u64)
            );
            return (msg, String::new());
        }
        return if truncated {
            ("…".into(), String::new())
        } else {
            (String::new(), String::new())
        };
    }
    let raw = share_body(bytes, truncated);
    if bytes == STRICT_REDACTED_BODY {
        return (raw, String::new());
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        let pretty_full = crate::present::pretty_body(content_type, text);
        if pretty_full != text {
            let mut pretty = clip_chars(&pretty_full, BODY_CHARS);
            if pretty_full.chars().count() > BODY_CHARS || truncated {
                pretty.push_str("\n…");
            }
            return (raw, pretty);
        }
    }
    if let Some(mut decoded) = crate::present::pretty_binary_body(content_type, bytes) {
        if let Some(opts) = redact
            && opts.enabled
        {
            if crate::present::is_msgpack(content_type)
                && let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&decoded)
            {
                let _ = redact_json_value(&mut val, opts);
                if let Ok(sanitized_pretty) = serde_json::to_string_pretty(&val) {
                    decoded = sanitized_pretty;
                }
            } else {
                let _ = redact_string_value(&mut decoded, opts, 0);
            }
        }
        let mut pretty = clip_chars(&decoded, BODY_CHARS);
        if decoded.chars().count() > BODY_CHARS || truncated {
            pretty.push_str("\n…");
        }
        return (raw, pretty);
    }
    (raw, String::new())
}

fn share_body(bytes: &[u8], truncated: bool) -> String {
    if bytes.is_empty() {
        return if truncated {
            "…".into()
        } else {
            String::new()
        };
    }
    let text = String::from_utf8_lossy(bytes);
    let broken = text.chars().filter(|ch| *ch == '\u{FFFD}').count();
    if broken > 0 && bytes.len() > 64 && broken.saturating_mul(8) > bytes.len() {
        return format!("[{} bytes]", bytes.len());
    }
    let mut out = clip_chars(&text, BODY_CHARS);
    if text.chars().count() > BODY_CHARS || truncated {
        out.push_str("\n…");
    }
    out
}

fn clip_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// `live` is the polling path. An empty live path is a file that does not refresh.
pub fn page(json: &str, live: &str, zh: bool) -> String {
    let safe_json = json
        .replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    let safe_live = live
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('<', "");
    let lang = if zh { "zh-CN" } else { "en" };
    let title = if zh { "zniff 分享" } else { "zniff share" };
    let zh_bool = if zh { "true" } else { "false" };
    let shell = PAGE_TEMPLATE
        .replace("__ZNIFF_LANG__", lang)
        .replace("__ZNIFF_TITLE__", title)
        .replace("__ZNIFF_LIVE__", &safe_live)
        .replace("__ZNIFF_ZH_BOOL__", zh_bool);
    if let Some((head, tail)) = shell.split_once("__ZNIFF_INITIAL_JSON__") {
        let mut out = String::with_capacity(head.len() + safe_json.len() + tail.len());
        out.push_str(head);
        out.push_str(&safe_json);
        out.push_str(tail);
        out
    } else {
        shell
    }
}

const PAGE_TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="__ZNIFF_LANG__">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="icon" href="data:,">
<title>__ZNIFF_TITLE__</title>
<style>
  :root {
    --bg-canvas: #0d1017;
    --bg-chrome: #131822;
    --bg-list: #10141d;
    --bg-list-header: #151b26;
    --bg-row-odd: #10141d;
    --bg-row-even: #131823;
    --bg-row-hover: #1a2130;
    --bg-row-selected: #172554;
    --bg-card: #131822;
    --bg-code: #0f131b;
    --bg-control: #1c2331;
    --bg-control-hover: #252e40;
    --border-strong: #222938;
    --border-subtle: #19202e;
    --border-control: #2b3448;
    --fg-primary: #f1f5f9;
    --fg-secondary: #cbd5e1;
    --fg-muted: #8e9bb3;
    --accent: #3b82f6;
    --accent-blue: #3b82f6;
    --accent-blue-soft: #172554;
    --accent-blue-border: #1e40af;
    --accent-blue-text: #93c5fd;
    --ok-fg: #34d399; --ok-bg: #0a2e26;
    --warn-fg: #fbbf24; --warn-bg: #33240b;
    --err-fg: #fb7185; --err-bg: #361320;
    --m-get-fg: #38bdf8; --m-get-bg: #0c2a42;
    --m-post-fg: #34d399; --m-post-bg: #0a2e26;
    --m-put-fg: #fbbf24; --m-put-bg: #33240b;
    --m-del-fg: #fb7185; --m-del-bg: #361320;
    --m-con-fg: #a78bfa; --m-con-bg: #241842;
    --t-dns: #38bdf8; --t-tcp: #fbbf24; --t-tls: #a78bfa; --t-wait: #34d399; --t-dl: #60a5fa;
    --diff-del-bg: rgba(251, 113, 133, 0.14); --diff-del-fg: #fda4af;
    --diff-add-bg: rgba(52, 211, 153, 0.14); --diff-add-fg: #6ee7b7;
    --shadow-pop: 0 12px 32px rgba(0, 0, 0, 0.52);
  }
  html[data-theme="light"] {
    --bg-canvas: #f8fafc;
    --bg-chrome: #ffffff;
    --bg-list: #ffffff;
    --bg-list-header: #f1f5f9;
    --bg-row-odd: #ffffff;
    --bg-row-even: #f8fafc;
    --bg-row-hover: #eff6ff;
    --bg-row-selected: #dbeafe;
    --bg-card: #ffffff;
    --bg-code: #f8fafc;
    --bg-control: #f1f5f9;
    --bg-control-hover: #e2e8f0;
    --border-strong: #e2e8f0;
    --border-subtle: #f1f5f9;
    --border-control: #cbd5e1;
    --fg-primary: #0f172a;
    --fg-secondary: #334155;
    --fg-muted: #526075;
    --accent: #3b82f6;
    --accent-blue: #2563eb;
    --accent-blue-soft: #dbeafe;
    --accent-blue-border: #93c5fd;
    --accent-blue-text: #1d4ed8;
    --ok-fg: #047857; --ok-bg: #d1fae5;
    --warn-fg: #b45309; --warn-bg: #fef3c7;
    --err-fg: #9f1239; --err-bg: #ffe4e6;
    --m-get-fg: #0284c7; --m-get-bg: #e0f2fe;
    --m-post-fg: #047857; --m-post-bg: #d1fae5;
    --m-put-fg: #b45309; --m-put-bg: #fef3c7;
    --m-del-fg: #9f1239; --m-del-bg: #ffe4e6;
    --m-con-fg: #6d28d9; --m-con-bg: #ede9fe;
    --t-dns: #0284c7; --t-tcp: #d97706; --t-tls: #7c3aed; --t-wait: #059669; --t-dl: #2563eb;
    --diff-del-bg: #ffe4e6; --diff-del-fg: #9f1239;
    --diff-add-bg: #d1fae5; --diff-add-fg: #065f46;
    --shadow-pop: 0 12px 32px rgba(15, 23, 42, 0.16);
  }
  * { box-sizing: border-box; }
  html, body { margin: 0; height: 100%; overflow: hidden; background: var(--bg-canvas); color: var(--fg-primary); font: 13px/1.45 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; -webkit-font-smoothing: antialiased; }
  button, input { font-family: inherit; }
  .mono { font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; }
  .shell { display: flex; flex-direction: column; height: 100vh; width: 100vw; overflow: hidden; }
  .topbar { height: 44px; flex-shrink: 0; background: var(--bg-chrome); border-bottom: 1px solid var(--border-strong); padding: 0 12px; display: flex; align-items: center; justify-content: space-between; gap: 10px; min-width: 0; }
  .brand { display: flex; align-items: center; gap: 8px; min-width: 0; flex: 1; overflow: hidden; }
  .logo { width: 22px; height: 22px; border-radius: 6px; background: var(--accent); color: #fff; font-weight: 800; font-size: 12px; display: inline-flex; align-items: center; justify-content: center; flex-shrink: 0; }
  .brand-name { font-weight: 700; font-size: 14px; letter-spacing: -0.01em; flex-shrink: 0; }
  .status-dot { width: 7px; height: 7px; border-radius: 99px; background: var(--ok-fg); flex-shrink: 0; }
  .status-dot.ended { background: var(--err-fg); }
  .status-dot.snapshot { background: var(--accent); }
  .meta-text { color: var(--fg-muted); font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; }
  .toolbar-right { display: flex; align-items: center; gap: 6px; flex-shrink: 0; }
  .pill { height: 28px; padding: 0 9px; border-radius: 6px; background: var(--bg-control); border: 1px solid var(--border-control); color: var(--fg-primary); font-size: 12px; font-weight: 500; display: inline-flex; align-items: center; gap: 5px; cursor: pointer; user-select: none; white-space: nowrap; }
  .pill:hover { background: var(--bg-control-hover); }
  .pill.active { background: var(--accent-blue-soft); border-color: var(--accent-blue-border); color: var(--accent-blue-text); }
  .seg-group { height: 28px; display: inline-flex; align-items: center; background: var(--bg-code); border: 1px solid var(--border-control); border-radius: 6px; padding: 2px; gap: 1px; }
  .seg-btn { height: 22px; padding: 0 7px; border-radius: 4px; border: none; background: transparent; color: var(--fg-muted); font-size: 11.5px; cursor: pointer; white-space: nowrap; }
  .seg-btn:hover { color: var(--fg-primary); }
  .seg-btn.active { background: var(--accent-blue-soft); color: var(--accent-blue-text); font-weight: 600; }
  .workspace { flex: 1; min-height: 0; display: flex; overflow: hidden; }
  .left-pane { width: 660px; min-width: 280px; max-width: 1100px; flex-shrink: 0; background: var(--bg-list); display: flex; flex-direction: column; min-height: 0; overflow: hidden; }
  .splitter { width: 5px; flex-shrink: 0; background: var(--border-strong); cursor: col-resize; transition: background 0.12s; }
  .splitter:hover, .splitter.dragging { background: var(--accent); }
  .filter-bar { padding: 7px 8px 6px; background: var(--bg-chrome); border-bottom: 1px solid var(--border-strong); display: flex; flex-direction: column; gap: 6px; flex-shrink: 0; }
  .filter-input-wrap { position: relative; display: flex; align-items: center; min-width: 0; flex: 1; }
  .filter-input { width: 100%; height: 29px; padding: 0 28px 0 9px; border-radius: 6px; border: 1px solid var(--border-control); background: var(--bg-code); color: var(--fg-primary); font-size: 12px; outline: none; }
  .filter-input:focus { border-color: var(--accent); }
  .filter-clear { position: absolute; right: 5px; width: 18px; height: 18px; border-radius: 4px; border: none; background: transparent; color: var(--fg-muted); cursor: pointer; display: none; align-items: center; justify-content: center; font-size: 14px; padding: 0; }
  .filter-clear:hover { color: var(--fg-primary); background: var(--bg-control); }
  .chips-row { display: flex; align-items: center; gap: 5px; flex-wrap: wrap; }
  .chip { height: 22px; padding: 0 8px; border-radius: 5px; border: 1px solid var(--border-control); background: var(--bg-control); color: var(--fg-secondary); font-size: 11px; cursor: pointer; display: inline-flex; align-items: center; gap: 4px; user-select: none; }
  .chip:hover { background: var(--bg-control-hover); }
  .chip.active { background: var(--accent-blue-soft); border-color: var(--accent-blue-border); color: var(--accent-blue-text); }
  .chip-dot { width: 5px; height: 5px; border-radius: 99px; background: var(--fg-muted); }
  .chip.active .chip-dot { background: var(--accent); }
  .sub-bar { padding: 5px 8px; background: var(--bg-chrome); border-bottom: 1px solid var(--border-strong); display: flex; align-items: center; gap: 6px; flex-shrink: 0; }
  .sub-input { width: 100%; height: 26px; padding: 0 92px 0 8px; border-radius: 5px; border: 1px solid var(--border-control); background: var(--bg-code); color: var(--fg-primary); font-size: 11.5px; outline: none; }
  .sub-input:focus { border-color: var(--accent); }
  .sub-actions { position: absolute; right: 4px; display: inline-flex; align-items: center; gap: 3px; }
  .merge-btn { height: 20px; padding: 0 6px; border-radius: 4px; border: 1px solid var(--accent-blue-border); background: var(--accent-blue-soft); color: var(--accent-blue-text); font-size: 10.5px; cursor: pointer; display: none; white-space: nowrap; }
  .icon-btn { height: 26px; padding: 0 8px; border-radius: 5px; border: 1px solid var(--border-control); background: var(--bg-control); color: var(--fg-secondary); font-size: 11.5px; cursor: pointer; display: inline-flex; align-items: center; gap: 4px; flex-shrink: 0; user-select: none; }
  .icon-btn:hover { background: var(--bg-control-hover); color: var(--fg-primary); }
  .icon-btn.active { background: var(--accent-blue-soft); border-color: var(--accent-blue-border); color: var(--accent-blue-text); }
  .facet-drawer { display: none; background: var(--bg-chrome); border-bottom: 1px solid var(--border-strong); padding: 8px; max-height: 196px; overflow-y: auto; flex-shrink: 0; flex-direction: column; gap: 8px; }
  .facet-drawer.open { display: flex; }
  .facet-sec-head { display: flex; align-items: center; justify-content: space-between; font-size: 11px; font-weight: 600; color: var(--fg-muted); margin-bottom: 4px; }
  .facet-pills { display: flex; flex-wrap: wrap; gap: 5px; }
  .facet-pill { display: inline-flex; align-items: center; height: 22px; border-radius: 5px; border: 1px solid var(--border-control); background: var(--bg-control); color: var(--fg-secondary); font-size: 11px; overflow: hidden; }
  .facet-pill.active { background: var(--accent-blue-soft); border-color: var(--accent-blue-border); color: var(--accent-blue-text); }
  .facet-pill.muted { background: var(--err-bg); border-color: var(--err-fg); color: var(--err-fg); text-decoration: line-through; }
  .facet-main { height: 100%; padding: 0 7px; border: none; background: transparent; color: inherit; font-size: 11px; cursor: pointer; display: inline-flex; align-items: center; gap: 4px; max-width: 220px; }
  .facet-main:hover { background: rgba(148, 163, 184, 0.12); }
  .facet-lbl { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .facet-cnt { color: var(--fg-muted); font-size: 10.5px; }
  .facet-err { color: var(--err-fg); font-weight: 700; font-size: 10.5px; }
  .facet-mute { height: 100%; padding: 0 5px; border: none; border-left: 1px solid var(--border-control); background: transparent; color: var(--fg-muted); cursor: pointer; font-size: 12px; line-height: 1; }
  .facet-mute:hover { background: var(--err-bg); color: var(--err-fg); }
  .table-scroll { flex: 1; min-height: 0; overflow: auto; position: relative; }
  table.flow-table { width: 100%; border-collapse: separate; border-spacing: 0; table-layout: fixed; }
  .flow-table thead th { position: sticky; top: 0; z-index: 3; height: 28px; padding: 0 6px; background: var(--bg-list-header); border-bottom: 1px solid var(--border-strong); color: var(--fg-muted); font-size: 11px; font-weight: 600; text-align: left; white-space: nowrap; cursor: pointer; user-select: none; overflow: hidden; text-overflow: ellipsis; }
  .flow-table thead th:hover { color: var(--fg-primary); }
  .flow-table thead th.sticky-col { z-index: 5; background: var(--bg-list-header); }
  .flow-table thead th.num, .flow-table tbody td.num { text-align: right; }
  .flow-table tbody tr { height: 29px; cursor: pointer; user-select: none; background: var(--bg-row-odd); }
  .flow-table tbody tr:nth-child(even) { background: var(--bg-row-even); }
  .flow-table tbody tr:hover { background: var(--bg-row-hover); }
  .flow-table tbody tr.selected { background: var(--bg-row-selected); }
  .flow-table tbody td { padding: 0 6px; height: 29px; border-bottom: 1px solid var(--border-subtle); font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; background: inherit; vertical-align: middle; }
  .flow-table tbody td.sticky-col { position: sticky; z-index: 2; background: inherit; }
  .flow-table .sticky-edge { border-right: 1px solid var(--border-subtle); }
  .flow-table tbody tr.selected td.lead-cell { box-shadow: inset 3px 0 0 var(--accent); }
  .flow-table tbody tr.baseline:not(.selected) td.lead-cell { box-shadow: inset 3px 0 0 var(--err-fg); }
  .flow-table tbody tr.baseline td.method-cell .base-bar { display: inline-block; }
  .base-bar { display: none; width: 3px; height: 14px; border-radius: 2px; background: var(--err-fg); flex-shrink: 0; }
  .method-wrap { display: inline-flex; align-items: center; gap: 4px; max-width: 100%; }
  .pin-slot { width: 7px; height: 7px; display: inline-flex; align-items: center; justify-content: center; flex-shrink: 0; }
  .pin-dot { width: 6px; height: 6px; border-radius: 99px; background: var(--accent); }
  .m-badge { padding: 1px 5px; border-radius: 4px; font-size: 10.5px; font-weight: 700; line-height: 1.35; background: var(--bg-control); color: var(--fg-secondary); display: inline-block; }
  .m-GET { color: var(--m-get-fg); background: var(--m-get-bg); }
  .m-POST { color: var(--m-post-fg); background: var(--m-post-bg); }
  .m-PUT, .m-PATCH { color: var(--m-put-fg); background: var(--m-put-bg); }
  .m-DELETE { color: var(--m-del-fg); background: var(--m-del-bg); }
  .m-CONNECT { color: var(--m-con-fg); background: var(--m-con-bg); }
  .relay-tag { padding: 0 5px; border-radius: 4px; font-size: 10px; background: var(--bg-control); color: var(--fg-muted); border: 1px solid var(--border-control); margin-right: 4px; }
  .row-note { color: var(--accent-blue-text); font-size: 11px; margin-left: 4px; }
  .s-ok { color: var(--ok-fg) !important; font-weight: 600; }
  .s-warn { color: var(--warn-fg) !important; font-weight: 600; }
  .s-err { color: var(--err-fg) !important; font-weight: 600; }
  .slow { color: var(--warn-fg) !important; font-weight: 600; }
  .muted-cell { color: var(--fg-muted); }
  .path-cell { color: var(--fg-secondary); }
  .path-cell.failed { color: var(--err-fg); }
  .right-pane { flex: 1; min-width: 340px; background: var(--bg-canvas); overflow-y: auto; padding: 10px; display: flex; flex-direction: column; gap: 10px; }
  .empty-state { margin: auto; color: var(--fg-muted); font-size: 12.5px; text-align: center; padding: 28px 16px; }
  .card { background: var(--bg-card); border: 1px solid var(--border-strong); border-radius: 8px; overflow: hidden; flex-shrink: 0; }
  .summary-card { padding: 10px 12px; display: flex; flex-direction: column; gap: 8px; }
  .summary-top { display: flex; align-items: center; justify-content: space-between; gap: 8px; flex-wrap: wrap; }
  .summary-badges { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .s-badge { padding: 2px 7px; border-radius: 4px; font-size: 11px; font-weight: 700; }
  .s-badge.info { color: var(--accent-blue-text); background: var(--accent-blue-soft); }
  .s-badge.ok { color: var(--ok-fg); background: var(--ok-bg); }
  .s-badge.warn { color: var(--warn-fg); background: var(--warn-bg); }
  .s-badge.err { color: var(--err-fg); background: var(--err-bg); }
  .proto-badge { padding: 2px 7px; border-radius: 4px; font-size: 11px; background: var(--bg-control); color: var(--fg-secondary); }
  .gql-badge { padding: 2px 7px; border-radius: 4px; font-size: 11px; background: var(--m-con-bg); color: var(--m-con-fg); font-weight: 600; }
  .rule-badge { padding: 2px 7px; border-radius: 4px; font-size: 11px; background: var(--accent-blue-soft); color: var(--accent-blue-text); border: 1px solid var(--accent-blue-border); }
  .url-box { padding: 7px 9px; border-radius: 6px; background: var(--bg-code); border: 1px solid var(--border-subtle); color: var(--fg-primary); font-size: 12px; word-break: break-all; line-height: 1.45; }
  .note-box { padding: 5px 9px; border-radius: 6px; background: var(--accent-blue-soft); border: 1px solid var(--accent-blue-border); color: var(--accent-blue-text); font-size: 12px; }
  .err-box { padding: 6px 9px; border-radius: 6px; background: var(--err-bg); color: var(--err-fg); font-size: 12px; }
  .meta-grid { display: flex; flex-wrap: wrap; gap: 6px 14px; color: var(--fg-muted); font-size: 11.5px; }
  .meta-item b { color: var(--fg-secondary); font-weight: 600; }
  .timing-box { padding: 8px 10px; border-radius: 6px; background: var(--bg-code); border: 1px solid var(--border-subtle); display: flex; flex-direction: column; gap: 6px; }
  .timing-meta { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; font-size: 11px; color: var(--fg-secondary); }
  .timing-bar { height: 7px; border-radius: 99px; background: var(--bg-control); overflow: hidden; display: flex; width: 100%; }
  .timing-seg { height: 100%; min-width: 2px; }
  .timing-legend { display: flex; flex-wrap: wrap; gap: 6px 12px; font-size: 11px; color: var(--fg-muted); }
  .t-dot { width: 7px; height: 7px; border-radius: 99px; display: inline-block; margin-right: 4px; }
  .stats-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 8px; }
  .stat-tile { padding: 8px 10px; border-radius: 6px; background: var(--bg-code); border: 1px solid var(--border-subtle); }
  .stat-lbl { font-size: 11px; color: var(--fg-muted); }
  .stat-val { font-size: 16px; font-weight: 700; color: var(--fg-primary); margin-top: 2px; }
  .stat-sub { font-size: 11px; color: var(--fg-muted); margin-top: 2px; }
  .slowest-row { padding: 7px 10px; border-radius: 6px; background: var(--bg-code); border: 1px solid var(--border-control); display: flex; align-items: center; justify-content: space-between; gap: 8px; cursor: pointer; font-size: 12px; }
  .slowest-row:hover { background: var(--bg-row-hover); border-color: var(--accent); }
  .card-head { min-height: 34px; padding: 4px 10px; display: flex; align-items: center; justify-content: space-between; gap: 8px; cursor: pointer; user-select: none; background: var(--bg-card); flex-wrap: wrap; }
  .card-head:hover { background: var(--bg-row-hover); }
  .card-title-wrap { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .card-arrow { font-size: 10px; color: var(--fg-muted); width: 12px; display: inline-block; transition: transform 0.12s; }
  .card.folded .card-arrow { transform: rotate(-90deg); }
  .card.folded .card-body { display: none; }
  .card-title { font-size: 12px; font-weight: 600; color: var(--fg-primary); white-space: nowrap; }
  .count-badge { padding: 0 6px; height: 18px; border-radius: 99px; background: var(--bg-control); color: var(--fg-muted); font-size: 11px; font-weight: 600; display: inline-flex; align-items: center; }
  .card-actions { display: flex; align-items: center; gap: 5px; flex-wrap: wrap; }
  .mini-input { height: 22px; width: 132px; padding: 0 6px; border-radius: 4px; border: 1px solid var(--border-control); background: var(--bg-code); color: var(--fg-primary); font-size: 11px; outline: none; }
  .mini-input:focus { border-color: var(--accent); }
  .mini-btn { height: 22px; padding: 0 8px; border-radius: 5px; border: 1px solid var(--border-control); background: var(--bg-control); color: var(--fg-secondary); font-size: 11px; cursor: pointer; display: inline-flex; align-items: center; gap: 4px; white-space: nowrap; }
  .mini-btn:hover { background: var(--bg-control-hover); color: var(--fg-primary); }
  .mini-btn.active { background: var(--accent-blue-soft); border-color: var(--accent-blue-border); color: var(--accent-blue-text); }
  .mini-btn.primary { background: var(--accent); border-color: var(--accent); color: #fff; font-weight: 600; }
  .mini-btn.primary:hover { filter: brightness(1.08); }
  .card-body { border-top: 1px solid var(--border-strong); }
  .body-note { padding: 5px 12px; font-size: 11.5px; color: var(--warn-fg); background: var(--warn-bg); border-bottom: 1px solid var(--border-subtle); }
  .kv-row { display: grid; grid-template-columns: 180px minmax(0, 1fr); gap: 10px; padding: 6px 12px; font-size: 12px; line-height: 1.45; border-top: 1px solid var(--border-subtle); }
  .kv-row:first-child { border-top: none; }
  .kv-key { color: var(--fg-muted); word-break: break-all; font-weight: 500; }
  .kv-val { color: var(--fg-primary); word-break: break-all; white-space: pre-wrap; min-width: 0; }
  .kv-hint { margin-top: 3px; padding: 3px 7px; border-radius: 4px; background: var(--bg-code); border: 1px solid var(--border-control); color: var(--accent-blue-text); font-size: 11px; white-space: pre-wrap; word-break: break-all; }
  .code-viewer { max-height: 260px; overflow: auto; background: var(--bg-code); font-size: 12px; line-height: 1.5; }
  .code-viewer.expanded { max-height: 640px; }
  .code-viewer.hex-dump { font-size: 11px; line-height: 1.45; }
  .code-viewer.hex-dump .code-line { padding: 0 8px; }
  .code-viewer.hex-dump .ln { width: 34px; padding-right: 8px; }
  .code-line { display: flex; padding: 0 10px; min-width: max-content; }
  .code-line:hover { background: var(--bg-row-hover); }
  .ln { width: 46px; flex-shrink: 0; color: var(--fg-muted); user-select: none; padding-right: 10px; text-align: right; opacity: 0.75; }
  .lc { white-space: pre; color: var(--fg-secondary); }
  mark.hl { background: var(--warn-bg); color: var(--warn-fg); border-radius: 2px; padding: 0 1px; }
  .img-preview-wrap { padding: 12px; background: var(--bg-code); display: flex; flex-direction: column; align-items: flex-start; gap: 8px; }
  .img-checker { padding: 8px; border-radius: 6px; border: 1px solid var(--border-control); background-image: linear-gradient(45deg, rgba(148,163,184,0.12) 25%, transparent 25%), linear-gradient(-45deg, rgba(148,163,184,0.12) 25%, transparent 25%), linear-gradient(45deg, transparent 75%, rgba(148,163,184,0.12) 75%), linear-gradient(-45deg, transparent 75%, rgba(148,163,184,0.12) 75%); background-size: 16px 16px; background-position: 0 0, 0 8px, 8px -8px, -8px 0; }
  .img-checker img { display: block; max-width: 200px; max-height: 200px; width: auto; height: auto; }
  .html-preview-frame { width: 100%; height: 280px; border: none; background: #ffffff; display: block; }
  .md-preview { padding: 12px 14px; background: var(--bg-code); color: var(--fg-primary); font-size: 13px; line-height: 1.6; max-height: 320px; overflow: auto; }
  .md-preview h1, .md-preview h2, .md-preview h3 { margin: 8px 0 6px; line-height: 1.3; }
  .md-preview pre { padding: 8px 10px; border-radius: 6px; background: var(--bg-card); border: 1px solid var(--border-strong); overflow-x: auto; }
  .md-preview code { padding: 1px 4px; border-radius: 4px; background: var(--bg-control); font-size: 12px; }
  .diff-viewer { max-height: 340px; overflow: auto; background: var(--bg-code); font-size: 12px; line-height: 1.45; }
  .diff-line { display: flex; padding: 1px 10px; white-space: pre; min-width: max-content; }
  .diff-line.del { background: var(--diff-del-bg); color: var(--diff-del-fg); }
  .diff-line.add { background: var(--diff-add-bg); color: var(--diff-add-fg); }
  .diff-line.ctx { color: var(--fg-muted); }
  .diff-sign { width: 18px; flex-shrink: 0; user-select: none; font-weight: 700; }
  .msg-list { max-height: 320px; overflow-y: auto; }
  .msg-row { display: flex; align-items: flex-start; gap: 8px; padding: 6px 10px; border-top: 1px solid var(--border-subtle); font-size: 12px; }
  .msg-row:first-child { border-top: none; }
  .msg-dir { padding: 1px 6px; border-radius: 4px; font-size: 10.5px; font-weight: 700; flex-shrink: 0; }
  .msg-dir.up { background: var(--m-get-bg); color: var(--m-get-fg); }
  .msg-dir.down { background: var(--m-post-bg); color: var(--m-post-fg); }
  .msg-kind { padding: 1px 5px; border-radius: 4px; font-size: 10.5px; background: var(--bg-control); color: var(--fg-muted); flex-shrink: 0; }
  .msg-text { flex: 1; min-width: 0; white-space: pre-wrap; word-break: break-all; color: var(--fg-primary); }
  .pop-menu { position: fixed; z-index: 60; min-width: 196px; max-width: 280px; max-height: calc(100vh - 24px); overflow-y: auto; background: var(--bg-card); border: 1px solid var(--border-control); border-radius: 8px; box-shadow: var(--shadow-pop); padding: 4px; display: none; }
  .pop-menu.open { display: block; }
  .menu-sec { padding: 4px 8px 2px; font-size: 10.5px; font-weight: 600; color: var(--fg-muted); }
  .menu-sep { height: 1px; background: var(--border-strong); margin: 4px 2px; }
  .menu-item { width: 100%; min-height: 27px; padding: 3px 8px; border: none; border-radius: 5px; background: transparent; color: var(--fg-primary); font-size: 12px; text-align: left; cursor: pointer; display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .menu-item:hover:not(:disabled) { background: var(--bg-row-hover); }
  .menu-item:disabled { opacity: 0.45; cursor: not-allowed; }
  .col-row { display: flex; align-items: center; justify-content: space-between; padding: 2px 6px; border-radius: 5px; font-size: 12px; }
  .col-row:hover { background: var(--bg-row-hover); }
  .col-row label { display: flex; align-items: center; gap: 6px; cursor: pointer; flex: 1; }
  .col-ord-btns { display: inline-flex; gap: 2px; }
  .col-ord-btn { width: 20px; height: 20px; border-radius: 4px; border: 1px solid var(--border-control); background: var(--bg-control); color: var(--fg-muted); font-size: 10px; cursor: pointer; padding: 0; }
  .col-ord-btn:hover:not(:disabled) { color: var(--fg-primary); background: var(--bg-control-hover); }
  .col-ord-btn:disabled { opacity: 0.35; cursor: default; }
  .modal-backdrop { position: fixed; inset: 0; z-index: 80; background: rgba(2, 6, 23, 0.62); display: none; align-items: center; justify-content: center; padding: 24px; }
  .modal-backdrop.open { display: flex; }
  .modal-box { width: min(1120px, calc(100vw - 48px)); max-height: calc(100vh - 64px); background: var(--bg-card); border: 1px solid var(--border-control); border-radius: 10px; box-shadow: var(--shadow-pop); display: flex; flex-direction: column; overflow: hidden; }
  .modal-head { padding: 10px 14px; background: var(--bg-chrome); border-bottom: 1px solid var(--border-strong); display: flex; align-items: center; justify-content: space-between; gap: 10px; flex-shrink: 0; flex-wrap: wrap; }
  .modal-sub { padding: 7px 14px; background: var(--bg-list-header); border-bottom: 1px solid var(--border-strong); display: flex; align-items: center; justify-content: space-between; gap: 8px; flex-shrink: 0; flex-wrap: wrap; font-size: 12px; }
  .modal-body { flex: 1; min-height: 0; display: flex; overflow: hidden; }
  .seq-pick-list { width: 250px; flex-shrink: 0; border-right: 1px solid var(--border-strong); overflow-y: auto; padding: 6px; background: var(--bg-list); }
  .seq-pick-item { display: flex; align-items: center; gap: 6px; padding: 4px 6px; border-radius: 5px; font-size: 11.5px; cursor: pointer; }
  .seq-pick-item:hover { background: var(--bg-row-hover); }
  .seq-preview { flex: 1; min-width: 0; overflow: auto; padding: 10px 12px; background: var(--bg-canvas); display: flex; flex-direction: column; gap: 6px; }
  .wf-row, .seq-step-row, .dep-card { padding: 6px 8px; border-radius: 6px; background: var(--bg-card); border: 1px solid var(--border-subtle); cursor: pointer; display: flex; align-items: center; gap: 8px; font-size: 12px; }
  .wf-row:hover, .seq-step-row:hover, .dep-card:hover { border-color: var(--accent); background: var(--bg-row-hover); }
  .wf-track { flex: 1; min-width: 140px; height: 10px; border-radius: 99px; background: var(--bg-code); position: relative; overflow: hidden; }
  .wf-bar { position: absolute; top: 0; bottom: 0; min-width: 3px; border-radius: 99px; display: flex; overflow: hidden; }
  .toast { position: fixed; bottom: 36px; right: 16px; z-index: 100; padding: 6px 12px; border-radius: 6px; background: var(--accent); color: #fff; font-size: 12px; font-weight: 600; box-shadow: var(--shadow-pop); opacity: 0; pointer-events: none; transition: opacity 0.16s; }
  .toast.show { opacity: 1; }
  .bottombar { height: 26px; flex-shrink: 0; background: var(--bg-chrome); border-top: 1px solid var(--border-strong); padding: 0 12px; display: flex; align-items: center; justify-content: space-between; font-size: 11.5px; color: var(--fg-muted); }
  @media (max-width: 860px) {
    .workspace { flex-direction: column; }
    .left-pane { width: 100% !important; max-width: none; height: 46%; border-bottom: 1px solid var(--border-strong); }
    .splitter { display: none; }
    .right-pane { height: 54%; min-width: 0; }
    .kv-row { grid-template-columns: 120px minmax(0, 1fr); }
    .seq-pick-list { width: 180px; }
  }
</style>
</head>
<body>
<div class="shell">
  <header class="topbar">
    <div class="brand">
      <span class="logo">Z</span>
      <span class="brand-name">zniff</span>
      <span class="status-dot" id="status-dot"></span>
      <span class="meta-text" id="meta"></span>
    </div>
    <div class="toolbar-right">
      <button class="pill mono" id="count-pill" type="button">0</button>
      <button class="pill" id="summary-btn" type="button"></button>
      <button class="pill" id="seq-top-btn" type="button"></button>
      <button class="pill active" id="pretty-btn" type="button"></button>
      <div class="seg-group" id="theme-seg"></div>
      <div class="seg-group" id="lang-seg"></div>
    </div>
  </header>

  <div class="workspace" id="workspace">
    <section class="left-pane" id="left-pane">
      <div class="filter-bar">
        <div class="filter-input-wrap">
          <input class="filter-input" id="filter-input" type="text" autocomplete="off" spellcheck="false">
          <button class="filter-clear" id="filter-clear" type="button">×</button>
        </div>
        <div class="chips-row" id="chips-row"></div>
      </div>
      <div class="sub-bar">
        <div class="filter-input-wrap">
          <input class="sub-input" id="sub-filter-input" type="text" autocomplete="off" spellcheck="false">
          <div class="sub-actions">
            <button class="merge-btn" id="merge-sub-btn" type="button"></button>
            <button class="filter-clear" id="sub-filter-clear" type="button" style="position:static">×</button>
          </div>
        </div>
        <button class="icon-btn" id="facet-btn" type="button"></button>
        <button class="icon-btn" id="col-btn" type="button"></button>
      </div>
      <div class="facet-drawer" id="facet-drawer"></div>
      <div class="table-scroll" id="table-scroll">
        <table class="flow-table" id="flow-table">
          <colgroup id="table-colgroup"></colgroup>
          <thead id="table-head"></thead>
          <tbody id="table-body"></tbody>
        </table>
      </div>
    </section>
    <div class="splitter" id="splitter"></div>
    <section class="right-pane" id="right-pane"></section>
  </div>

  <footer class="bottombar">
    <span id="footer-left"></span>
    <span id="footer-right"></span>
  </footer>
</div>

<div class="pop-menu" id="col-menu"></div>
<div class="pop-menu" id="ctx-menu"></div>
<div class="pop-menu" id="more-menu"></div>
<div class="modal-backdrop" id="seq-modal">
  <div class="modal-box" id="seq-modal-box"></div>
</div>
<div class="toast" id="toast"></div>

<script>
const RAW_LIVE = "__ZNIFF_LIVE__";
const LIVE = RAW_LIVE
  ? ((typeof location !== "undefined" && location.pathname && /\/s\/[0-9a-fA-F-]{36}\/?$/.test(location.pathname))
      ? `${location.pathname.replace(/\/+$/, "")}/flows.json`
      : RAW_LIVE)
  : "";
const DEFAULT_ZH = __ZNIFF_ZH_BOOL__;
const INITIAL = __ZNIFF_INITIAL_JSON__;
const FIXED_DEF = [
  { key: "id", zh: "序号", en: "ID", width: 44, defOn: true, num: false },
  { key: "method", zh: "方法", en: "Method", width: 78, defOn: true, num: false },
  { key: "path", zh: "路径", en: "Path", width: 220, defOn: true, num: false, flex: true },
];
const SCROLL_DEF = [
  { key: "status", zh: "状态", en: "Status", width: 52, defOn: true, num: false },
  { key: "gap", zh: "间隔", en: "Gap", width: 66, defOn: true, num: true },
  { key: "elapsed", zh: "耗时", en: "Time", width: 68, defOn: true, num: true },
  { key: "size", zh: "大小", en: "Size", width: 66, defOn: true, num: true },
  { key: "protocol", zh: "协议", en: "Proto", width: 80, defOn: false, num: false },
  { key: "host", zh: "主机", en: "Host", width: 156, defOn: false, num: false },
  { key: "client_ip", zh: "来源 IP", en: "Client IP", width: 156, defOn: false, num: false },
];
const ALL_COL_MAP = Object.fromEntries(FIXED_DEF.concat(SCROLL_DEF).map(c => [c.key, c]));

function safeGetLS(k) { try { return localStorage.getItem(k); } catch (_) { return null; } }
function safeSetLS(k, v) { try { localStorage.setItem(k, v); } catch (_) {} }

function defaultColPrefs() {
  const vis = {};
  FIXED_DEF.concat(SCROLL_DEF).forEach(c => { vis[c.key] = c.defOn; });
  return {
    fixedOrder: FIXED_DEF.map(c => c.key),
    scrollOrder: SCROLL_DEF.map(c => c.key),
    vis,
    showFullUrl: false,
    ellipsize: "middle",
  };
}

function loadColPrefs() {
  const base = defaultColPrefs();
  const raw = safeGetLS("zniff_share_cols_v2");
  if (!raw) return base;
  try {
    const p = JSON.parse(raw);
    if (Array.isArray(p.fixedOrder)) {
      const valid = p.fixedOrder.filter(k => FIXED_DEF.some(c => c.key === k));
      FIXED_DEF.forEach(c => { if (!valid.includes(c.key)) valid.push(c.key); });
      base.fixedOrder = valid;
    }
    if (Array.isArray(p.scrollOrder)) {
      const valid = p.scrollOrder.filter(k => SCROLL_DEF.some(c => c.key === k));
      SCROLL_DEF.forEach(c => { if (!valid.includes(c.key)) valid.push(c.key); });
      base.scrollOrder = valid;
    }
    if (p.vis && typeof p.vis === "object") {
      Object.keys(base.vis).forEach(k => { if (typeof p.vis[k] === "boolean") base.vis[k] = p.vis[k]; });
    }
    if (typeof p.showFullUrl === "boolean") base.showFullUrl = p.showFullUrl;
    if (["head", "middle", "tail"].includes(p.ellipsize)) base.ellipsize = p.ellipsize;
  } catch (_) {}
  return base;
}

function parseInitialHashFlowId() {
  const m = String(location.hash || "").match(/^#flow=(\d+)$/);
  if (!m) return null;
  const id = Number(m[1]);
  const flows = (INITIAL && Array.isArray(INITIAL.flows)) ? INITIAL.flows : [];
  return flows.some(f => f.id === id) ? id : null;
}

const savedLang = safeGetLS("zniff_share_lang");
const savedTheme = safeGetLS("zniff_share_theme");

let state = {
  data: INITIAL || { flows: [] },
  ended: false,
  zh: savedLang === "zh" ? true : (savedLang === "en" ? false : DEFAULT_ZH),
  themePref: ["system", "dark", "light"].includes(savedTheme) ? savedTheme : "system",
  cols: loadColPrefs(),
  filter: "",
  listFilter: "",
  facetOpen: false,
  sortCol: null,
  sortDir: 0,
  selectedId: parseInitialHashFlowId(),
  baselineId: null,
  localPins: new Map(),
  pretty: true,
  folded: {},
  reqBodySearch: "",
  resBodySearch: "",
  reqPreview: false,
  resPreview: false,
  reqExpanded: false,
  resExpanded: false,
  diffChangesOnly: false,
  msgDir: "all",
  msgKind: "all",
  msgSearch: "",
  hexCols: 16,
  seqOpen: false,
  seqTab: "waterfall",
  seqMermaidKind: "sequence",
  seqIncludeRes: true,
  seqIncludeQuery: false,
  seqChecked: new Set(),
};

function tr(zh, en) { return state.zh ? zh : en; }

function applyTheme() {
  let eff = state.themePref;
  if (eff === "system") {
    eff = (window.matchMedia && window.matchMedia("(prefers-color-scheme: light)").matches) ? "light" : "dark";
  }
  document.documentElement.setAttribute("data-theme", eff);
  document.documentElement.setAttribute("lang", state.zh ? "zh-CN" : "en");
}
applyTheme();
if (window.matchMedia) {
  const mq = window.matchMedia("(prefers-color-scheme: light)");
  if (mq.addEventListener) mq.addEventListener("change", () => { if (state.themePref === "system") applyTheme(); });
}

function esc(text) {
  return String(text ?? "").replace(/[&<>"']/g, ch => ({"&":"&amp;","<":"&lt;",">":"&gt;","\"":"&quot;","'":"&#39;"}[ch]));
}

function showToast(msg) {
  const el = document.getElementById("toast");
  if (!el) return;
  el.textContent = msg;
  el.classList.add("show");
  clearTimeout(showToast._t);
  showToast._t = setTimeout(() => el.classList.remove("show"), 1400);
}

async function copyRawText(text, btn) {
  try {
    await navigator.clipboard.writeText(String(text ?? ""));
    if (btn) {
      const prev = btn.textContent;
      btn.textContent = tr("已复制", "Copied");
      setTimeout(() => { btn.textContent = prev; }, 900);
    } else {
      showToast(tr("已复制到剪贴板", "Copied to clipboard"));
    }
  } catch (_) {
    showToast(tr("复制失败", "Copy failed"));
  }
}

function localizeError(err) {
  const s = String(err || "");
  if (!s) return "";
  const map = [
    ["断点丢弃", "Dropped at breakpoint"],
    ["连接被拒绝", "Connection refused"],
    ["连接超时", "Connection timed out"],
    ["DNS 解析失败", "DNS lookup failed"],
    ["TLS 握手失败", "TLS handshake failed"],
    ["客户端断开", "Client disconnected"],
    ["源站关闭连接", "Origin closed connection"],
  ];
  for (const [zh, en] of map) {
    if (state.zh && s.includes(en)) return s.replace(en, zh);
    if (!state.zh && s.includes(zh)) return s.replace(zh, en);
  }
  return s;
}

function formatBytes(b) {
  const n = Number(b || 0);
  if (n < 1024) return n + " B";
  if (n < 1024 * 1024) return (n / 1024).toFixed(1) + " KB";
  return (n / (1024 * 1024)).toFixed(2) + " MB";
}

function formatGapMs(ms) {
  if (!Number.isFinite(ms) || ms <= 0) return "0 ms";
  if (ms < 1000) return Math.round(ms) + " ms";
  if (ms < 60000) return (ms / 1000).toFixed(2) + " s";
  return (ms / 60000).toFixed(1) + " m";
}

function ellipsizeText(text, maxLen, mode) {
  const s = String(text || "");
  if (s.length <= maxLen) return s;
  if (mode === "head") return "…" + s.slice(-(maxLen - 1));
  if (mode === "tail") return s.slice(0, maxLen - 1) + "…";
  const half = Math.max(4, Math.floor((maxLen - 1) / 2));
  return s.slice(0, half) + "…" + s.slice(-(maxLen - 1 - half));
}

function isPinned(flow) {
  if (state.localPins.has(flow.id)) return state.localPins.get(flow.id);
  return !!flow.pinned;
}

function togglePin(flow) {
  state.localPins.set(flow.id, !isPinned(flow));
  renderAll();
}

function isSlow(flow) {
  return !!flow.slow || (flow.total_ms != null && Number(flow.total_ms) >= 1000);
}

function relayShort(src) {
  return String(src || "").replace(/[^0-9a-fA-F]/g, "").slice(0, 8);
}

function flowClientIp(flow) {
  if (flow.client_ip) return flow.client_ip;
  if (flow.relay_source) return (state.zh ? "中继 " : "Relay ") + relayShort(flow.relay_source);
  return "";
}

function extractAppFacet(flow) {
  const raw = flowClientIp(flow).trim();
  if (!raw || raw === "—") return null;
  let label = raw;
  if (raw.endsWith(")")) {
    const idx = raw.lastIndexOf("(");
    if (idx >= 0) {
      const inner = raw.slice(idx + 1, -1).trim();
      if (inner) label = inner;
    }
  }
  const token = label.split(/\s+/).filter(Boolean).join("_");
  return token ? { label, token } : null;
}

function statusClass(flow) {
  const failed = !!flow.error && flow.status == null;
  if (failed || (flow.status != null && flow.status >= 400)) return "s-err";
  if (flow.status != null && flow.status >= 300) return "s-warn";
  if (flow.status != null && flow.status >= 100) return "s-ok";
  return "muted-cell";
}

function statusBadgeClass(flow) {
  const failed = !!flow.error && flow.status == null;
  if (failed || (flow.status != null && flow.status >= 400)) return "s-badge err";
  if (flow.status != null && flow.status >= 300) return "s-badge warn";
  if (flow.status != null && flow.status >= 200) return "s-badge ok";
  if (flow.status != null && flow.status >= 100) return "s-badge info";
  return "proto-badge";
}

function extractGraphqlOps(flow) {
  const ops = [];
  if (flow.graphql_op) {
    String(flow.graphql_op).split(",").map(s => s.trim().toLowerCase()).filter(Boolean).forEach(op => {
      if (!ops.includes(op)) ops.push(op);
    });
  }
  const addFromQuery = (q) => {
    const cleaned = String(q || "").replace(/#[^\r\n]*/g, " ").trim();
    if (!cleaned) return;
    const m = cleaned.match(/^(?:query|mutation|subscription)\b\s*([_A-Za-z][_0-9A-Za-z]*)?/);
    if (m) {
      const name = (m[1] || "anonymous").toLowerCase();
      if (!ops.includes(name)) ops.push(name);
      return;
    }
    if (cleaned.startsWith("{") && !ops.includes("anonymous")) ops.push("anonymous");
  };
  const addFromObj = (obj) => {
    if (!obj || typeof obj !== "object") return;
    if (typeof obj.operationName === "string" && obj.operationName.trim()) {
      const name = obj.operationName.trim().toLowerCase();
      if (!ops.includes(name)) ops.push(name);
      return;
    }
    if (typeof obj.query === "string") addFromQuery(obj.query);
  };
  const raw = (flow.request_body || "").trim();
  if (raw.startsWith("{") || raw.startsWith("[")) {
    try {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) parsed.forEach(addFromObj);
      else addFromObj(parsed);
    } catch (_) {
      if (raw.startsWith("{")) addFromQuery(raw);
    }
  } else if (raw && (flow.request_body_kind === "graphql" || String(flow.path || "").toLowerCase().includes("graphql"))) {
    addFromQuery(raw);
  }
  if (!ops.length && Array.isArray(flow.query)) {
    const opPair = flow.query.find(p => String(p[0]).toLowerCase() === "operationname");
    const qPair = flow.query.find(p => String(p[0]).toLowerCase() === "query");
    if (opPair && String(opPair[1]).trim()) ops.push(String(opPair[1]).trim().toLowerCase());
    else if (qPair) addFromQuery(qPair[1]);
  }
  return ops;
}

function graphqlDisplayLabel(flow) {
  if (flow.graphql_op) return flow.graphql_op;
  const ops = extractGraphqlOps(flow);
  return ops.length ? ops.join(", ") : "";
}

function matchWildcard(pat, val) {
  if (!pat.includes("*")) return val === pat;
  const parts = pat.split("*");
  let pos = 0;
  for (let i = 0; i < parts.length; i++) {
    const seg = parts[i];
    if (!seg) continue;
    const idx = val.indexOf(seg, pos);
    if (idx < 0) return false;
    if (i === 0 && !pat.startsWith("*") && idx !== 0) return false;
    pos = idx + seg.length;
  }
  return pat.endsWith("*") || val.endsWith(parts[parts.length - 1]);
}

function matchHostTerm(host, pat) {
  if (!pat) return true;
  if (pat.startsWith("*.")) {
    const bare = pat.slice(2);
    return host === bare || host.endsWith("." + bare);
  }
  if (pat.includes("*")) return matchWildcard(pat, host);
  return host.includes(pat);
}

function evalSingleToken(flow, rawTok, ctx) {
  const { host, path, comment, method, failed, ct, clientIp, gqlLabel } = ctx;
  let neg = false;
  let tok = rawTok;
  if (tok.startsWith("-") && tok.length > 1) {
    neg = true;
    tok = tok.slice(1);
  }
  const apply = (matched) => neg ? !matched : matched;

  if (tok === "pin:") return apply(isPinned(flow));
  if (tok === "slow:") return apply(isSlow(flow));
  if (tok.startsWith("mute:")) {
    const ex = tok.slice(5).split(/[|,]/).filter(Boolean);
    if (!ex.length) return true;
    const hit = ex.some(p => matchHostTerm(host, p));
    return neg ? hit : !hit;
  }
  if (tok.startsWith("host:") || tok.startsWith("domain:")) {
    const terms = tok.replace(/^(host|domain):/, "").split(/[|,]/).filter(Boolean);
    if (!terms.length) return true;
    return apply(terms.some(p => matchHostTerm(host, p)));
  }
  if (tok.startsWith("path:")) {
    const terms = tok.slice(5).split(/[|,]/).filter(Boolean);
    if (!terms.length) return true;
    return apply(terms.some(p => path.includes(p) || path.startsWith(p) || path.replace(/^\//, "").startsWith(p)));
  }
  if (tok.startsWith("ip:") || tok.startsWith("client:") || tok.startsWith("app:") || tok.startsWith("proc:")) {
    const terms = tok.replace(/^(ip|client|app|proc):/, "").split(/[|,]/).filter(Boolean);
    if (!terms.length) return apply(!!clientIp);
    const normIp = clientIp.replace(/\s+/g, "_");
    return apply(terms.some(p => clientIp.includes(p) || normIp.includes(p) || clientIp.includes(p.replace(/_/g, " "))));
  }
  if (tok.startsWith("method:")) {
    const terms = tok.slice(7).split(/[|,]/).filter(Boolean);
    if (!terms.length) return true;
    return apply(terms.some(p => p.toUpperCase() === method));
  }
  if (tok.startsWith("status:")) {
    const terms = tok.slice(7).split(/[|,]/).filter(Boolean);
    if (!terms.length) return true;
    return apply(terms.some(p => {
      if (["fail", "failed", "err", "error", "失败"].includes(p)) return failed;
      if (/^[1-5]xx$/.test(p)) return flow.status != null && Math.floor(flow.status / 100) === Number(p[0]);
      return /^\d{3}$/.test(p) && flow.status === Number(p);
    }));
  }
  if (tok.startsWith("type:") || tok.startsWith("ct:")) {
    const terms = tok.replace(/^(type|ct):/, "").split(/[|,]/).filter(Boolean);
    if (!terms.length) return true;
    const proto = String(flow.protocol || "").toLowerCase();
    const kinds = (flow.request_body_kind || "") + " " + (flow.response_body_kind || "");
    return apply(terms.some(k => {
      if (k === "ws" || k === "websocket") return proto.includes("websocket");
      if (k === "gql" || k === "graphql") return kinds.includes("graphql") || extractGraphqlOps(flow).length > 0;
      return ct.includes(k) || kinds.includes(k) || proto.includes(k);
    }));
  }
  if (tok.startsWith("header:")) {
    const terms = tok.slice(7).split(/[|,]/).filter(Boolean);
    if (!terms.length) return true;
    const allH = (flow.request_headers || []).concat(flow.response_headers || []);
    return apply(terms.some(k => {
      const col = k.indexOf(":");
      return allH.some(h => {
        const hn = String(h[0] || "").toLowerCase();
        const hv = String(h[1] || "").toLowerCase();
        if (hn.includes(k) || hv.includes(k) || (hn + ": " + hv).includes(k)) return true;
        return col > 0 && hn.includes(k.slice(0, col)) && hv.includes(k.slice(col + 1));
      });
    }));
  }
  if (tok.startsWith("note:") || tok.startsWith("comment:")) {
    const val = tok.replace(/^(note|comment):/, "");
    if (!val) return apply(comment.length > 0);
    const terms = val.split(/[|,]/).filter(Boolean);
    return apply(terms.some(k => comment.includes(k)));
  }
  if (tok.startsWith("repeat:")) {
    const val = tok.slice(7).trim();
    if (!val) return apply(comment.includes("repeat:"));
    const terms = val.split(/[|,]/).filter(Boolean);
    return apply(terms.some(k => comment.includes("repeat:" + k) || String(flow.id) === k));
  }
  if (tok.startsWith("op:") || tok.startsWith("gql:") || tok.startsWith("operation:")) {
    const val = tok.replace(/^(op|gql|operation):/, "");
    const ops = extractGraphqlOps(flow);
    if (!val) return apply(ops.length > 0);
    const terms = val.split(/[|,]/).filter(Boolean);
    return apply(terms.some(pat => ops.some(op => matchWildcard(pat, op) || (!pat.includes("*") && op.includes(pat)))));
  }
  if (tok.startsWith("body:")) {
    const terms = tok.slice(5).split(/[|,]/).filter(Boolean);
    if (!terms.length) return apply(!!(flow.request_body || flow.response_body));
    const reqB = ((flow.request_body || "") + "\n" + (flow.request_body_pretty || "")).toLowerCase();
    const resB = ((flow.response_body || "") + "\n" + (flow.response_body_pretty || "")).toLowerCase();
    const msgs = (flow.messages || []).map(m => String(m).toLowerCase())
      .concat((flow.message_items || []).map(m => String(m.text || "").toLowerCase()));
    return apply(terms.some(k => reqB.includes(k) || resB.includes(k) || msgs.some(m => m.includes(k))));
  }

  const ors = tok.split(/[|,]/).filter(Boolean);
  if (!ors.length) return true;
  if (ors.every(p => ["get","post","put","patch","delete","head","options","connect"].includes(p))) {
    return apply(ors.some(p => p.toUpperCase() === method));
  }
  if (ors.every(p => ["fail","failed","err","error","失败"].includes(p) || /^[1-5]xx$/.test(p) || /^\d{3}$/.test(p))) {
    return apply(ors.some(p => {
      if (["fail","failed","err","error","失败"].includes(p)) return failed || (flow.status != null && flow.status >= 400);
      if (/^[1-5]xx$/.test(p)) return flow.status != null && Math.floor(flow.status / 100) === Number(p[0]);
      return flow.status === Number(p);
    }));
  }
  return apply(ors.some(p => host.includes(p) || path.includes(p) || comment.includes(p) || clientIp.includes(p) || gqlLabel.includes(p)));
}

function matchesFlow(flow, rawFilter) {
  const trimmed = String(rawFilter || "").trim().toLowerCase();
  if (!trimmed) return true;
  const tokens = trimmed.replace(/\s*\|\s*/g, "|").split(/\s+/).filter(Boolean);
  const ctx = {
    host: String(flow.host || "").toLowerCase(),
    path: String(flow.path || "").toLowerCase(),
    comment: String(flow.comment || "").toLowerCase(),
    method: String(flow.method || "").toUpperCase(),
    failed: !!flow.error && flow.status == null,
    ct: (flow.response_headers || []).concat(flow.request_headers || []).filter(h => String(h[0]).toLowerCase() === "content-type").map(h => h[1]).join(" ").toLowerCase(),
    clientIp: flowClientIp(flow).toLowerCase(),
    gqlLabel: String(flow.graphql_op || "").toLowerCase(),
  };
  for (const tok of tokens) {
    if (!evalSingleToken(flow, tok, ctx)) return false;
  }
  return true;
}

function hasToken(filter, token) {
  return String(filter || "").toLowerCase().trim().split(/\s+/).includes(token.toLowerCase());
}

function setMainFilter(next) {
  state.filter = String(next || "").trim();
  const el = document.getElementById("filter-input");
  if (el && el.value !== state.filter) el.value = state.filter;
  renderAll();
}

function toggleToken(token) {
  const parts = state.filter.trim().split(/\s+/).filter(Boolean);
  const idx = parts.findIndex(p => p.toLowerCase() === token.toLowerCase());
  if (idx >= 0) parts.splice(idx, 1);
  else parts.push(token);
  setMainFilter(parts.join(" "));
}

function mergeSecondaryFilter() {
  const sec = state.listFilter.trim();
  if (!sec) return;
  const mainParts = state.filter.trim().split(/\s+/).filter(Boolean);
  const seen = new Set(mainParts.map(t => t.toLowerCase()));
  sec.split(/\s+/).filter(Boolean).forEach(t => {
    if (!seen.has(t.toLowerCase())) {
      seen.add(t.toLowerCase());
      mainParts.push(t);
    }
  });
  state.listFilter = "";
  const subEl = document.getElementById("sub-filter-input");
  if (subEl) subEl.value = "";
  setMainFilter(mainParts.join(" "));
}

function togglePrefixOrToken(prefix, value) {
  const lowPre = prefix.toLowerCase();
  const lowVal = value.toLowerCase();
  const parts = state.filter.trim().split(/\s+/).filter(Boolean);
  const out = [];
  let foundGroup = false;
  let removed = false;
  for (const p of parts) {
    if (p.toLowerCase().startsWith(lowPre)) {
      foundGroup = true;
      const items = p.slice(prefix.length).split("|").filter(Boolean);
      const idx = items.findIndex(x => x.toLowerCase() === lowVal);
      if (idx >= 0) {
        items.splice(idx, 1);
        removed = true;
      } else {
        items.push(value);
      }
      if (items.length) out.push(prefix + items.join("|"));
    } else {
      out.push(p);
    }
  }
  if (!foundGroup && !removed) out.push(prefix + value);
  setMainFilter(out.join(" "));
}

function onlyHostFilter(host) {
  if (!host) return;
  const parts = state.filter.trim().split(/\s+/).filter(p => p && !/^(host|domain|mute):/i.test(p));
  parts.unshift("host:" + host);
  setMainFilter(parts.join(" "));
}

function shellQuote(s) { return "'" + String(s ?? "").replace(/'/g, "'\\''") + "'"; }

function buildCurl(flow) {
  const parts = ["curl -X " + shellQuote(flow.method || "GET") + " " + shellQuote(flow.url || "")];
  for (const [k, v] of (flow.request_headers || [])) {
    if (/^(host|content-length)$/i.test(k)) continue;
    parts.push("-H " + shellQuote(k + ": " + v));
  }
  if (flow.request_body && !flow.request_body.startsWith("[")) {
    parts.push("--data-raw " + shellQuote(flow.request_body));
  }
  return parts.join(" \\\n  ");
}

function buildFetch(flow) {
  const hdrs = {};
  for (const [k, v] of (flow.request_headers || [])) {
    if (/^(host|content-length|connection)$/i.test(k)) continue;
    hdrs[k] = v;
  }
  const opts = { method: flow.method || "GET" };
  if (Object.keys(hdrs).length) opts.headers = hdrs;
  if (flow.request_body && !["GET", "HEAD"].includes((flow.method || "").toUpperCase())) {
    opts.body = flow.request_body;
  }
  return `const res = await fetch(${JSON.stringify(flow.url || "")}, ${JSON.stringify(opts, null, 2)});\nconst text = await res.text();`;
}

function buildPythonRequests(flow) {
  const hdrs = (flow.request_headers || []).filter(([k]) => !/^(host|content-length)$/i.test(k));
  const hLines = hdrs.map(([k, v]) => `    ${JSON.stringify(k)}: ${JSON.stringify(v)},`).join("\n");
  let call = `resp = requests.request(\n    ${JSON.stringify(flow.method || "GET")},\n    ${JSON.stringify(flow.url || "")},`;
  if (hdrs.length) call += `\n    headers={\n${hLines}\n    },`;
  if (flow.request_body) call += `\n    data=${JSON.stringify(flow.request_body)},`;
  call += "\n)\nprint(resp.status_code, resp.text)";
  return "import requests\n\n" + call;
}

function buildRustReqwest(flow) {
  const m = (flow.method || "GET").toUpperCase();
  const lines = [
    "let client = reqwest::Client::new();",
    `let resp = client.request(reqwest::Method::from_bytes(b${JSON.stringify(m)})?, ${JSON.stringify(flow.url || "")})`,
  ];
  for (const [k, v] of (flow.request_headers || [])) {
    if (/^(host|content-length)$/i.test(k)) continue;
    lines.push(`    .header(${JSON.stringify(k)}, ${JSON.stringify(v)})`);
  }
  if (flow.request_body) lines.push(`    .body(${JSON.stringify(flow.request_body)})`);
  lines.push("    .send()", "    .await?;");
  return lines.join("\n");
}

function buildGoHttp(flow) {
  const hasBody = !!flow.request_body;
  const bodyExpr = hasBody ? `strings.NewReader(${JSON.stringify(flow.request_body)})` : "nil";
  const lines = [
    `req, err := http.NewRequest(${JSON.stringify(flow.method || "GET")}, ${JSON.stringify(flow.url || "")}, ${bodyExpr})`,
    "if err != nil { panic(err) }",
  ];
  for (const [k, v] of (flow.request_headers || [])) {
    if (/^(host|content-length)$/i.test(k)) continue;
    lines.push(`req.Header.Set(${JSON.stringify(k)}, ${JSON.stringify(v)})`);
  }
  lines.push("resp, err := http.DefaultClient.Do(req)");
  return lines.join("\n");
}

function sampleJsonForTypes(flow) {
  for (const candidate of [flow.response_body, flow.response_body_pretty, flow.request_body]) {
    if (!candidate) continue;
    try {
      const v = JSON.parse(candidate);
      if (v && typeof v === "object") return Array.isArray(v) ? (v[0] && typeof v[0] === "object" ? v[0] : null) : v;
    } catch (_) {}
  }
  return null;
}

function tsTypeOf(v) {
  if (v === null) return "null";
  if (Array.isArray(v)) return v.length ? `${tsTypeOf(v[0])}[]` : "unknown[]";
  if (typeof v === "object") return "Record<string, unknown>";
  return typeof v;
}

function buildTsInterface(flow) {
  const obj = sampleJsonForTypes(flow);
  if (!obj) return "// No JSON object body";
  const fields = Object.entries(obj).map(([k, v]) => {
    const safeK = /^[$_a-zA-Z][$_a-zA-Z0-9]*$/.test(k) ? k : JSON.stringify(k);
    return `  ${safeK}: ${tsTypeOf(v)};`;
  });
  return `export interface RootPayload {\n${fields.join("\n")}\n}`;
}

function rustTypeOf(v) {
  if (v === null) return "Option<serde_json::Value>";
  if (typeof v === "boolean") return "bool";
  if (typeof v === "number") return Number.isInteger(v) ? "i64" : "f64";
  if (typeof v === "string") return "String";
  if (Array.isArray(v)) return v.length ? `Vec<${rustTypeOf(v[0])}>` : "Vec<serde_json::Value>";
  return "serde_json::Value";
}

const RUST_KEYWORDS = new Set([
  "as", "async", "await", "box", "break", "const", "continue", "crate", "dyn", "else",
  "enum", "extern", "false", "fn", "for", "gen", "if", "impl", "in", "let", "loop", "macro",
  "match", "mod", "move", "mut", "priv", "pub", "ref", "return", "self", "Self", "static",
  "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "yield"
]);

function rustFieldIdent(key) {
  let snake = "";
  let prevLowerOrDigit = false;
  for (const ch of String(key)) {
    if (/^[a-zA-Z0-9]$/.test(ch)) {
      if (ch >= "A" && ch <= "Z") {
        if (prevLowerOrDigit && !snake.endsWith("_")) snake += "_";
        snake += ch.toLowerCase();
        prevLowerOrDigit = false;
      } else {
        snake += ch;
        prevLowerOrDigit = true;
      }
    } else {
      if (snake && !snake.endsWith("_")) snake += "_";
      prevLowerOrDigit = false;
    }
  }
  const clean = snake.replace(/^_+|_+$/g, "");
  let ident = !clean ? "field" : (/^\d/.test(clean) ? `f_${clean}` : clean);
  let renameNeeded = ident !== key;
  if (RUST_KEYWORDS.has(ident)) {
    if (ident === "self" || ident === "Self" || ident === "super" || ident === "crate") {
      return { ident: ident + "_", renameNeeded: true };
    }
    ident = `r#${ident}`;
  }
  return { ident, renameNeeded };
}

function buildRustStruct(flow) {
  const obj = sampleJsonForTypes(flow);
  if (!obj) return "// No JSON object body";
  const lines = [
    "use serde::{Deserialize, Serialize};",
    "",
    "#[derive(Debug, Clone, Serialize, Deserialize)]",
    "pub struct RootPayload {"
  ];
  const seen = new Set();
  for (const [k, v] of Object.entries(obj)) {
    let { ident, renameNeeded } = rustFieldIdent(k);
    const base = ident.replace(/^r#/, "");
    let n = 2;
    while (seen.has(ident)) {
      ident = `${base}_${n++}`;
      renameNeeded = true;
    }
    seen.add(ident);
    if (renameNeeded) lines.push(`    #[serde(rename = ${JSON.stringify(k)})]`);
    lines.push(`    pub ${ident}: ${rustTypeOf(v)},`);
  }
  lines.push("}");
  return lines.join("\n");
}

function decodeBase64Utf8(b64) {
  const norm = b64.replace(/-/g, "+").replace(/_/g, "/");
  const pad = norm + "=".repeat((4 - (norm.length % 4)) % 4);
  const bin = atob(pad);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
}

function annotateValue(rawVal) {
  const s = String(rawVal ?? "").trim();
  if (!s || s.includes("[REDACTED]")) return "";
  const tokenCandidate = s.replace(/^(Bearer|JWT)\s+/i, "").trim();
  const jwtParts = tokenCandidate.split(".");
  if (jwtParts.length === 3 && jwtParts[0].startsWith("eyJ") && jwtParts[1].length >= 4) {
    try {
      const hdr = JSON.parse(decodeBase64Utf8(jwtParts[0]));
      const pay = JSON.parse(decodeBase64Utf8(jwtParts[1]));
      let expNote = "";
      if (typeof pay.exp === "number") {
        expNote = ` · exp ${new Date(pay.exp * 1000).toISOString()}`;
      }
      return `JWT (${hdr.alg || "?"})${expNote}\n` + JSON.stringify(pay);
    } catch (_) {}
  }
  if (/^\d{10}$/.test(s)) {
    const sec = Number(s);
    if (sec >= 946684800 && sec <= 4102444800) return `Unix s → ${new Date(sec * 1000).toISOString()}`;
  }
  if (/^\d{13}$/.test(s)) {
    const ms = Number(s);
    if (ms >= 946684800000 && ms <= 4102444800000) return `Unix ms → ${new Date(ms).toISOString()}`;
  }
  if (s.length >= 16 && s.length <= 512 && s.length % 4 === 0 && /^[A-Za-z0-9+/]+={0,2}$/.test(s)) {
    try {
      const dec = decodeBase64Utf8(s);
      if (dec && dec.length >= 8 && /^[\x09\x0A\x0D\x20-\x7E\u00A0-\uFFFF]+$/.test(dec)) {
        return "Base64 → " + (dec.length > 180 ? dec.slice(0, 180) + "…" : dec);
      }
    } catch (_) {}
  }
  return "";
}
function closeAllMenus() {
  document.querySelectorAll(".pop-menu.open").forEach(el => el.classList.remove("open"));
}

function positionMenu(menuEl, x, y) {
  menuEl.style.left = "0px";
  menuEl.style.top = "0px";
  menuEl.classList.add("open");
  const rect = menuEl.getBoundingClientRect();
  const maxX = Math.max(8, window.innerWidth - rect.width - 8);
  const maxY = Math.max(8, window.innerHeight - rect.height - 8);
  menuEl.style.left = Math.max(8, Math.min(x, maxX)) + "px";
  menuEl.style.top = Math.max(8, Math.min(y, maxY)) + "px";
}

function selectFlowById(id) {
  state.selectedId = id;
  try {
    if (id == null) {
      if (location.hash.startsWith("#flow=")) history.replaceState(null, "", location.pathname + location.search);
    } else {
      history.replaceState(null, "", "#flow=" + id);
    }
  } catch (_) {}
  renderAll();
}

function countActiveFacets() {
  const parts = state.filter.trim().split(/\s+/).filter(Boolean);
  let n = 0;
  for (const p of parts) {
    const low = p.toLowerCase();
    if (/^(host|domain|mute|app|client|ip|proc):/.test(low)) {
      const val = p.slice(p.indexOf(":") + 1);
      n += val.split(/[|,]/).filter(Boolean).length || 1;
    }
  }
  return n;
}

function renderToolbar(filteredCount, totalCount, allFlows) {
  const dot = document.getElementById("status-dot");
  const meta = document.getElementById("meta");
  const countPill = document.getElementById("count-pill");
  const summaryBtn = document.getElementById("summary-btn");
  const seqTopBtn = document.getElementById("seq-top-btn");
  const prettyBtn = document.getElementById("pretty-btn");
  const themeSeg = document.getElementById("theme-seg");
  const langSeg = document.getElementById("lang-seg");

  summaryBtn.textContent = tr("统计", "Summary");
  summaryBtn.classList.toggle("active", state.selectedId == null && filteredCount > 0);
  seqTopBtn.textContent = tr("生成时序图", "Sequence");
  prettyBtn.textContent = state.pretty ? tr("美化", "Pretty") : tr("原文", "Raw");
  prettyBtn.classList.toggle("active", state.pretty);

  const themes = [
    ["system", tr("系统", "Auto")],
    ["dark", tr("深色", "Dark")],
    ["light", tr("浅色", "Light")],
  ];
  themeSeg.innerHTML = themes.map(([k, lbl]) =>
    `<button class="seg-btn${state.themePref === k ? " active" : ""}" data-theme="${k}" type="button">${esc(lbl)}</button>`
  ).join("");
  themeSeg.querySelectorAll(".seg-btn").forEach(btn => {
    btn.onclick = () => {
      state.themePref = btn.dataset.theme;
      safeSetLS("zniff_share_theme", state.themePref);
      applyTheme();
      renderAll();
    };
  });

  langSeg.innerHTML = `<button class="seg-btn${state.zh ? " active" : ""}" data-lang="zh" type="button">中文</button>`
    + `<button class="seg-btn${!state.zh ? " active" : ""}" data-lang="en" type="button">EN</button>`;
  langSeg.querySelectorAll(".seg-btn").forEach(btn => {
    btn.onclick = () => {
      state.zh = btn.dataset.lang === "zh";
      safeSetLS("zniff_share_lang", state.zh ? "zh" : "en");
      applyTheme();
      renderAll();
    };
  });

  const filterInput = document.getElementById("filter-input");
  const filterClear = document.getElementById("filter-clear");
  const subInput = document.getElementById("sub-filter-input");
  const subClear = document.getElementById("sub-filter-clear");
  const mergeBtn = document.getElementById("merge-sub-btn");
  const facetBtn = document.getElementById("facet-btn");
  const colBtn = document.getElementById("col-btn");

  filterInput.placeholder = tr(
    "过滤域名、路径、状态码、方法、type:json、body:、header:、note:、op:、app:、-词…",
    "Filter host, path, status, method, type:json, body:, header:, note:, op:, app:, -term…"
  );
  subInput.placeholder = tr("在当前结果中再次过滤…", "Filter within current results…");
  mergeBtn.textContent = tr("并入主过滤", "Merge");
  const hasSub = !!state.listFilter.trim();
  mergeBtn.style.display = hasSub ? "inline-flex" : "none";
  subClear.style.display = hasSub ? "inline-flex" : "none";
  filterClear.style.display = state.filter ? "inline-flex" : "none";

  const activeFacets = countActiveFacets();
  facetBtn.innerHTML = esc(tr("导航", "Facets")) + (activeFacets ? ` <span class="count-badge mono">${activeFacets}</span>` : "");
  facetBtn.classList.toggle("active", state.facetOpen || activeFacets > 0);
  colBtn.textContent = tr("列设置", "Columns");

  document.getElementById("footer-left").textContent = tr(
    "↑ / ↓ 切换请求 · Esc 返回统计 · Cmd/Ctrl+F 搜索 · 右键行或表头打开菜单",
    "↑ / ↓ navigate · Esc summary · Cmd/Ctrl+F search · Right-click row or header"
  );

  if (state.ended || !state.data || !state.data.flows) {
    dot.className = "status-dot ended";
    meta.textContent = tr("分享已结束或已过期", "This share has ended or expired");
    countPill.textContent = "0";
    return;
  }
  dot.className = LIVE ? "status-dot" : "status-dot snapshot";
  const parts = [LIVE ? tr("实时分享", "Live share") : tr("快照归档", "Snapshot")];
  if (LIVE && state.data.expires) {
    const when = new Date(state.data.expires * 1000);
    parts.push(tr("有效期至 ", "Expires ") + when.toLocaleTimeString());
  }
  if (state.data.redacted) parts.push(tr("已脱敏", "Redacted"));
  if (state.data.filter) parts.push(tr("分享过滤: ", "Share filter: ") + state.data.filter);
  if (state.data.omitted) parts.push(tr("已省略较早记录 ", "Omitted ") + "(" + state.data.omitted + ")");
  meta.textContent = parts.join(" · ");

  const hasAnyFilter = !!(state.filter.trim() || state.listFilter.trim());
  countPill.textContent = hasAnyFilter ? `${filteredCount} / ${totalCount}` : String(totalCount);
  countPill.title = tr("点击查看当前列表统计", "Click to view current list summary");

  const chips = [
    { token: "type:json", label: "JSON" },
    { token: "失败", alt: "err", label: tr("失败", "Errors") },
    { token: "pin:", label: tr("星标", "Starred") },
    { token: "slow:", label: tr("慢请求", "Slow") },
  ];
  const chipsRow = document.getElementById("chips-row");
  chipsRow.innerHTML = chips.map(c => {
    const on = (hasToken(state.filter, c.token) || (c.alt && hasToken(state.filter, c.alt))) ? " active" : "";
    return `<button class="chip${on}" data-token="${esc(c.token)}" type="button"><span class="chip-dot"></span>${esc(c.label)}</button>`;
  }).join("");
  chipsRow.querySelectorAll(".chip").forEach(btn => {
    btn.onclick = () => toggleToken(btn.dataset.token);
  });

  renderFacetDrawer(allFlows);
}

function renderFacetDrawer(allFlows) {
  const drawer = document.getElementById("facet-drawer");
  drawer.classList.toggle("open", state.facetOpen);
  if (!state.facetOpen) return;

  const domainMap = new Map();
  const appMap = new Map();
  for (const f of allFlows) {
    const isErr = (!!f.error && f.status == null) || (f.status != null && f.status >= 400);
    const h = String(f.host || "").trim();
    if (h) {
      const k = h.toLowerCase();
      let e = domainMap.get(k);
      if (!e) { e = { label: h, token: h, count: 0, err: 0 }; domainMap.set(k, e); }
      e.count++;
      if (isErr) e.err++;
    }
    const app = extractAppFacet(f);
    if (app) {
      const k = app.token.toLowerCase();
      let e = appMap.get(k);
      if (!e) { e = { label: app.label, token: app.token, count: 0, err: 0 }; appMap.set(k, e); }
      e.count++;
      if (isErr) e.err++;
    }
  }
  const domains = Array.from(domainMap.values()).sort((a, b) => b.count - a.count || a.token.localeCompare(b.token)).slice(0, 24);
  const apps = Array.from(appMap.values()).sort((a, b) => b.count - a.count || a.token.localeCompare(b.token)).slice(0, 24);

  const fParts = state.filter.toLowerCase().trim().split(/\s+/).filter(Boolean);
  const isOrActive = (pre, val) => fParts.some(p => p.startsWith(pre) && p.slice(pre.length).split(/[|,]/).includes(val.toLowerCase()));

  let html = `<div><div class="facet-sec-head"><span>${esc(tr("域名", "Domains"))} (${domains.length})</span>`
    + (countActiveFacets() > 0 ? `<button class="mini-btn" id="clear-facets-btn" type="button" style="height:18px;font-size:10px">${esc(tr("清除导航筛选", "Clear"))}</button>` : "")
    + `</div><div class="facet-pills">`;
  if (!domains.length) {
    html += `<span class="muted-cell" style="font-size:11px">${esc(tr("暂无域名", "No domains"))}</span>`;
  } else {
    for (const d of domains) {
      const on = isOrActive("host:", d.token) || isOrActive("domain:", d.token);
      const muted = isOrActive("mute:", d.token);
      const cls = muted ? " muted" : (on ? " active" : "");
      html += `<span class="facet-pill${cls}">`
        + `<button class="facet-main mono" data-host="${esc(d.token)}" type="button" title="${esc(d.label)}">`
        + `<span class="facet-lbl">${esc(d.label)}</span>`
        + `<span class="facet-cnt">${d.count}</span>`
        + (d.err ? `<span class="facet-err">${d.err}!</span>` : "")
        + `</button>`
        + `<button class="facet-mute" data-mute="${esc(d.token)}" type="button" title="${esc(tr("屏蔽此域名 (mute:)", "Mute domain (mute:)"))}">×</button>`
        + `</span>`;
    }
  }
  html += `</div></div>`;

  html += `<div><div class="facet-sec-head"><span>${esc(tr("应用 / 来源", "Apps / Sources"))} (${apps.length})</span></div><div class="facet-pills">`;
  if (!apps.length) {
    html += `<span class="muted-cell" style="font-size:11px">${esc(tr("暂无来源进程信息", "No client attribution"))}</span>`;
  } else {
    for (const a of apps) {
      const on = isOrActive("app:", a.token) || isOrActive("client:", a.token);
      html += `<span class="facet-pill${on ? " active" : ""}">`
        + `<button class="facet-main mono" data-app="${esc(a.token)}" type="button">`
        + `<span class="facet-lbl">${esc(a.label)}</span>`
        + `<span class="facet-cnt">${a.count}</span>`
        + (a.err ? `<span class="facet-err">${a.err}!</span>` : "")
        + `</button></span>`;
    }
  }
  html += `</div></div>`;

  drawer.innerHTML = html;
  const clr = document.getElementById("clear-facets-btn");
  if (clr) {
    clr.onclick = () => {
      const rest = state.filter.trim().split(/\s+/).filter(p => p && !/^(host|domain|mute|app|client|ip|proc):/i.test(p));
      setMainFilter(rest.join(" "));
    };
  }
  drawer.querySelectorAll("[data-host]").forEach(btn => {
    btn.onclick = () => togglePrefixOrToken("host:", btn.dataset.host);
  });
  drawer.querySelectorAll("[data-mute]").forEach(btn => {
    btn.onclick = () => togglePrefixOrToken("mute:", btn.dataset.mute);
  });
  drawer.querySelectorAll("[data-app]").forEach(btn => {
    btn.onclick = () => togglePrefixOrToken("app:", btn.dataset.app);
  });
}

function saveColsAndRender() {
  safeSetLS("zniff_share_cols_v2", JSON.stringify(state.cols));
  renderAll();
}

function openColumnsMenu(x, y) {
  closeAllMenus();
  const menu = document.getElementById("col-menu");
  const renderContent = () => {
    const groupHtml = (title, arrName) => {
      const arr = state.cols[arrName];
      return `<div class="menu-sec">${esc(title)}</div>` + arr.map((k, idx) => {
        const def = ALL_COL_MAP[k];
        const checked = state.cols.vis[k] ? "checked" : "";
        return `<div class="col-row">`
          + `<label><input type="checkbox" data-col-vis="${k}" ${checked}> <span>${esc(tr(def.zh, def.en))}</span></label>`
          + `<span class="col-ord-btns">`
          + `<button class="col-ord-btn" data-grp="${arrName}" data-idx="${idx}" data-dir="-1" type="button" ${idx === 0 ? "disabled" : ""}>↑</button>`
          + `<button class="col-ord-btn" data-grp="${arrName}" data-idx="${idx}" data-dir="1" type="button" ${idx === arr.length - 1 ? "disabled" : ""}>↓</button>`
          + `</span></div>`;
      }).join("");
    };

    const ellModes = [
      ["head", tr("开头", "Head")],
      ["middle", tr("中间", "Middle")],
      ["tail", tr("结尾", "Tail")],
    ];

    menu.innerHTML = groupHtml(tr("固定列", "Pinned columns"), "fixedOrder")
      + `<div class="menu-sep"></div>`
      + groupHtml(tr("滚动列", "Scrollable columns"), "scrollOrder")
      + `<div class="menu-sep"></div>`
      + `<div class="menu-sec">${esc(tr("路径显示", "Path display"))}</div>`
      + `<button class="menu-item" id="toggle-full-url" type="button"><span>${esc(state.cols.showFullUrl ? tr("显示: 完整 URL", "Mode: Full URL") : tr("显示: 仅路径", "Mode: Path only"))}</span><span>⇄</span></button>`
      + `<div class="menu-sec">${esc(tr("省略位置", "Ellipsize"))}</div>`
      + `<div style="display:flex;gap:4px;padding:2px 6px 4px">`
      + ellModes.map(([m, lbl]) => `<button class="mini-btn${state.cols.ellipsize === m ? " active" : ""}" data-ell="${m}" type="button" style="flex:1;justify-content:center">${esc(lbl)}</button>`).join("")
      + `</div>`
      + `<div class="menu-sep"></div>`
      + `<button class="menu-item" id="reset-cols-btn" type="button"><span>${esc(tr("恢复默认列", "Reset columns"))}</span></button>`;

    menu.querySelectorAll("[data-col-vis]").forEach(cb => {
      cb.onchange = () => {
        const key = cb.dataset.colVis;
        const visCount = Object.values(state.cols.vis).filter(Boolean).length;
        if (!cb.checked && visCount <= 1) { cb.checked = true; return; }
        state.cols.vis[key] = cb.checked;
        saveColsAndRender();
      };
    });
    menu.querySelectorAll(".col-ord-btn").forEach(btn => {
      btn.onclick = (e) => {
        e.stopPropagation();
        const grp = state.cols[btn.dataset.grp];
        const i = Number(btn.dataset.idx);
        const j = i + Number(btn.dataset.dir);
        if (j < 0 || j >= grp.length) return;
        const tmp = grp[i]; grp[i] = grp[j]; grp[j] = tmp;
        saveColsAndRender();
        renderContent();
      };
    });
    menu.querySelector("#toggle-full-url").onclick = (e) => {
      e.stopPropagation();
      state.cols.showFullUrl = !state.cols.showFullUrl;
      saveColsAndRender();
      renderContent();
    };
    menu.querySelectorAll("[data-ell]").forEach(btn => {
      btn.onclick = (e) => {
        e.stopPropagation();
        state.cols.ellipsize = btn.dataset.ell;
        saveColsAndRender();
        renderContent();
      };
    });
    menu.querySelector("#reset-cols-btn").onclick = () => {
      state.cols = defaultColPrefs();
      saveColsAndRender();
      closeAllMenus();
    };
  };
  renderContent();
  positionMenu(menu, x, y);
}

function openRowContextMenu(flow, x, y) {
  closeAllMenus();
  const menu = document.getElementById("ctx-menu");
  const pinned = isPinned(flow);
  const isBase = state.baselineId === flow.id;
  menu.innerHTML = [
    `<button class="menu-item" data-act="url" type="button"><span>${esc(tr("复制 URL", "Copy URL"))}</span></button>`,
    `<button class="menu-item" data-act="curl" type="button"><span>${esc(tr("复制 cURL", "Copy cURL"))}</span></button>`,
    `<div class="menu-sep"></div>`,
    `<button class="menu-item" data-act="only-host" type="button" ${flow.host ? "" : "disabled"}><span>${esc(tr("只看此域名", "Only this host"))}</span><span class="muted-cell mono" style="font-size:10.5px">${esc(ellipsizeText(flow.host || "", 16, "middle"))}</span></button>`,
    `<button class="menu-item" data-act="mute-host" type="button" ${flow.host ? "" : "disabled"}><span>${esc(tr("屏蔽此域名", "Mute this host"))}</span></button>`,
    `<div class="menu-sep"></div>`,
    `<button class="menu-item" data-act="pin" type="button"><span>${esc(pinned ? tr("取消星标", "Unstar") : tr("星标", "Star"))}</span><span>${pinned ? "★" : "☆"}</span></button>`,
    `<button class="menu-item" data-act="base" type="button"><span>${esc(isBase ? tr("取消基准", "Clear baseline") : tr("设为基准", "Set as baseline"))}</span></button>`,
  ].join("");

  menu.querySelectorAll("[data-act]").forEach(btn => {
    btn.onclick = () => {
      closeAllMenus();
      const act = btn.dataset.act;
      if (act === "url") copyRawText(flow.url || flow.path || "");
      else if (act === "curl") copyRawText(buildCurl(flow));
      else if (act === "only-host") onlyHostFilter(flow.host);
      else if (act === "mute-host") togglePrefixOrToken("mute:", flow.host);
      else if (act === "pin") togglePin(flow);
      else if (act === "base") {
        state.baselineId = isBase ? null : flow.id;
        if (state.baselineId != null) ensureFlowBody(flow);
        renderAll();
      }
    };
  });
  positionMenu(menu, x, y);
}

function getVisibleColumnList(availW) {
  const fixed = state.cols.fixedOrder.filter(k => state.cols.vis[k]).map(k => ({ ...ALL_COL_MAP[k], fixed: true }));
  const scroll = state.cols.scrollOrder.filter(k => state.cols.vis[k]).map(k => ({ ...ALL_COL_MAP[k], fixed: false }));
  const all = fixed.concat(scroll);
  const nonFlexSum = all.filter(c => !c.flex).reduce((acc, c) => acc + c.width, 0);
  const flexCol = all.find(c => c.flex);
  if (flexCol) {
    const minFlex = flexCol.width || 220;
    flexCol.width = (availW && availW > nonFlexSum + minFlex) ? Math.floor(availW - nonFlexSum) : minFlex;
  }
  let leftAcc = 0;
  fixed.forEach((col, i) => {
    col.left = leftAcc;
    leftAcc += col.width;
    col.lastFixed = i === fixed.length - 1 && scroll.length > 0;
  });
  const totalW = all.reduce((acc, c) => acc + c.width, 0);
  return { cols: all, totalW };
}

function sortValue(flow, col) {
  switch (col) {
    case "id": return Number(flow.id || 0);
    case "method": return String(flow.method || "");
    case "path": return String(state.cols.showFullUrl ? (flow.url || flow.path) : (flow.path || flow.url) || "");
    case "status": return flow.status == null ? (flow.error ? 999 : 0) : Number(flow.status);
    case "gap": return Number(flow._dynGapMs || 0);
    case "elapsed": return flow.total_ms == null ? -1 : Number(flow.total_ms);
    case "size": return Number(flow.response_bytes || 0);
    case "protocol": return String(flow.protocol || "");
    case "host": return String(flow.host || "");
    case "client_ip": return flowClientIp(flow);
    default: return Number(flow.id || 0);
  }
}

function computeVisibleFlows(allFlows) {
  let list = allFlows.filter(f => matchesFlow(f, state.filter));
  if (state.listFilter.trim()) {
    list = list.filter(f => matchesFlow(f, state.listFilter));
  }
  for (let i = 0; i < list.length; i++) {
    const cur = list[i];
    if (i === 0) {
      cur._dynGapMs = 0;
      cur._dynGapStr = "0 ms";
    } else {
      const prev = list[i - 1];
      if (cur.started_ms && prev.started_ms) {
        const diff = Math.max(0, cur.started_ms - prev.started_ms);
        cur._dynGapMs = diff;
        cur._dynGapStr = formatGapMs(diff);
      } else {
        cur._dynGapMs = 0;
        cur._dynGapStr = cur.gap || "0 ms";
      }
    }
  }
  if (state.sortCol && state.sortDir !== 0) {
    const col = state.sortCol;
    const dir = state.sortDir;
    list = list.slice().sort((a, b) => {
      const va = sortValue(a, col);
      const vb = sortValue(b, col);
      if (typeof va === "number" && typeof vb === "number") return (va - vb) * dir || (a.id - b.id);
      return String(va).localeCompare(String(vb)) * dir || (a.id - b.id);
    });
  }
  return list;
}

function renderList(flows) {
  const tableEl = document.getElementById("flow-table");
  const colgroup = document.getElementById("table-colgroup");
  const thead = document.getElementById("table-head");
  const tbody = document.getElementById("table-body");
  const scrollWrap = document.getElementById("table-scroll");
  const leftPane = document.getElementById("left-pane");
  const availW = scrollWrap.clientWidth || (leftPane ? parseFloat(leftPane.style.width) || leftPane.clientWidth : 660) || 660;
  const { cols, totalW } = getVisibleColumnList(availW);

  tableEl.style.width = totalW <= availW ? "100%" : (totalW + "px");
  if (colgroup) {
    colgroup.innerHTML = cols.map(c => `<col style="width:${c.width}px">`).join("");
  }

  thead.innerHTML = `<tr>` + cols.map(c => {
    const arrow = state.sortCol === c.key ? (state.sortDir === 1 ? " ↑" : (state.sortDir === -1 ? " ↓" : "")) : "";
    const cls = [c.fixed ? "sticky-col" : "", c.lastFixed ? "sticky-edge" : "", c.num ? "num" : ""].filter(Boolean).join(" ");
    const st = `width:${c.width}px;min-width:${c.width}px;max-width:${c.width}px;` + (c.fixed ? `left:${c.left}px;` : "");
    return `<th class="${cls}" style="${st}" data-sort="${c.key}">${esc(tr(c.zh, c.en) + arrow)}</th>`;
  }).join("") + `</tr>`;

  thead.oncontextmenu = (e) => {
    e.preventDefault();
    openColumnsMenu(e.clientX, e.clientY);
  };
  thead.querySelectorAll("th[data-sort]").forEach(th => {
    th.onclick = () => {
      const k = th.dataset.sort;
      if (state.sortCol !== k) { state.sortCol = k; state.sortDir = 1; }
      else if (state.sortDir === 1) state.sortDir = -1;
      else { state.sortCol = null; state.sortDir = 0; }
      renderAll();
    };
  });

  if (!flows.length) {
    tbody.innerHTML = `<tr><td colspan="${cols.length}" style="height:180px;text-align:center;color:var(--fg-muted);border:none">${esc(tr("暂无匹配的抓包记录", "No matching flows"))}</td></tr>`;
    return;
  }

  const nearBottom = scrollWrap.scrollHeight - scrollWrap.scrollTop - scrollWrap.clientHeight < 36;
  const ellMode = state.cols.ellipsize;

  tbody.innerHTML = flows.map(flow => {
    const isSel = flow.id === state.selectedId;
    const isBase = flow.id === state.baselineId;
    const rowCls = [isSel ? "selected" : "", isBase ? "baseline" : ""].filter(Boolean).join(" ");
    const failed = !!flow.error && flow.status == null;
    const statusText = flow.status == null ? (failed ? "ERR" : "—") : String(flow.status);
    const sCls = statusClass(flow);
    const pinned = isPinned(flow);
    const slow = isSlow(flow);

    const cells = cols.map((c, idx) => {
      const baseCls = [
        idx === 0 ? "lead-cell" : "",
        c.fixed ? "sticky-col" : "",
        c.lastFixed ? "sticky-edge" : "",
        c.num ? "num" : "",
      ];
      const st = `width:${c.width}px;min-width:${c.width}px;max-width:${c.width}px;` + (c.fixed ? `left:${c.left}px;` : "");
      if (c.key === "id") {
        baseCls.push("mono", "muted-cell");
        return `<td class="${baseCls.join(" ")}" style="${st}">${flow.id}</td>`;
      }
      if (c.key === "method") {
        baseCls.push("method-cell");
        const pinHtml = pinned ? `<span class="pin-dot"></span>` : "";
        return `<td class="${baseCls.join(" ")}" style="${st}"><span class="method-wrap"><span class="base-bar"></span><span class="pin-slot">${pinHtml}</span><span class="m-badge mono m-${esc(flow.method)}">${esc(flow.method)}</span></span></td>`;
      }
      if (c.key === "path") {
        baseCls.push("mono", "path-cell");
        if (failed) baseCls.push("failed");
        const rawTarget = failed ? localizeError(flow.error || "ERR") : (state.cols.showFullUrl ? (flow.url || flow.path || "/") : (flow.path || flow.url || "/"));
        const maxPathChars = Math.max(24, Math.floor((c.width - 14) / 7));
        const shownTarget = ellipsizeText(rawTarget, maxPathChars, ellMode);
        const relayHtml = flow.relay_source ? `<span class="relay-tag mono">${esc(tr("中继 ", "Relay ") + relayShort(flow.relay_source))}</span>` : "";
        const noteHtml = flow.comment ? `<span class="row-note">#${esc(ellipsizeText(flow.comment, 22, "tail"))}</span>` : "";
        const fullHover = (flow.url || rawTarget) + (flow.comment ? ` · #${flow.comment}` : "");
        return `<td class="${baseCls.join(" ")}" style="${st}" title="${esc(fullHover)}">${relayHtml}${esc(shownTarget)}${noteHtml}</td>`;
      }
      if (c.key === "status") {
        baseCls.push("mono", sCls);
        return `<td class="${baseCls.join(" ")}" style="${st}">${esc(statusText)}</td>`;
      }
      if (c.key === "gap") {
        baseCls.push("mono", "muted-cell");
        return `<td class="${baseCls.join(" ")}" style="${st}" title="${esc(flow.started || "")}">${esc(flow._dynGapStr || flow.gap || "0 ms")}</td>`;
      }
      if (c.key === "elapsed") {
        baseCls.push("mono", slow ? "slow" : "muted-cell");
        return `<td class="${baseCls.join(" ")}" style="${st}">${esc(flow.elapsed || "—")}</td>`;
      }
      if (c.key === "size") {
        baseCls.push("mono", "muted-cell");
        return `<td class="${baseCls.join(" ")}" style="${st}">${esc(flow.size || "0 B")}</td>`;
      }
      if (c.key === "protocol") {
        baseCls.push("mono", "muted-cell");
        return `<td class="${baseCls.join(" ")}" style="${st}">${esc(flow.protocol || "HTTP/1.1")}</td>`;
      }
      if (c.key === "host") {
        baseCls.push("mono", "muted-cell");
        const h = flow.host || "—";
        return `<td class="${baseCls.join(" ")}" style="${st}" title="${esc(h)}">${esc(ellipsizeText(h, 24, ellMode))}</td>`;
      }
      if (c.key === "client_ip") {
        baseCls.push("mono", "muted-cell");
        const ip = flowClientIp(flow) || "—";
        return `<td class="${baseCls.join(" ")}" style="${st}" title="${esc(ip)}">${esc(ip)}</td>`;
      }
      return `<td style="${st}"></td>`;
    });

    return `<tr class="${rowCls}" data-id="${flow.id}">${cells.join("")}</tr>`;
  }).join("");

  tbody.querySelectorAll("tr[data-id]").forEach(trEl => {
    const id = Number(trEl.dataset.id);
    trEl.onclick = () => selectFlowById(id);
    trEl.oncontextmenu = (e) => {
      e.preventDefault();
      state.selectedId = id;
      renderAll();
      const picked = flows.find(f => f.id === id);
      if (picked) openRowContextMenu(picked, e.clientX, e.clientY);
    };
  });

  if (LIVE && nearBottom) {
    scrollWrap.scrollTop = scrollWrap.scrollHeight;
  }
}
function renderSummaryView(visibleFlows) {
  const pane = document.getElementById("right-pane");
  if (!visibleFlows.length) {
    pane.innerHTML = `<div class="empty-state">${esc(tr("暂无匹配的抓包记录", "No matching flows"))}</div>`;
    return;
  }
  let s1 = 0, s2 = 0, s3 = 0, s4 = 0, s5 = 0, sErr = 0, sPend = 0;
  let reqSum = 0, resSum = 0, elapsedSum = 0, elapsedCnt = 0;
  let slowest = null;

  for (const f of visibleFlows) {
    reqSum += Number(f.request_bytes || 0);
    resSum += Number(f.response_bytes || 0);
    const st = f.status;
    if (st != null && st >= 100 && st < 200) s1++;
    else if (st != null && st >= 200 && st < 300) s2++;
    else if (st != null && st >= 300 && st < 400) s3++;
    else if (st != null && st >= 400 && st < 500) s4++;
    else if (st != null && st >= 500 && st < 600) s5++;
    else if (f.error) sErr++;
    else if (st == null) sPend++;

    if (f.total_ms != null) {
      const ms = Number(f.total_ms);
      elapsedSum += ms;
      elapsedCnt++;
      if (!slowest || ms > slowest.ms) slowest = { flow: f, ms };
    }
  }
  const avgStr = elapsedCnt ? Math.round(elapsedSum / elapsedCnt) + " ms" : "—";
  const statusPills = [
    { tok: "1xx", lbl: `1xx · ${s1}`, cnt: s1, cls: "info" },
    { tok: "2xx", lbl: `2xx · ${s2}`, cnt: s2, cls: "ok" },
    { tok: "3xx", lbl: `3xx · ${s3}`, cnt: s3, cls: "warn" },
    { tok: "4xx", lbl: `4xx · ${s4}`, cnt: s4, cls: "err" },
    { tok: "5xx", lbl: `5xx · ${s5}`, cnt: s5, cls: "err" },
    { tok: "失败", lbl: `${tr("失败", "ERR")} · ${sErr}`, cnt: sErr, cls: "err" },
  ].filter(p => p.cnt > 0);

  let html = `<div class="card summary-card">`
    + `<div class="summary-top">`
    + `<div style="font-size:13px;font-weight:700">${esc(tr("当前列表统计", "Current list summary"))}</div>`
    + `<button class="mini-btn primary" id="open-seq-btn" type="button">${esc(tr("生成时序图", "Sequence diagram"))}</button>`
    + `</div>`
    + `<div class="stats-grid">`
    + `<div class="stat-tile"><div class="stat-lbl">${esc(tr("总请求", "Requests"))}</div><div class="stat-val mono">${visibleFlows.length}</div><div class="stat-sub">${sPend ? esc(tr(`进行中 ${sPend}`, `${sPend} pending`)) : esc(tr("已完成", "Completed"))}</div></div>`
    + `<div class="stat-tile"><div class="stat-lbl">${esc(tr("总流量", "Transfer"))}</div><div class="stat-val mono">${esc(formatBytes(reqSum + resSum))}</div><div class="stat-sub mono">↑ ${esc(formatBytes(reqSum))} · ↓ ${esc(formatBytes(resSum))}</div></div>`
    + `<div class="stat-tile"><div class="stat-lbl">${esc(tr("平均耗时", "Avg time"))}</div><div class="stat-val mono">${esc(avgStr)}</div><div class="stat-sub">${esc(tr(`${elapsedCnt} 条已结束`, `${elapsedCnt} finished`))}</div></div>`
    + `</div>`;

  if (statusPills.length) {
    html += `<div class="summary-badges">` + statusPills.map(p =>
      `<button class="s-badge ${p.cls} mono" data-st-tok="${esc(p.tok)}" type="button" style="border:none;cursor:pointer">${esc(p.lbl)}</button>`
    ).join("") + `</div>`;
  }

  if (slowest) {
    const sf = slowest.flow;
    const target = (sf.host || "") + (sf.path || "");
    html += `<div class="slowest-row" id="jump-slowest-row" title="${esc(sf.url || target)}">`
      + `<div style="display:flex;align-items:center;gap:6px;min-width:0">`
      + `<span class="muted-cell" style="font-size:11px;flex-shrink:0">${esc(tr("最慢请求", "Slowest"))}</span>`
      + `<span class="mono muted-cell">#${sf.id}</span>`
      + `<span class="m-badge mono m-${esc(sf.method)}">${esc(sf.method)}</span>`
      + `<span class="mono" style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(ellipsizeText(target || sf.url, 54, "middle"))}</span>`
      + `</div>`
      + `<span class="mono ${slowest.ms >= 1000 ? "slow" : "muted-cell"}" style="flex-shrink:0">${slowest.ms} ms →</span>`
      + `</div>`;
  }
  html += `</div>`
    + `<div class="empty-state" style="margin:12px auto">${esc(tr("从左侧列表点击一条请求查看请求头、正文、时序与消息", "Select a request on the left to inspect headers, body, timing, and frames"))}</div>`;

  pane.innerHTML = html;
  const seqBtn = document.getElementById("open-seq-btn");
  if (seqBtn) seqBtn.onclick = () => openSequenceModal(visibleFlows);
  pane.querySelectorAll("[data-st-tok]").forEach(btn => {
    btn.onclick = () => toggleToken(btn.dataset.stTok);
  });
  const jumpRow = document.getElementById("jump-slowest-row");
  if (jumpRow && slowest) jumpRow.onclick = () => selectFlowById(slowest.flow.id);
}

function mermaidSafeId(host, idx) { return "P" + idx + "_" + String(host || "origin").replace(/[^a-zA-Z0-9]/g, "_").slice(0, 18); }
function mermaidEsc(s) { return String(s || "").replace(/["#;\r\n]/g, " ").trim(); }

function detectDependencies(flows) {
  const capped = flows.slice(-200);
  const produced = new Map();
  const addToken = (val, flow, source) => {
    const s = String(val ?? "").trim();
    if (s.length < 4 || s.length > 256 || /^(true|false|null|undefined|\d{1,2}|\[REDACTED\])$/i.test(s)) return;
    if (!produced.has(s)) produced.set(s, { flow, source });
  };

  const edges = [];
  const seenEdge = new Set();

  for (const f of capped) {
    const checkConsume = (val, via) => {
      const s = String(val ?? "").trim();
      const hit = produced.get(s);
      if (hit && hit.flow.id !== f.id) {
        const key = `${hit.flow.id}->${f.id}:${via}`;
        if (!seenEdge.has(key)) {
          seenEdge.add(key);
          edges.push({ from: hit.flow, to: f, via, source: hit.source });
        }
      }
    };

    for (const [qk, qv] of (f.query || [])) checkConsume(qv, `query.${qk}`);
    for (const [hk, hv] of (f.request_headers || [])) {
      const low = hk.toLowerCase();
      if (low === "authorization") checkConsume(hv.replace(/^Bearer\s+/i, "").trim(), "Authorization");
      else if (low === "cookie") {
        hv.split(";").forEach(seg => {
          const eq = seg.indexOf("=");
          if (eq > 0) checkConsume(seg.slice(eq + 1).trim(), `Cookie.${seg.slice(0, eq).trim()}`);
        });
      } else if (/token|key|session|csrf|id/i.test(low)) {
        checkConsume(hv, hk);
      }
    }
    for (const [fk, fv] of (f.request_form_rows || [])) checkConsume(fv, `form.${fk}`);
    if (f.request_body && f.request_body.trim().startsWith("{")) {
      try {
        const obj = JSON.parse(f.request_body);
        const walkReq = (o, pre) => {
          if (!o || typeof o !== "object") return;
          for (const [k, v] of Object.entries(o)) {
            if (typeof v === "string" || typeof v === "number") checkConsume(v, pre ? `${pre}.${k}` : k);
            else if (v && typeof v === "object" && !Array.isArray(v)) walkReq(v, k);
          }
        };
        walkReq(obj, "");
      } catch (_) {}
    }

    for (const [hk, hv] of (f.response_headers || [])) {
      const low = hk.toLowerCase();
      if (low === "set-cookie") {
        const first = hv.split(";")[0] || "";
        const eq = first.indexOf("=");
        if (eq > 0) addToken(first.slice(eq + 1).trim(), f, `Set-Cookie.${first.slice(0, eq).trim()}`);
      } else if (low === "location") {
        addToken(hv, f, "Location");
      }
    }
    if (f.response_body && f.response_body.trim().startsWith("{")) {
      try {
        const obj = JSON.parse(f.response_body);
        const walkRes = (o, depth) => {
          if (!o || typeof o !== "object" || depth > 3) return;
          for (const [k, v] of Object.entries(o)) {
            if ((typeof v === "string" || typeof v === "number") && /(^id$|_id$|Id$|token|Token|code|cursor|session|key)/i.test(k)) {
              addToken(v, f, `json.${k}`);
            } else if (v && typeof v === "object") {
              if (Array.isArray(v)) { if (v[0]) walkRes(v[0], depth + 1); }
              else walkRes(v, depth + 1);
            }
          }
        };
        walkRes(obj, 0);
      } catch (_) {}
    }
  }
  return edges;
}

function buildMermaidSequence(flows, includeRes, includeQuery) {
  const lines = ["sequenceDiagram", "  autonumber", "  participant Client"];
  const hostIds = new Map();
  flows.forEach(f => {
    const h = f.host || "origin";
    if (!hostIds.has(h)) {
      const pid = mermaidSafeId(h, hostIds.size + 1);
      hostIds.set(h, pid);
      lines.push(`  participant ${pid} as ${mermaidEsc(h)}`);
    }
  });
  for (const f of flows) {
    const pid = hostIds.get(f.host || "origin");
    let p = f.path || "/";
    if (!includeQuery) p = p.split("?")[0] || "/";
    const gql = graphqlDisplayLabel(f);
    const reqLbl = `${f.method} ${mermaidEsc(ellipsizeText(p, 56, "middle"))}${gql ? " [" + mermaidEsc(gql) + "]" : ""}`;
    lines.push(`  Client->>${pid}: #${f.id} ${reqLbl}`);
    if (includeRes) {
      const st = f.status == null ? (f.error ? "ERR" : "...") : String(f.status);
      lines.push(`  ${pid}-->>Client: ${st} (${mermaidEsc(f.elapsed || "—")})`);
    }
  }
  return lines.join("\n");
}

function buildMermaidFlowchart(flows, edges) {
  const lines = ["flowchart TD"];
  for (const f of flows) {
    const p = (f.path || "/").split("?")[0];
    const st = f.status == null ? (f.error ? "ERR" : "...") : String(f.status);
    lines.push(`  F${f.id}["#${f.id} ${mermaidEsc(f.method)} ${mermaidEsc(ellipsizeText((f.host || "") + p, 42, "middle"))} (${st})"]`);
  }
  if (edges.length) {
    for (const e of edges) {
      lines.push(`  F${e.from.id} -->|${mermaidEsc(e.via)}| F${e.to.id}`);
    }
  } else {
    for (let i = 1; i < Math.min(flows.length, 60); i++) {
      lines.push(`  F${flows[i - 1].id} -.-> F${flows[i].id}`);
    }
  }
  return lines.join("\n");
}

function currentVisibleFlows() {
  const all = (!state.ended && state.data && Array.isArray(state.data.flows)) ? state.data.flows : [];
  return computeVisibleFlows(all);
}

function openSequenceModal(visibleFlows) {
  if (!Array.isArray(visibleFlows)) visibleFlows = currentVisibleFlows();
  state.seqChecked = new Set(visibleFlows.map(f => f.id));
  state.seqOpen = true;
  renderSequenceModal(visibleFlows);
}

function closeSequenceModal() {
  state.seqOpen = false;
  document.getElementById("seq-modal").classList.remove("open");
}

function renderSequenceModal(visibleFlows) {
  const backdrop = document.getElementById("seq-modal");
  const box = document.getElementById("seq-modal-box");
  backdrop.classList.toggle("open", state.seqOpen);
  if (!state.seqOpen) return;
  if (!Array.isArray(visibleFlows)) visibleFlows = currentVisibleFlows();

  const checkedFlows = visibleFlows.filter(f => state.seqChecked.has(f.id));
  const previewFlows = checkedFlows.slice(-200);
  const cappedNotice = checkedFlows.length > 200
    ? `<div class="body-note">${esc(tr(`已勾选 ${checkedFlows.length} 条，预览仅展示最近 200 条；复制 Mermaid / Markdown 包含全部勾选流量。`, `${checkedFlows.length} checked; preview shows latest 200. Copying Mermaid / Markdown includes all checked flows.`))}</div>`
    : "";
  const edges = (state.seqTab === "dependency" || (state.seqTab === "mermaid" && state.seqMermaidKind === "flowchart"))
    ? detectDependencies(previewFlows)
    : [];

  const tabs = [
    ["waterfall", tr("瀑布图", "Waterfall")],
    ["sequence", tr("时序图", "Sequence")],
    ["dependency", tr("依赖图", "Dependency")],
    ["mermaid", tr("源码", "Mermaid")],
  ];

  const mermaidSrc = state.seqMermaidKind === "flowchart"
    ? buildMermaidFlowchart(checkedFlows, edges)
    : buildMermaidSequence(checkedFlows, state.seqIncludeRes, state.seqIncludeQuery);

  let previewBody = "";
  if (!checkedFlows.length) {
    previewBody = `<div class="empty-state">${esc(tr("请在左侧至少勾选一条流量", "Check at least one flow on the left"))}</div>`;
  } else if (state.seqTab === "waterfall") {
    const validStarts = previewFlows.map(f => f.started_ms).filter(Boolean);
    const minStart = validStarts.length ? Math.min(...validStarts) : 0;
    let maxEnd = minStart + 1;
    previewFlows.forEach((f, idx) => {
      const s = f.started_ms || (minStart + idx * 25);
      const dur = Math.max(2, Number(f.total_ms || 20));
      if (s + dur > maxEnd) maxEnd = s + dur;
    });
    const span = Math.max(1, maxEnd - minStart);
    previewBody = `<div class="timing-legend" style="padding:2px 4px 6px">`
      + `<span><i class="t-dot" style="background:var(--t-dns)"></i>DNS</span>`
      + `<span><i class="t-dot" style="background:var(--t-tcp)"></i>TCP</span>`
      + `<span><i class="t-dot" style="background:var(--t-tls)"></i>TLS</span>`
      + `<span><i class="t-dot" style="background:var(--t-wait)"></i>${esc(tr("等待", "Wait"))}</span>`
      + `<span><i class="t-dot" style="background:var(--t-dl)"></i>${esc(tr("下载", "Download"))}</span>`
      + `</div>` + previewFlows.map((f, idx) => {
        const s = f.started_ms || (minStart + idx * 25);
        const dur = Math.max(2, Number(f.total_ms || 15));
        const leftPct = Math.min(96, Math.max(0, ((s - minStart) / span) * 100));
        const widthPct = Math.max(1.2, Math.min(100 - leftPct, (dur / span) * 100));
        const ci = f.conn_info || {};
        const phases = [
          [ci.dns_ms, "var(--t-dns)"],
          [ci.tcp_ms, "var(--t-tcp)"],
          [ci.tls_ms, "var(--t-tls)"],
          [ci.wait_ms, "var(--t-wait)"],
          [ci.download_ms, "var(--t-dl)"],
        ].filter(([v]) => v != null && v > 0);
        const pSum = phases.reduce((a, [v]) => a + v, 0);
        const segHtml = pSum > 0
          ? phases.map(([v, col]) => `<span style="width:${(v / pSum) * 100}%;background:${col}"></span>`).join("")
          : `<span style="width:100%;background:var(--accent)"></span>`;
        const p = state.seqIncludeQuery ? (f.path || "/") : (f.path || "/").split("?")[0];
        return `<div class="wf-row" data-jump="${f.id}">`
          + `<span class="mono muted-cell" style="width:36px">#${f.id}</span>`
          + `<span class="m-badge mono m-${esc(f.method)}">${esc(f.method)}</span>`
          + `<span class="mono" style="width:220px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap" title="${esc(f.url || "")}">${esc(ellipsizeText((f.host || "") + p, 34, "middle"))}</span>`
          + `<div class="wf-track"><div class="wf-bar" style="left:${leftPct.toFixed(2)}%;width:${widthPct.toFixed(2)}%">${segHtml}</div></div>`
          + `<span class="mono ${isSlow(f) ? "slow" : "muted-cell"}" style="width:68px;text-align:right">${esc(f.elapsed || "—")}</span>`
          + `</div>`;
      }).join("");
  } else if (state.seqTab === "sequence") {
    const baseMs = (previewFlows[0] && previewFlows[0].started_ms) || 0;
    previewBody = previewFlows.map(f => {
      const off = (baseMs && f.started_ms) ? `+${Math.max(0, f.started_ms - baseMs)} ms` : (f.started || "");
      const p = state.seqIncludeQuery ? (f.path || "/") : (f.path || "/").split("?")[0];
      const st = f.status == null ? (f.error ? "ERR" : "…") : String(f.status);
      return `<div class="seq-step-row" data-jump="${f.id}">`
        + `<span class="mono muted-cell" style="width:72px">${esc(off)}</span>`
        + `<span class="mono muted-cell">#${f.id}</span>`
        + `<span class="m-badge mono m-${esc(f.method)}">${esc(f.method)}</span>`
        + `<span class="mono" style="flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">Client → <b>${esc(f.host || "origin")}</b>${esc(ellipsizeText(p, 48, "middle"))}</span>`
        + (state.seqIncludeRes ? `<span class="${statusBadgeClass(f)} mono">${esc(st)}</span><span class="mono muted-cell">${esc(f.elapsed || "—")}</span>` : "")
        + `</div>`;
    }).join("");
  } else if (state.seqTab === "dependency") {
    if (edges.length) {
      previewBody = edges.map(e =>
        `<div class="dep-card" data-jump="${e.to.id}">`
        + `<span class="mono">#${e.from.id} ${esc(e.from.host || "")}${esc(ellipsizeText((e.from.path || "").split("?")[0], 24, "tail"))}</span>`
        + `<span class="rule-badge mono">${esc(e.source)} → ${esc(e.via)}</span>`
        + `<span class="mono">▶ #${e.to.id} <b>${esc(e.to.method)}</b> ${esc(e.to.host || "")}${esc(ellipsizeText((e.to.path || "").split("?")[0], 24, "tail"))}</span>`
        + `</div>`
      ).join("");
    } else {
      const byHost = new Map();
      previewFlows.forEach(f => { const h = f.host || "origin"; byHost.set(h, (byHost.get(h) || []).concat(f)); });
      previewBody = `<div class="muted-cell" style="font-size:12px;padding:4px">${esc(tr("未检测到跨请求显式 Token/ID 传递，按源站调用链聚合：", "No explicit cross-flow token/ID edges detected; grouped by host call chain:"))}</div>`
        + Array.from(byHost.entries()).map(([h, list]) =>
          `<div class="dep-card" data-jump="${list[0].id}"><b class="mono">${esc(h)}</b><span class="count-badge mono">${list.length}</span><span class="mono muted-cell" style="flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${list.map(x => "#" + x.id + " " + x.method).join(" → ")}</span></div>`
        ).join("");
    }
  } else {
    previewBody = `<div class="card"><div class="code-viewer expanded mono">${renderCodeLines(mermaidSrc, "").html}</div></div>`;
  }

  box.innerHTML = `<div class="modal-head">`
    + `<div style="display:flex;align-items:center;gap:10px">`
    + `<b style="font-size:14px">${esc(tr("生成时序图", "Sequence & Dependency Diagram"))}</b>`
    + `<div class="seg-group">` + tabs.map(([k, lbl]) => `<button class="seg-btn${state.seqTab === k ? " active" : ""}" data-seq-tab="${k}" type="button">${esc(lbl)}</button>`).join("") + `</div>`
    + `</div>`
    + `<div style="display:flex;align-items:center;gap:6px">`
    + `<button class="mini-btn" id="copy-mermaid-btn" type="button">${esc(tr("复制 Mermaid", "Copy Mermaid"))}</button>`
    + `<button class="mini-btn" id="copy-md-btn" type="button">${esc(tr("复制 Markdown", "Copy Markdown"))}</button>`
    + `<button class="mini-btn" id="close-seq-btn" type="button">✕</button>`
    + `</div></div>`
    + `<div class="modal-sub">`
    + `<div style="display:flex;align-items:center;gap:6px;flex-wrap:wrap">`
    + `<button class="mini-btn" id="seq-all-btn" type="button">${esc(tr("全选", "All"))}</button>`
    + `<button class="mini-btn" id="seq-none-btn" type="button">${esc(tr("全不选", "None"))}</button>`
    + `<button class="mini-btn" id="seq-inv-btn" type="button">${esc(tr("反选", "Invert"))}</button>`
    + `<span class="muted-cell mono">${checkedFlows.length} / ${visibleFlows.length}</span>`
    + `</div>`
    + `<div style="display:flex;align-items:center;gap:8px;flex-wrap:wrap">`
    + (state.seqTab === "mermaid" ? `<div class="seg-group"><button class="seg-btn${state.seqMermaidKind === "sequence" ? " active" : ""}" data-mk="sequence" type="button">sequenceDiagram</button><button class="seg-btn${state.seqMermaidKind === "flowchart" ? " active" : ""}" data-mk="flowchart" type="button">flowchart TD</button></div>` : "")
    + `<label style="display:inline-flex;align-items:center;gap:4px;cursor:pointer"><input type="checkbox" id="seq-inc-res" ${state.seqIncludeRes ? "checked" : ""}> ${esc(tr("含响应", "Responses"))}</label>`
    + `<label style="display:inline-flex;align-items:center;gap:4px;cursor:pointer"><input type="checkbox" id="seq-inc-q" ${state.seqIncludeQuery ? "checked" : ""}> ${esc(tr("含查询参数", "Query params"))}</label>`
    + `</div></div>`
    + cappedNotice
    + `<div class="modal-body">`
    + `<div class="seq-pick-list">` + visibleFlows.map(f =>
      `<label class="seq-pick-item"><input type="checkbox" data-seq-id="${f.id}" ${state.seqChecked.has(f.id) ? "checked" : ""}><span class="mono muted-cell">#${f.id}</span><span class="m-badge mono m-${esc(f.method)}">${esc(f.method)}</span><span class="mono" style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(ellipsizeText((f.path || "/").split("?")[0], 20, "tail"))}</span></label>`
    ).join("") + `</div>`
    + `<div class="seq-preview">${previewBody}</div>`
    + `</div>`;

  box.querySelector("#close-seq-btn").onclick = closeSequenceModal;
  box.querySelector("#copy-mermaid-btn").onclick = function() { copyRawText(mermaidSrc, this); };
  box.querySelector("#copy-md-btn").onclick = function() { copyRawText("```mermaid\n" + mermaidSrc + "\n```", this); };
  box.querySelector("#seq-all-btn").onclick = () => { visibleFlows.forEach(f => state.seqChecked.add(f.id)); renderSequenceModal(visibleFlows); };
  box.querySelector("#seq-none-btn").onclick = () => { state.seqChecked.clear(); renderSequenceModal(visibleFlows); };
  box.querySelector("#seq-inv-btn").onclick = () => {
    visibleFlows.forEach(f => { if (state.seqChecked.has(f.id)) state.seqChecked.delete(f.id); else state.seqChecked.add(f.id); });
    renderSequenceModal(visibleFlows);
  };
  box.querySelector("#seq-inc-res").onchange = (e) => { state.seqIncludeRes = e.target.checked; renderSequenceModal(visibleFlows); };
  box.querySelector("#seq-inc-q").onchange = (e) => { state.seqIncludeQuery = e.target.checked; renderSequenceModal(visibleFlows); };
  box.querySelectorAll("[data-seq-tab]").forEach(btn => {
    btn.onclick = () => { state.seqTab = btn.dataset.seqTab; renderSequenceModal(visibleFlows); };
  });
  box.querySelectorAll("[data-mk]").forEach(btn => {
    btn.onclick = () => { state.seqMermaidKind = btn.dataset.mk; renderSequenceModal(visibleFlows); };
  });
  box.querySelectorAll("[data-seq-id]").forEach(cb => {
    cb.onchange = () => {
      const id = Number(cb.dataset.seqId);
      if (cb.checked) state.seqChecked.add(id); else state.seqChecked.delete(id);
      renderSequenceModal(visibleFlows);
    };
  });
  box.querySelectorAll("[data-jump]").forEach(row => {
    row.onclick = () => {
      const id = Number(row.dataset.jump);
      closeSequenceModal();
      selectFlowById(id);
    };
  });
}

function b64ToBytes(b64) {
  if (!b64) return new Uint8Array(0);
  try {
    const bin = atob(b64);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  } catch (_) {
    return new Uint8Array(0);
  }
}

function formatHexDump(b64, cols) {
  const bytes = b64ToBytes(b64);
  if (!bytes.length) return { html: "", cleanHex: "" };
  const perLine = cols === 32 ? 32 : 16;
  const linesHtml = [];
  let rowIdx = 0;
  for (let off = 0; off < bytes.length; off += perLine) {
    const slice = bytes.subarray(off, Math.min(bytes.length, off + perLine));
    const hexParts = [];
    let ascii = "";
    for (let i = 0; i < perLine; i++) {
      if (i === 16 && perLine === 32) hexParts.push("");
      if (i < slice.length) {
        const h = slice[i].toString(16).padStart(2, "0");
        hexParts.push(h);
        ascii += (slice[i] >= 0x20 && slice[i] <= 0x7e) ? String.fromCharCode(slice[i]) : ".";
      } else {
        hexParts.push("  ");
      }
    }
    rowIdx++;
    const ln = String(rowIdx).padStart(4, "0");
    linesHtml.push(`<div class="code-line"><span class="ln">${ln}</span><span class="lc">${esc(hexParts.join(" "))}  ${esc(ascii)}</span></div>`);
  }
  const cleanLines = [];
  for (let off = 0; off < bytes.length; off += 16) {
    const slice = bytes.subarray(off, Math.min(bytes.length, off + 16));
    const row = [];
    for (let i = 0; i < slice.length; i++) row.push(slice[i].toString(16).padStart(2, "0"));
    cleanLines.push(row.join(" "));
  }
  return { html: linesHtml.join(""), cleanHex: cleanLines.join("\n") };
}

function highlightEscaped(line, needleLower) {
  if (!needleLower) return esc(line);
  const low = line.toLowerCase();
  let out = "";
  let pos = 0;
  while (pos < line.length) {
    const idx = low.indexOf(needleLower, pos);
    if (idx < 0) { out += esc(line.slice(pos)); break; }
    out += esc(line.slice(pos, idx));
    out += `<mark class="hl">${esc(line.slice(idx, idx + needleLower.length))}</mark>`;
    pos = idx + needleLower.length;
  }
  return out;
}

function renderCodeLines(text, query) {
  const raw = String(text ?? "");
  const lines = raw.split("\n");
  const q = String(query || "").trim().toLowerCase();
  if (!q) {
    const html = lines.map((line, i) =>
      `<div class="code-line"><span class="ln">${String(i + 1).padStart(4, "0")}</span><span class="lc">${esc(line)}</span></div>`
    ).join("");
    return { hits: 0, html };
  }
  let hits = 0;
  const matched = [];
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const low = line.toLowerCase();
    let pos = 0, c = 0;
    while ((pos = low.indexOf(q, pos)) >= 0) { c++; pos += q.length; }
    if (c > 0) {
      hits += c;
      matched.push(`<div class="code-line"><span class="ln">[L${i + 1}]</span><span class="lc">${highlightEscaped(line, q)}</span></div>`);
    }
  }
  const html = matched.length
    ? matched.join("")
    : `<div style="padding:10px 12px;color:var(--fg-muted)">${esc(tr("无匹配行", "No matching lines"))}</div>`;
  return { hits, html };
}

function renderSafeMarkdown(md) {
  const lines = String(md || "").replace(/\r\n/g, "\n").split("\n");
  let out = "";
  let inCode = false;
  let codeBuf = [];
  const fmtInline = (s) => {
    let e = esc(s);
    e = e.replace(/`([^`]+)`/g, "<code>$1</code>");
    e = e.replace(/\*\*([^*]+)\*\*/g, "<b>$1</b>");
    e = e.replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>');
    return e;
  };
  for (const line of lines) {
    if (line.trim().startsWith("```")) {
      if (inCode) { out += `<pre class="mono">${esc(codeBuf.join("\n"))}</pre>`; codeBuf = []; inCode = false; }
      else inCode = true;
      continue;
    }
    if (inCode) { codeBuf.push(line); continue; }
    if (/^###\s+/.test(line)) out += `<h3>${fmtInline(line.replace(/^###\s+/, ""))}</h3>`;
    else if (/^##\s+/.test(line)) out += `<h2>${fmtInline(line.replace(/^##\s+/, ""))}</h2>`;
    else if (/^#\s+/.test(line)) out += `<h1>${fmtInline(line.replace(/^#\s+/, ""))}</h1>`;
    else if (/^[-*]\s+/.test(line)) out += `<div>• ${fmtInline(line.replace(/^[-*]\s+/, ""))}</div>`;
    else if (!line.trim()) out += `<div style="height:6px"></div>`;
    else out += `<div>${fmtInline(line)}</div>`;
  }
  if (inCode && codeBuf.length) out += `<pre class="mono">${esc(codeBuf.join("\n"))}</pre>`;
  return out;
}

function flowCanonicalSnapshot(flow) {
  const lines = [`${flow.method || "GET"} ${flow.url || flow.path || "/"}`];
  const reqH = (flow.request_headers || []).slice().sort((a, b) => a[0].toLowerCase().localeCompare(b[0].toLowerCase()));
  reqH.forEach(([k, v]) => lines.push(`${k}: ${v}`));
  if (flow.request_body_pretty || flow.request_body) {
    lines.push("", "--- request body ---");
    String(flow.request_body_pretty || flow.request_body).split("\n").forEach(l => lines.push(l));
  }
  lines.push("", `HTTP ${flow.status != null ? flow.status : (flow.error ? "ERR" : "—")}`);
  const resH = (flow.response_headers || []).slice().sort((a, b) => a[0].toLowerCase().localeCompare(b[0].toLowerCase()));
  resH.forEach(([k, v]) => lines.push(`${k}: ${v}`));
  if (flow.response_body_pretty || flow.response_body) {
    lines.push("", "--- response body ---");
    String(flow.response_body_pretty || flow.response_body).split("\n").forEach(l => lines.push(l));
  }
  return lines;
}

function computeLineDiff(a, b) {
  const maxLines = 400;
  const left = a.slice(0, maxLines);
  const right = b.slice(0, maxLines);
  const n = left.length, m = right.length;
  const dp = Array.from({ length: n + 1 }, () => new Uint16Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = left[i] === right[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  const out = [];
  let i = 0, j = 0;
  while (i < n && j < m) {
    if (left[i] === right[j]) { out.push({ kind: "ctx", text: left[i] }); i++; j++; }
    else if (dp[i + 1][j] >= dp[i][j + 1]) { out.push({ kind: "del", text: left[i++] }); }
    else { out.push({ kind: "add", text: right[j++] }); }
  }
  while (i < n) out.push({ kind: "del", text: left[i++] });
  while (j < m) out.push({ kind: "add", text: right[j++] });
  return out;
}
async function ensureFlowBody(flow) {
  if (!LIVE || !flow || flow._bodyLoading || flow._bodyLoaded) return;
  if (!flow.request_body_spilled && !flow.response_body_spilled) return;
  flow._bodyLoading = true;
  try {
    const base = LIVE.replace(/\/flows\.json.*$/, "");
    const res = await fetch(`${base}/flows/${flow.id}.json`, { cache: "no-store" });
    if (res.ok) {
      const b = await res.json();
      const keys = [
        "request_body", "request_body_pretty", "request_body_kind", "request_form_rows",
        "request_hex_b64", "request_raw_len", "request_truncated", "request_expired", "request_encoding_error",
        "response_body", "response_body_pretty", "response_body_kind",
        "response_hex_b64", "response_raw_len", "response_truncated", "response_expired", "response_encoding_error",
        "response_image_data_url", "response_image_dims",
        "graphql_op", "messages", "message_items", "messages_truncated"
      ];
      for (const k of keys) {
        if (b[k] !== undefined) flow[k] = b[k];
      }
      flow.request_body_spilled = false;
      flow.response_body_spilled = false;
      flow._bodyLoaded = true;
      renderAll();
    }
  } catch (_) {
  } finally {
    flow._bodyLoading = false;
  }
}

function downloadResponseBody(flow) {
  let blob = null;
  let ext = "bin";
  const kind = flow.response_body_kind || "text";
  if (flow.response_image_data_url && flow.response_image_data_url.startsWith("data:")) {
    const comma = flow.response_image_data_url.indexOf(",");
    if (comma > 0) {
      const meta = flow.response_image_data_url.slice(5, comma);
      const mime = meta.split(";")[0] || "application/octet-stream";
      const bytes = b64ToBytes(flow.response_image_data_url.slice(comma + 1));
      blob = new Blob([bytes], { type: mime });
      const sub = (mime.split("/")[1] || "bin").replace("jpeg", "jpg").replace("x-icon", "ico");
      ext = sub.replace(/[^a-z0-9]/gi, "") || "img";
    }
  }
  if (!blob && flow.response_body) {
    const extMap = { json: "json", graphql: "graphql", html: "html", xml: "xml", markdown: "md", text: "txt" };
    ext = extMap[kind] || "txt";
    blob = new Blob([flow.response_body], { type: "text/plain;charset=utf-8" });
  }
  if (!blob && flow.response_hex_b64) {
    blob = new Blob([b64ToBytes(flow.response_hex_b64)], { type: "application/octet-stream" });
    ext = "bin";
  }
  if (!blob) return;
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `flow-${flow.id}.${ext}`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function openDetailMoreMenu(flow, anchorBtn) {
  closeAllMenus();
  const menu = document.getElementById("more-menu");
  const hasJson = !!sampleJsonForTypes(flow);
  const reqCopy = state.pretty && flow.request_body_pretty ? flow.request_body_pretty : (flow.request_body || "");
  const resCopy = state.pretty && flow.response_body_pretty ? flow.response_body_pretty : (flow.response_body || "");
  const canDl = !!(flow.response_image_data_url || flow.response_body || flow.response_hex_b64);
  const pinned = isPinned(flow);
  const isBase = state.baselineId === flow.id;

  const items = [
    { lbl: tr("复制 URL", "Copy URL"), fn: () => copyRawText(flow.url || flow.path || "") },
    { lbl: tr("复制 cURL", "Copy cURL"), fn: () => copyRawText(buildCurl(flow)) },
    { lbl: tr("复制 fetch", "Copy fetch"), fn: () => copyRawText(buildFetch(flow)) },
    { lbl: tr("复制 Python requests", "Copy Python requests"), fn: () => copyRawText(buildPythonRequests(flow)) },
    { lbl: tr("复制 Rust reqwest", "Copy Rust reqwest"), fn: () => copyRawText(buildRustReqwest(flow)) },
    { lbl: tr("复制 Go net/http", "Copy Go net/http"), fn: () => copyRawText(buildGoHttp(flow)) },
    { lbl: tr("复制 TS 类型", "Copy TS interface"), dis: !hasJson, fn: () => copyRawText(buildTsInterface(flow)) },
    { lbl: tr("复制 Rust 结构体", "Copy Rust struct"), dis: !hasJson, fn: () => copyRawText(buildRustStruct(flow)) },
    { sep: true },
    { lbl: tr("复制请求正文", "Copy request body"), dis: !reqCopy, fn: () => copyRawText(reqCopy) },
    { lbl: tr("复制响应正文", "Copy response body"), dis: !resCopy, fn: () => copyRawText(resCopy) },
    { lbl: tr("另存响应正文", "Download response"), dis: !canDl, fn: () => downloadResponseBody(flow) },
    { sep: true },
    { lbl: tr("只看此域名", "Only this host"), dis: !flow.host, fn: () => onlyHostFilter(flow.host) },
    { lbl: tr("屏蔽此域名", "Mute this host"), dis: !flow.host, fn: () => togglePrefixOrToken("mute:", flow.host) },
    { lbl: pinned ? tr("取消星标", "Unstar") : tr("星标", "Star"), fn: () => togglePin(flow) },
    { lbl: isBase ? tr("取消基准", "Clear baseline") : tr("设为基准", "Set as baseline"), fn: () => { state.baselineId = isBase ? null : flow.id; if (state.baselineId != null) ensureFlowBody(flow); renderAll(); } },
    { lbl: state.pretty ? tr("切换原文", "Switch to Raw") : tr("切换美化", "Switch to Pretty"), fn: () => { state.pretty = !state.pretty; renderAll(); } },
  ];

  menu.innerHTML = items.map((it, i) => it.sep
    ? `<div class="menu-sep"></div>`
    : `<button class="menu-item" data-mi="${i}" type="button" ${it.dis ? "disabled" : ""}><span>${esc(it.lbl)}</span></button>`
  ).join("");

  menu.querySelectorAll("button[data-mi]").forEach(btn => {
    btn.onclick = () => {
      const it = items[Number(btn.dataset.mi)];
      closeAllMenus();
      if (it && !it.dis && it.fn) it.fn();
    };
  });

  const rect = anchorBtn.getBoundingClientRect();
  positionMenu(menu, rect.right - 200, rect.bottom + 4);
}

function formatHeadersForCard(rows, pretty, splitCookie) {
  const src = Array.isArray(rows) ? rows.slice() : [];
  if (pretty) {
    src.sort((a, b) => String(a[0]).toLowerCase().localeCompare(String(b[0]).toLowerCase()));
  }
  if (!pretty || !splitCookie) return src;
  const out = [];
  for (const [k, v] of src) {
    if (String(k).toLowerCase() === "cookie") {
      const parts = String(v || "").split(";").map(s => s.trim()).filter(Boolean);
      if (parts.length > 1) {
        for (const p of parts) {
          const eq = p.indexOf("=");
          if (eq > 0) out.push([`Cookie · ${p.slice(0, eq).trim()}`, p.slice(eq + 1).trim()]);
          else out.push(["Cookie", p]);
        }
        continue;
      }
    }
    out.push([k, v]);
  }
  return out;
}

function createKvCard(foldKey, title, displayRows, rawCopyText) {
  const card = document.createElement("div");
  card.className = "card" + (state.folded[foldKey] ? " folded" : "");
  const head = document.createElement("div");
  head.className = "card-head";
  head.innerHTML = `<div class="card-title-wrap"><span class="card-arrow">▾</span><span class="card-title">${esc(title)}</span><span class="count-badge mono">${displayRows.length}</span></div>`
    + `<div class="card-actions"><button class="mini-btn" data-act="copy" type="button">${esc(tr("复制", "Copy"))}</button></div>`;
  head.onclick = (e) => {
    if (e.target.closest("button")) return;
    state.folded[foldKey] = !state.folded[foldKey];
    card.classList.toggle("folded", !!state.folded[foldKey]);
  };
  head.querySelector('[data-act="copy"]').onclick = (e) => {
    e.stopPropagation();
    copyRawText(rawCopyText, e.currentTarget);
  };
  const body = document.createElement("div");
  body.className = "card-body mono";
  body.innerHTML = displayRows.map(([k, v]) => {
    const hint = annotateValue(v);
    const hintHtml = hint ? `<div class="kv-hint mono">${esc(hint)}</div>` : "";
    return `<div class="kv-row"><div class="kv-key">${esc(k)}</div><div class="kv-val">${esc(v)}${hintHtml}</div></div>`;
  }).join("");
  card.appendChild(head);
  card.appendChild(body);
  return card;
}

function createBodyCard(side, flow) {
  const isRes = side === "res";
  const foldKey = isRes ? "res_body" : "req_body";
  const rawText = isRes ? (flow.response_body || "") : (flow.request_body || "");
  const prettyText = isRes ? (flow.response_body_pretty || "") : (flow.request_body_pretty || "");
  const kind = (isRes ? flow.response_body_kind : flow.request_body_kind) || "text";
  const formRows = !isRes ? (flow.request_form_rows || []) : [];
  const imgUrl = isRes ? (flow.response_image_data_url || "") : "";
  const imgDims = isRes ? flow.response_image_dims : null;
  const hexB64 = isRes ? (flow.response_hex_b64 || "") : (flow.request_hex_b64 || "");
  const spilled = isRes ? !!flow.response_body_spilled : !!flow.request_body_spilled;
  const truncated = isRes ? !!flow.response_truncated : !!flow.request_truncated;
  const expired = isRes ? flow.response_expired : flow.request_expired;
  const encErr = isRes ? (flow.response_encoding_error || "") : (flow.request_encoding_error || "");
  const rawLen = Number((isRes ? flow.response_raw_len : flow.request_raw_len) || (isRes ? flow.response_bytes : flow.request_bytes) || 0);

  if (!rawText && !prettyText && !formRows.length && !imgUrl && !hexB64 && !spilled && !truncated && expired == null && !encErr) {
    return null;
  }

  const canPreview = (kind === "html" || kind === "markdown") && !!rawText;
  const previewOn = canPreview && (isRes ? state.resPreview : state.reqPreview);
  const expanded = isRes ? state.resExpanded : state.reqExpanded;
  const title = isRes ? tr("响应正文", "Response Body") : tr("请求正文", "Request Body");
  const kindBadge = kind.toUpperCase() + (rawLen ? ` · ${formatBytes(rawLen)}` : "");

  const card = document.createElement("div");
  card.className = "card" + (state.folded[foldKey] ? " folded" : "");
  const head = document.createElement("div");
  head.className = "card-head";

  const searchValInit = isRes ? state.resBodySearch : state.reqBodySearch;
  head.innerHTML = `<div class="card-title-wrap">`
    + `<span class="card-arrow">▾</span>`
    + `<span class="card-title">${esc(title)}</span>`
    + `<span class="count-badge mono">${esc(kindBadge)}</span>`
    + `<span class="count-badge mono" data-role="hits" style="display:none"></span>`
    + `</div>`
    + `<div class="card-actions">`
    + `<input class="mini-input" data-role="search" type="text" value="${esc(searchValInit)}" placeholder="${esc(tr("正文内搜索…", "Search body…"))}" autocomplete="off" spellcheck="false">`
    + (canPreview ? `<button class="mini-btn${previewOn ? " active" : ""}" data-act="preview" type="button">${esc(tr("预览", "Preview"))}</button>` : "")
    + `<button class="mini-btn${expanded ? " active" : ""}" data-act="expand" type="button">${esc(expanded ? tr("收起高度", "Compact") : tr("展开高度", "Expand"))}</button>`
    + `<button class="mini-btn${state.pretty ? " active" : ""}" data-act="pretty" type="button">${esc(state.pretty ? tr("美化", "Pretty") : tr("原文", "Raw"))}</button>`
    + `<button class="mini-btn" data-act="copy" type="button">${esc(tr("复制", "Copy"))}</button>`
    + (isRes && (imgUrl || rawText || hexB64) ? `<button class="mini-btn" data-act="dl" type="button">${esc(tr("另存", "Save"))}</button>` : "")
    + `</div>`;

  const bodyEl = document.createElement("div");
  bodyEl.className = "card-body";
  let currentCopyPayload = "";

  const renderInner = () => {
    const q = (isRes ? state.resBodySearch : state.reqBodySearch).trim();
    const hitsBadge = head.querySelector('[data-role="hits"]');
    let notesHtml = "";

    if (spilled) {
      bodyEl.innerHTML = `<div class="body-note">${esc(LIVE ? tr("正在按需加载大体积正文…", "Loading spilled body…") : tr("大体积正文未内联", "Large body omitted"))}</div>`;
      hitsBadge.style.display = "none";
      currentCopyPayload = "";
      return;
    }
    if (expired != null) {
      notesHtml += `<div class="body-note">${esc(tr(`大体积正文（${formatBytes(expired)}）已从溢出缓冲区回收`, `Large body (${formatBytes(expired)}) reclaimed from spill ring`))}</div>`;
    }
    if (encErr) {
      notesHtml += `<div class="body-note">${esc(tr(`解压失败，按原文展示：${encErr}`, `Decompression failed, showing raw: ${encErr}`))}</div>`;
    }
    if (truncated) {
      notesHtml += `<div class="body-note">${esc(tr(`正文超过记录上限已截断（总计 ${formatBytes(rawLen)}）`, `Body truncated at capture cap (${formatBytes(rawLen)} total)`))}</div>`;
    }

    const showPrettyImage = isRes && state.pretty && !!imgUrl && !previewOn;
    if (showPrettyImage) {
      hitsBadge.style.display = "none";
      const dimStr = Array.isArray(imgDims) ? `${imgDims[0]} × ${imgDims[1]} px · ` : "";
      currentCopyPayload = imgUrl;
      bodyEl.innerHTML = notesHtml
        + `<div class="img-preview-wrap">`
        + `<div class="img-checker"><img src="${esc(imgUrl)}" alt="response image"></div>`
        + `<div class="muted-cell mono" style="font-size:11px">${esc(dimStr + formatBytes(rawLen))}</div>`
        + `</div>`;
      return;
    }

    if (previewOn && kind === "html") {
      hitsBadge.style.display = "none";
      currentCopyPayload = rawText;
      bodyEl.innerHTML = notesHtml + `<iframe class="html-preview-frame" sandbox="" referrerpolicy="no-referrer" srcdoc="${esc(rawText)}"></iframe>`;
      return;
    }
    if (previewOn && kind === "markdown") {
      hitsBadge.style.display = "none";
      currentCopyPayload = rawText;
      bodyEl.innerHTML = notesHtml + `<div class="md-preview">${renderSafeMarkdown(rawText)}</div>`;
      return;
    }

    if (!isRes && state.pretty && formRows.length > 0) {
      const qLow = q.toLowerCase();
      const filteredRows = qLow ? formRows.filter(([k, v]) => k.toLowerCase().includes(qLow) || v.toLowerCase().includes(qLow)) : formRows;
      if (qLow) {
        hitsBadge.style.display = "inline-flex";
        hitsBadge.textContent = tr(`${filteredRows.length} 项命中`, `${filteredRows.length} hits`);
      } else {
        hitsBadge.style.display = "none";
      }
      currentCopyPayload = rawText || formRows.map(([k, v]) => `${k}=${v}`).join("&");
      bodyEl.innerHTML = notesHtml + `<div class="mono">` + (filteredRows.length
        ? filteredRows.map(([k, v]) => {
            const hint = annotateValue(v);
            return `<div class="kv-row"><div class="kv-key">${highlightEscaped(k, qLow)}</div><div class="kv-val">${highlightEscaped(v, qLow)}${hint ? `<div class="kv-hint mono">${esc(hint)}</div>` : ""}</div></div>`;
          }).join("")
        : `<div style="padding:10px 12px;color:var(--fg-muted)">${esc(tr("无匹配表单项", "No matching fields"))}</div>`
      ) + `</div>`;
      return;
    }

    const useHex = !!hexB64 && (kind === "binary" || (!state.pretty && ["image", "protobuf", "msgpack"].includes(kind)));
    if (useHex) {
      if (rawLen > 4096) {
        notesHtml += `<div class="body-note">${esc(tr(`十六进制只画前 4096 字节（共 ${formatBytes(rawLen)}）`, `Hex view shows first 4096 bytes (${formatBytes(rawLen)} total)`))}</div>`;
      }
      const hexRes = formatHexDump(hexB64, state.hexCols);
      currentCopyPayload = hexRes.cleanHex;
      if (q) {
        const codeRes = renderCodeLines(hexRes.cleanHex, q);
        hitsBadge.style.display = "inline-flex";
        hitsBadge.textContent = tr(`${codeRes.hits} 处命中`, `${codeRes.hits} hits`);
        bodyEl.innerHTML = notesHtml + `<div class="code-viewer hex-dump mono${expanded ? " expanded" : ""}">${codeRes.html}</div>`;
      } else {
        hitsBadge.style.display = "none";
        bodyEl.innerHTML = notesHtml + `<div class="code-viewer hex-dump mono${expanded ? " expanded" : ""}">${hexRes.html}</div>`;
      }
      return;
    }

    const shownText = (state.pretty && prettyText) ? prettyText : (rawText || prettyText);
    currentCopyPayload = shownText;
    if (!shownText) {
      hitsBadge.style.display = "none";
      bodyEl.innerHTML = notesHtml;
      return;
    }
    const codeRes = renderCodeLines(shownText, q);
    if (q) {
      hitsBadge.style.display = "inline-flex";
      hitsBadge.textContent = tr(`${codeRes.hits} 处命中`, `${codeRes.hits} hits`);
    } else {
      hitsBadge.style.display = "none";
    }
    bodyEl.innerHTML = notesHtml + `<div class="code-viewer mono${expanded ? " expanded" : ""}">${codeRes.html}</div>`;
  };

  renderInner();

  head.onclick = (e) => {
    if (e.target.closest("button") || e.target.closest("input")) return;
    state.folded[foldKey] = !state.folded[foldKey];
    card.classList.toggle("folded", !!state.folded[foldKey]);
  };
  const searchInput = head.querySelector('[data-role="search"]');
  searchInput.onclick = (e) => e.stopPropagation();
  searchInput.oninput = () => {
    if (isRes) state.resBodySearch = searchInput.value;
    else state.reqBodySearch = searchInput.value;
    renderInner();
  };
  const prevBtn = head.querySelector('[data-act="preview"]');
  if (prevBtn) {
    prevBtn.onclick = (e) => {
      e.stopPropagation();
      if (isRes) state.resPreview = !state.resPreview;
      else state.reqPreview = !state.reqPreview;
      renderAll();
    };
  }
  head.querySelector('[data-act="expand"]').onclick = (e) => {
    e.stopPropagation();
    if (isRes) state.resExpanded = !state.resExpanded;
    else state.reqExpanded = !state.reqExpanded;
    renderAll();
  };
  head.querySelector('[data-act="pretty"]').onclick = (e) => {
    e.stopPropagation();
    state.pretty = !state.pretty;
    renderAll();
  };
  head.querySelector('[data-act="copy"]').onclick = (e) => {
    e.stopPropagation();
    copyRawText(currentCopyPayload, e.currentTarget);
  };
  const dlBtn = head.querySelector('[data-act="dl"]');
  if (dlBtn) {
    dlBtn.onclick = (e) => {
      e.stopPropagation();
      downloadResponseBody(flow);
    };
  }

  card.appendChild(head);
  card.appendChild(bodyEl);
  return card;
}
function createDiffCard(flow) {
  if (state.baselineId == null || state.baselineId === flow.id) return null;
  const all = (state.data && Array.isArray(state.data.flows)) ? state.data.flows : [];
  const baseFlow = all.find(f => f.id === state.baselineId);
  if (!baseFlow) return null;
  ensureFlowBody(baseFlow);

  const foldKey = "diff";
  const diffRows = computeLineDiff(flowCanonicalSnapshot(baseFlow), flowCanonicalSnapshot(flow));
  let adds = 0, dels = 0;
  for (const r of diffRows) {
    if (r.kind === "add") adds++;
    else if (r.kind === "del") dels++;
  }
  const shown = state.diffChangesOnly ? diffRows.filter(r => r.kind !== "ctx") : diffRows;

  const card = document.createElement("div");
  card.className = "card" + (state.folded[foldKey] ? " folded" : "");
  const head = document.createElement("div");
  head.className = "card-head";
  head.innerHTML = `<div class="card-title-wrap">`
    + `<span class="card-arrow">▾</span>`
    + `<span class="card-title">${esc(tr(`行级对比 #${baseFlow.id} (基准) ↔ #${flow.id} (当前)`, `Diff #${baseFlow.id} (Base) ↔ #${flow.id} (Current)`))}</span>`
    + `<span class="count-badge mono">+${adds} / -${dels}</span>`
    + `</div>`
    + `<div class="card-actions">`
    + `<button class="mini-btn${state.diffChangesOnly ? " active" : ""}" data-act="changes" type="button">${esc(tr("仅看差异", "Changes only"))}</button>`
    + `<button class="mini-btn" data-act="clear-base" type="button">${esc(tr("取消基准", "Clear baseline"))}</button>`
    + `</div>`;

  head.onclick = (e) => {
    if (e.target.closest("button")) return;
    state.folded[foldKey] = !state.folded[foldKey];
    card.classList.toggle("folded", !!state.folded[foldKey]);
  };
  head.querySelector('[data-act="changes"]').onclick = (e) => {
    e.stopPropagation();
    state.diffChangesOnly = !state.diffChangesOnly;
    renderAll();
  };
  head.querySelector('[data-act="clear-base"]').onclick = (e) => {
    e.stopPropagation();
    state.baselineId = null;
    renderAll();
  };

  const body = document.createElement("div");
  body.className = "card-body";
  if (adds === 0 && dels === 0) {
    body.innerHTML = `<div style="padding:10px 12px;color:var(--fg-muted);font-size:12px">${esc(tr("与基准请求完全一致", "Identical to baseline flow"))}</div>`;
  } else {
    body.innerHTML = `<div class="diff-viewer mono">` + shown.map(r => {
      const sign = r.kind === "del" ? "-" : (r.kind === "add" ? "+" : " ");
      return `<div class="diff-line ${r.kind}"><span class="diff-sign">${sign}</span><span>${esc(r.text)}</span></div>`;
    }).join("") + `</div>`;
  }
  card.appendChild(head);
  card.appendChild(body);
  return card;
}

function normalizeMessageItems(flow) {
  if (Array.isArray(flow.message_items) && flow.message_items.length) return flow.message_items;
  const raw = Array.isArray(flow.messages) ? flow.messages : [];
  return raw.map(line => {
    const s = String(line || "");
    if (s.startsWith("SSE")) {
      const idx = s.indexOf("·");
      return { dir: "down", kind: "sse", label: idx > 0 ? s.slice(0, idx).trim() : "SSE", text: idx > 0 ? s.slice(idx + 1).trim() : s };
    }
    const sseLegacy = s.match(/^←\s*\[([^\]]+)\]\s*([\s\S]*)$/);
    if (sseLegacy) {
      return { dir: "down", kind: "sse", label: sseLegacy[1] || "message", text: sseLegacy[2] || "" };
    }
    const up = s.startsWith("↑") || s.startsWith("→");
    const rest = s.replace(/^[↑↓→←]\s*/, "");
    let lbl = "text", txt = rest;
    const dotIdx = rest.indexOf("·");
    const spIdx = rest.indexOf(" ");
    if (dotIdx > 0 && (spIdx < 0 || dotIdx < spIdx + 8)) {
      lbl = rest.slice(0, dotIdx).trim();
      txt = rest.slice(dotIdx + 1).trim();
    } else if (spIdx > 0) {
      lbl = rest.slice(0, spIdx).trim();
      txt = rest.slice(spIdx + 1);
    }
    const low = lbl.toLowerCase();
    const kind = (low.includes("二进制") || low.includes("bin")) ? "binary"
      : ((low.includes("ping") || low.includes("pong") || low.includes("close") || low.includes("关闭")) ? "control" : "text");
    return { dir: up ? "up" : "down", kind, label: lbl, text: txt };
  });
}

function createMessagesCard(flow) {
  const items = normalizeMessageItems(flow);
  if (!items.length) return null;
  const foldKey = "messages";
  const card = document.createElement("div");
  card.className = "card" + (state.folded[foldKey] ? " folded" : "");

  const head = document.createElement("div");
  head.className = "card-head";
  const badgeStr = String(items.length) + (flow.messages_truncated ? "+" : "");

  const dirs = [
    ["all", tr("全部", "All")],
    ["up", tr("↑ 客户端", "↑ Client")],
    ["down", tr("↓ 源站", "↓ Server")],
  ];
  const kinds = [
    ["all", tr("全部", "All")],
    ["text", tr("文本", "Text")],
    ["binary", tr("二进制", "Bin")],
    ["control", tr("控制", "Ctrl")],
  ];

  head.innerHTML = `<div class="card-title-wrap">`
    + `<span class="card-arrow">▾</span>`
    + `<span class="card-title">${esc(tr("消息", "Messages"))}</span>`
    + `<span class="count-badge mono">${esc(badgeStr)}</span>`
    + `</div>`
    + `<div class="card-actions">`
    + `<div class="seg-group">` + dirs.map(([k, l]) => `<button class="seg-btn${state.msgDir === k ? " active" : ""}" data-mdir="${k}" type="button">${esc(l)}</button>`).join("") + `</div>`
    + `<div class="seg-group">` + kinds.map(([k, l]) => `<button class="seg-btn${state.msgKind === k ? " active" : ""}" data-mkind="${k}" type="button">${esc(l)}</button>`).join("") + `</div>`
    + `<input class="mini-input" data-role="msearch" type="text" value="${esc(state.msgSearch)}" placeholder="${esc(tr("搜索消息…", "Search frames…"))}" autocomplete="off" spellcheck="false" style="width:110px">`
    + `<button class="mini-btn" data-act="copy-all" type="button">${esc(tr("复制全部", "Copy all"))}</button>`
    + `</div>`;

  const bodyEl = document.createElement("div");
  bodyEl.className = "card-body";
  let filteredCopyText = "";

  const renderInner = () => {
    const q = state.msgSearch.trim().toLowerCase();
    const filtered = items.filter(it => {
      if (state.msgDir !== "all" && it.dir !== state.msgDir) return false;
      if (state.msgKind !== "all") {
        if (state.msgKind === "text" && it.kind !== "text" && it.kind !== "sse") return false;
        if (state.msgKind !== "text" && it.kind !== state.msgKind) return false;
      }
      if (q && !String(it.text || "").toLowerCase().includes(q) && !String(it.label || "").toLowerCase().includes(q)) return false;
      return true;
    });
    filteredCopyText = filtered.map(it => `${it.kind === "sse" ? "SSE" : (it.dir === "up" ? "↑" : "↓")} [${it.label}] ${it.text}`).join("\n");

    let html = flow.messages_truncated
      ? `<div class="body-note">${esc(tr("消息较多，仅展示前 200 条帧", "Showing first 200 message frames"))}</div>`
      : "";
    if (!filtered.length) {
      html += `<div style="padding:10px 12px;color:var(--fg-muted);font-size:12px">${esc(tr("无匹配消息帧", "No matching frames"))}</div>`;
    } else {
      html += `<div class="msg-list">` + filtered.map((it, idx) => {
        const dirBadge = it.kind === "sse"
          ? `<span class="msg-dir down mono">SSE</span>`
          : `<span class="msg-dir ${it.dir === "up" ? "up" : "down"} mono">${it.dir === "up" ? "↑" : "↓"}</span>`;
        return `<div class="msg-row">`
          + dirBadge
          + `<span class="msg-kind mono">${esc(it.label || it.kind)}</span>`
          + `<div class="msg-text mono">${highlightEscaped(String(it.text || ""), q)}</div>`
          + `<button class="mini-btn" data-mcopy="${idx}" type="button" style="flex-shrink:0">${esc(tr("复制", "Copy"))}</button>`
          + `</div>`;
      }).join("") + `</div>`;
    }
    bodyEl.innerHTML = html;
    bodyEl.querySelectorAll("[data-mcopy]").forEach(btn => {
      btn.onclick = () => {
        const it = filtered[Number(btn.dataset.mcopy)];
        if (it) copyRawText(it.text || "", btn);
      };
    });
  };

  renderInner();

  head.onclick = (e) => {
    if (e.target.closest("button") || e.target.closest("input")) return;
    state.folded[foldKey] = !state.folded[foldKey];
    card.classList.toggle("folded", !!state.folded[foldKey]);
  };
  head.querySelectorAll("[data-mdir]").forEach(btn => {
    btn.onclick = (e) => { e.stopPropagation(); state.msgDir = btn.dataset.mdir; renderAll(); };
  });
  head.querySelectorAll("[data-mkind]").forEach(btn => {
    btn.onclick = (e) => { e.stopPropagation(); state.msgKind = btn.dataset.mkind; renderAll(); };
  });
  const mSearch = head.querySelector('[data-role="msearch"]');
  mSearch.onclick = (e) => e.stopPropagation();
  mSearch.oninput = () => { state.msgSearch = mSearch.value; renderInner(); };
  head.querySelector('[data-act="copy-all"]').onclick = (e) => {
    e.stopPropagation();
    copyRawText(filteredCopyText, e.currentTarget);
  };

  card.appendChild(head);
  card.appendChild(bodyEl);
  return card;
}

function renderDetail(flow, visibleFlows) {
  const pane = document.getElementById("right-pane");
  if (state.ended || !state.data || !state.data.flows) {
    pane.innerHTML = `<div class="empty-state">${esc(tr("分享已结束或已过期", "This share has ended or expired"))}</div>`;
    return;
  }
  if (!flow) {
    renderSummaryView(visibleFlows);
    return;
  }
  ensureFlowBody(flow);
  pane.innerHTML = "";

  const failed = !!flow.error && flow.status == null;
  const statusText = flow.status == null ? (failed ? "ERR" : "—") : String(flow.status);
  const pinned = isPinned(flow);
  const isBase = state.baselineId === flow.id;
  const gql = graphqlDisplayLabel(flow);
  const clientIp = flowClientIp(flow);
  const ci = flow.conn_info || null;

  let timingHtml = "";
  if (ci) {
    const connMeta = [];
    if (ci.reused != null) connMeta.push(`<span class="proto-badge">${esc(ci.reused ? tr("复用连接", "Reused") : tr("新建连接", "New conn"))}</span>`);
    if (ci.tls_version) connMeta.push(`<span class="proto-badge mono">${esc(ci.tls_version)}</span>`);
    if (ci.client_cert) {
      const certBase = String(ci.client_cert).split(/[\\/]/).pop() || ci.client_cert;
      connMeta.push(`<span class="proto-badge mono" title="${esc(ci.client_cert)}">mTLS ${esc(certBase)}</span>`);
    }
    if (ci.server_addr) connMeta.push(`<span class="mono muted-cell">${esc(ci.server_addr)}</span>`);

    const phases = [
      { lbl: "DNS", ms: ci.dns_ms, color: "var(--t-dns)" },
      { lbl: "TCP", ms: ci.tcp_ms, color: "var(--t-tcp)" },
      { lbl: "TLS", ms: ci.tls_ms, color: "var(--t-tls)" },
      { lbl: tr("等待", "Wait"), ms: ci.wait_ms, color: "var(--t-wait)" },
      { lbl: tr("下载", "Download"), ms: ci.download_ms, color: "var(--t-dl)" },
    ].filter(p => p.ms != null && Number(p.ms) >= 0);

    const sumMs = phases.reduce((a, p) => a + Math.max(0, Number(p.ms || 0)), 0);
    const barHtml = sumMs > 0
      ? `<div class="timing-bar">` + phases.filter(p => Number(p.ms) > 0).map(p =>
          `<span class="timing-seg" style="width:${((Number(p.ms) / sumMs) * 100).toFixed(2)}%;background:${p.color}" title="${esc(p.lbl)}: ${p.ms} ms"></span>`
        ).join("") + `</div>`
      : "";
    const legHtml = phases.length
      ? `<div class="timing-legend mono">` + phases.map(p =>
          `<span><span class="t-dot" style="background:${p.color}"></span>${esc(p.lbl)} ${p.ms} ms</span>`
        ).join("") + `</div>`
      : "";

    if (connMeta.length || barHtml || legHtml) {
      timingHtml = `<div class="timing-box">`
        + (connMeta.length ? `<div class="timing-meta">${connMeta.join("")}</div>` : "")
        + barHtml + legHtml
        + `</div>`;
    }
  }

  const summary = document.createElement("div");
  summary.className = "card summary-card";
  summary.innerHTML = `<div class="summary-top">`
    + `<div class="summary-badges">`
    + `<span class="m-badge mono m-${esc(flow.method)}">${esc(flow.method)}</span>`
    + `<span class="${statusBadgeClass(flow)} mono">${esc(statusText)}</span>`
    + `<span class="proto-badge mono">${esc(flow.protocol || "HTTP/1.1")}</span>`
    + (flow.relay_source ? `<span class="proto-badge mono">${esc(tr("中继 ", "Relay ") + relayShort(flow.relay_source))}</span>` : "")
    + (gql ? `<span class="gql-badge mono">gql:${esc(gql)}</span>` : "")
    + (flow.rule_hit ? `<span class="rule-badge">${esc(flow.rule_hit)}</span>` : "")
    + `</div>`
    + `<div class="card-actions">`
    + `<button class="mini-btn${pinned ? " active" : ""}" id="sum-pin-btn" type="button">${esc(pinned ? tr("★ 已星标", "★ Starred") : tr("☆ 星标", "☆ Star"))}</button>`
    + `<button class="mini-btn${isBase ? " active" : ""}" id="sum-base-btn" type="button">${esc(isBase ? tr("取消基准", "Clear base") : tr("基准", "Baseline"))}</button>`
    + `<button class="mini-btn" id="sum-more-btn" type="button">${esc(tr("更多 ▾", "More ▾"))}</button>`
    + `</div>`
    + `</div>`
    + `<div class="url-box mono">${esc(flow.url || flow.path || "/")}</div>`
    + (flow.comment ? `<div class="note-box"># ${esc(flow.comment)}</div>` : "")
    + (flow.error ? `<div class="err-box">${esc(localizeError(flow.error))}</div>` : "")
    + `<div class="meta-grid mono">`
    + `<span class="meta-item">${esc(tr("时间", "Time"))}: <b>${esc(flow.started || "—")}</b></span>`
    + (clientIp ? `<span class="meta-item">${esc(tr("来源", "Client"))}: <b>${esc(clientIp)}</b></span>` : "")
    + (flow.ttfb_ms != null ? `<span class="meta-item">TTFB: <b>${flow.ttfb_ms} ms</b></span>` : "")
    + `<span class="meta-item">${esc(tr("耗时", "Elapsed"))}: <b class="${isSlow(flow) ? "slow" : ""}">${esc(flow.elapsed || "—")}</b></span>`
    + `<span class="meta-item">${esc(tr("请求 / 响应", "Req / Res"))}: <b>${esc(formatBytes(flow.request_bytes || 0))} / ${esc(formatBytes(flow.response_bytes || 0))}</b></span>`
    + `</div>`
    + timingHtml;

  summary.querySelector("#sum-pin-btn").onclick = () => togglePin(flow);
  summary.querySelector("#sum-base-btn").onclick = () => {
    state.baselineId = isBase ? null : flow.id;
    if (state.baselineId != null) ensureFlowBody(flow);
    renderAll();
  };
  summary.querySelector("#sum-more-btn").onclick = (e) => {
    e.stopPropagation();
    openDetailMoreMenu(flow, e.currentTarget);
  };
  pane.appendChild(summary);

  const resBodyCard = createBodyCard("res", flow);
  if (resBodyCard) pane.appendChild(resBodyCard);

  if ((flow.response_headers || []).length) {
    const disp = formatHeadersForCard(flow.response_headers, state.pretty, false);
    const raw = flow.response_headers.map(([k, v]) => `${k}: ${v}`).join("\n");
    pane.appendChild(createKvCard("res_headers", tr("响应头", "Response Headers"), disp, raw));
  }

  const reqBodyCard = createBodyCard("req", flow);
  if (reqBodyCard) pane.appendChild(reqBodyCard);

  if ((flow.query || []).length) {
    const raw = flow.query.map(([k, v]) => `${k}=${v}`).join("&");
    pane.appendChild(createKvCard("query", tr("查询参数", "Query Params"), flow.query, raw));
  }

  if ((flow.request_headers || []).length) {
    const disp = formatHeadersForCard(flow.request_headers, state.pretty, true);
    const raw = flow.request_headers.map(([k, v]) => `${k}: ${v}`).join("\n");
    pane.appendChild(createKvCard("req_headers", tr("请求头", "Request Headers"), disp, raw));
  }

  const diffCard = createDiffCard(flow);
  if (diffCard) pane.appendChild(diffCard);

  const msgCard = createMessagesCard(flow);
  if (msgCard) pane.appendChild(msgCard);
}

function renderAll() {
  const all = (!state.ended && state.data && Array.isArray(state.data.flows)) ? state.data.flows : [];
  const visible = computeVisibleFlows(all);
  renderToolbar(visible.length, all.length, all);
  renderList(visible);
  const selected = state.selectedId != null ? (visible.find(f => f.id === state.selectedId) || null) : null;
  renderDetail(selected, visible);
  if (state.seqOpen) renderSequenceModal();
}

function initSplitter() {
  const leftPane = document.getElementById("left-pane");
  const splitter = document.getElementById("splitter");
  const clampW = (w) => Math.max(280, Math.min(w, Math.min(1100, Math.max(280, window.innerWidth - 340))));
  const savedW = Number(safeGetLS("zniff_share_list_w"));
  let curW = clampW(Number.isFinite(savedW) && savedW >= 280 ? savedW : 660);
  leftPane.style.width = curW + "px";

  let dragging = false;
  splitter.addEventListener("mousedown", (e) => {
    e.preventDefault();
    dragging = true;
    splitter.classList.add("dragging");
    document.body.style.cursor = "col-resize";
  });
  window.addEventListener("mousemove", (e) => {
    if (!dragging) return;
    curW = clampW(e.clientX);
    leftPane.style.width = curW + "px";
    renderList(currentVisibleFlows());
  });
  window.addEventListener("mouseup", () => {
    if (!dragging) return;
    dragging = false;
    splitter.classList.remove("dragging");
    document.body.style.cursor = "";
    safeSetLS("zniff_share_list_w", String(Math.round(curW)));
    renderList(currentVisibleFlows());
  });
  window.addEventListener("resize", () => {
    curW = clampW(curW);
    leftPane.style.width = curW + "px";
    renderList(currentVisibleFlows());
  });
}

function initHexWidthObserver() {
  const rightPane = document.getElementById("right-pane");
  const leftPane = document.getElementById("left-pane");
  const calcHexCols = (w) => w >= 980 ? 32 : 16;
  const initW = (rightPane && rightPane.clientWidth)
    || Math.max(340, window.innerWidth - ((leftPane && parseFloat(leftPane.style.width)) || 660) - 5);
  state.hexCols = calcHexCols(initW);
  if (!window.ResizeObserver || !rightPane) return;
  const ro = new window.ResizeObserver(entries => {
    for (const ent of entries) {
      const nextCols = calcHexCols(ent.contentRect.width);
      if (nextCols !== state.hexCols) {
        state.hexCols = nextCols;
        if (state.selectedId != null) renderAll();
      }
    }
  });
  ro.observe(rightPane);
}

document.getElementById("count-pill").onclick = () => selectFlowById(null);
document.getElementById("summary-btn").onclick = () => selectFlowById(null);
document.getElementById("seq-top-btn").onclick = () => openSequenceModal();
document.getElementById("pretty-btn").onclick = () => { state.pretty = !state.pretty; renderAll(); };

const mainInput = document.getElementById("filter-input");
mainInput.oninput = () => { state.filter = mainInput.value; renderAll(); };
document.getElementById("filter-clear").onclick = () => setMainFilter("");

const subInput = document.getElementById("sub-filter-input");
subInput.oninput = () => { state.listFilter = subInput.value; renderAll(); };
document.getElementById("sub-filter-clear").onclick = () => {
  state.listFilter = "";
  subInput.value = "";
  renderAll();
};
document.getElementById("merge-sub-btn").onclick = () => mergeSecondaryFilter();

document.getElementById("facet-btn").onclick = () => {
  state.facetOpen = !state.facetOpen;
  renderAll();
};
document.getElementById("col-btn").onclick = (e) => {
  e.stopPropagation();
  const rect = e.currentTarget.getBoundingClientRect();
  openColumnsMenu(rect.right - 210, rect.bottom + 4);
};

["col-menu", "ctx-menu", "more-menu"].forEach(id => {
  document.getElementById(id).addEventListener("click", e => e.stopPropagation());
});
document.addEventListener("click", () => closeAllMenus());
document.getElementById("seq-modal").addEventListener("click", (e) => {
  if (e.target.id === "seq-modal") {
    state.seqOpen = false;
    e.currentTarget.classList.remove("open");
  }
});

window.addEventListener("keydown", (e) => {
  const tag = document.activeElement ? document.activeElement.tagName : "";
  const inInput = tag === "INPUT" || tag === "TEXTAREA";
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
    e.preventDefault();
    mainInput.focus();
    mainInput.select();
    return;
  }
  if (e.key === "Escape") {
    if (document.querySelector(".pop-menu.open")) { closeAllMenus(); return; }
    if (state.seqOpen) {
      state.seqOpen = false;
      document.getElementById("seq-modal").classList.remove("open");
      return;
    }
    if (inInput) { document.activeElement.blur(); return; }
    if (state.selectedId != null) { selectFlowById(null); return; }
    return;
  }
  if (inInput || state.seqOpen) return;
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    const all = (!state.ended && state.data && Array.isArray(state.data.flows)) ? state.data.flows : [];
    const vis = computeVisibleFlows(all);
    if (!vis.length) return;
    e.preventDefault();
    const idx = vis.findIndex(f => f.id === state.selectedId);
    const nextIdx = idx < 0
      ? (e.key === "ArrowDown" ? 0 : vis.length - 1)
      : Math.max(0, Math.min(vis.length - 1, idx + (e.key === "ArrowDown" ? 1 : -1)));
    selectFlowById(vis[nextIdx].id);
    const row = document.querySelector(`#table-body tr[data-id="${vis[nextIdx].id}"]`);
    if (row) row.scrollIntoView({ block: "nearest" });
  }
});

function sameIdList(a, b) {
  if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

function mergeLoadedSides(oldFlow, f) {
  if (!oldFlow || !oldFlow._bodyLoaded) return;
  if (f.request_body_spilled) {
    const reqKeys = [
      "request_body", "request_body_pretty", "request_body_kind", "request_form_rows",
      "request_hex_b64", "request_raw_len", "request_truncated", "request_expired",
      "request_encoding_error", "graphql_op"
    ];
    for (const k of reqKeys) {
      if (oldFlow[k] !== undefined) f[k] = oldFlow[k];
    }
    f.request_body_spilled = false;
  }
  if (f.response_body_spilled) {
    const resKeys = [
      "response_body", "response_body_pretty", "response_body_kind",
      "response_hex_b64", "response_raw_len", "response_truncated", "response_expired",
      "response_encoding_error", "response_image_data_url", "response_image_dims",
      "messages", "message_items", "messages_truncated"
    ];
    for (const k of resKeys) {
      if (oldFlow[k] !== undefined) f[k] = oldFlow[k];
    }
    f.response_body_spilled = false;
  }
  f._bodyLoaded = !f.request_body_spilled && !f.response_body_spilled;
}

function applyShareUpdate(prevData, next) {
  const prev = (prevData && Array.isArray(prevData.flows)) ? prevData : null;
  const prevIds = prev ? (Array.isArray(prev.ids) ? prev.ids : prev.flows.map(f => f.id)) : [];
  const nextIds = Array.isArray(next.ids) ? next.ids : (Array.isArray(next.flows) ? next.flows.map(f => f.id) : []);

  if (prev && next.incremental && (!next.flows || next.flows.length === 0)
      && sameIdList(prevIds, nextIds) && (next.omitted || 0) === (prev.omitted || 0)
      && !!next.redacted === !!prev.redacted) {
    if (next.rev != null) prev.rev = next.rev;
    if (next.expires != null) prev.expires = next.expires;
    return { data: prev, changed: false, needFullSync: false };
  }
  if (prev && !next.incremental && next.rev != null && next.rev === prev.rev
      && sameIdList(prevIds, nextIds) && (next.omitted || 0) === (prev.omitted || 0)
      && !!next.redacted === !!prev.redacted) {
    if (next.expires != null) prev.expires = next.expires;
    return { data: prev, changed: false, needFullSync: false };
  }

  let mergedFlows = [];
  let needFullSync = false;
  if (next.incremental && prev) {
    const map = new Map(prev.flows.map(f => [f.id, f]));
    for (const f of (next.flows || [])) {
      mergeLoadedSides(map.get(f.id), f);
      map.set(f.id, f);
    }
    if (Array.isArray(next.ids)) {
      needFullSync = next.ids.some(id => !map.has(id));
      mergedFlows = next.ids.map(id => map.get(id)).filter(Boolean);
    } else {
      mergedFlows = Array.from(map.values()).sort((a, b) => a.id - b.id);
    }
  } else {
    const prevMap = new Map(prev ? prev.flows.map(f => [f.id, f]) : []);
    const inFlows = Array.isArray(next.flows) ? next.flows : [];
    for (const f of inFlows) mergeLoadedSides(prevMap.get(f.id), f);
    if (Array.isArray(next.ids)) {
      const inMap = new Map(inFlows.map(f => [f.id, f]));
      mergedFlows = next.ids.map(id => inMap.get(id)).filter(Boolean);
    } else {
      mergedFlows = inFlows;
    }
  }

  const liveIds = new Set(mergedFlows.map(f => f.id));
  if (state.selectedId != null && !liveIds.has(state.selectedId)) state.selectedId = null;
  if (state.baselineId != null && !liveIds.has(state.baselineId)) state.baselineId = null;
  for (const k of Array.from(state.localPins.keys())) {
    if (!liveIds.has(k)) state.localPins.delete(k);
  }

  const outData = Object.assign({}, next, {
    ids: Array.isArray(next.ids) ? next.ids : mergedFlows.map(f => f.id),
    flows: mergedFlows
  });
  return { data: outData, changed: true, needFullSync };
}

initSplitter();
initHexWidthObserver();
renderAll();

if (LIVE) {
  let timer = null;
  const poll = async (forceFull) => {
    try {
      const sep = LIVE.includes("?") ? "&" : "?";
      const curRev = (!forceFull && state.data && state.data.rev) ? state.data.rev : 0;
      const url = curRev ? `${LIVE}${sep}since=${curRev}` : LIVE;
      const res = await fetch(url, { cache: "no-store" });
      if (res.status === 404 || res.status === 410) {
        if (!state.ended) {
          state.ended = true;
          renderAll();
        }
        clearInterval(timer);
        return;
      }
      if (!res.ok) return;
      const next = await res.json();
      const upd = applyShareUpdate(state.data, next);
      state.data = upd.data;
      if (upd.needFullSync && !forceFull) {
        await poll(true);
        return;
      }
      if (upd.changed) renderAll();
    } catch (_) {}
  };
  timer = setInterval(() => poll(false), 2000);
}
</script>
</body>
</html>
"##;

pub fn expired_page(zh: bool) -> String {
    let title = if zh { "分享已结束" } else { "Share ended" };
    let body = if zh {
        "这个分享地址已经停止，或超过了有效期。"
    } else {
        "This share link was stopped, or it is past its expiry."
    };
    format!(
        "<!DOCTYPE html><html lang=\"{lang}\"><head><meta charset=\"utf-8\"><link rel=\"icon\" href=\"data:,\"><title>{title}</title></head><body style=\"background:#0d1017;color:#e2e8f0;font:16px sans-serif;padding:24px\"><h1>{title}</h1><p>{body}</p></body></html>",
        lang = if zh { "zh-CN" } else { "en" },
    )
}

pub fn prefers_zh(accept_language: Option<&str>) -> bool {
    let primary = accept_language
        .unwrap_or("")
        .split(',')
        .next()
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    primary.is_empty() || primary.to_ascii_lowercase().starts_with("zh")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_paths_accept_only_the_id() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        assert!(valid_id(id));
        assert_eq!(
            parse_share_path(&format!("/s/{id}")),
            Some((id.into(), SharePath::Page))
        );
        assert_eq!(
            parse_share_path(&format!("/s/{id}/flows.json")),
            Some((id.into(), SharePath::Data { since: None }))
        );
        assert_eq!(
            parse_share_path(&format!("/s/{id}/flows.json?since=42")),
            Some((id.into(), SharePath::Data { since: Some(42) }))
        );
        assert_eq!(
            parse_share_path(&format!("/s/{id}/flows/7.json")),
            Some((id.into(), SharePath::FlowBody(7)))
        );
        assert!(parse_share_path("/s/not-a-uuid").is_none());
        assert!(parse_share_path("/zniff").is_none());
        assert!(parse_share_path(&format!("http://example.com/s/{id}")).is_none());
    }

    #[test]
    fn grant_ends_at_the_deadline() {
        let share = LiveShare {
            id: "550e8400-e29b-41d4-a716-446655440000".into(),
            expires: UNIX_EPOCH + std::time::Duration::from_secs(100),
            filter: "header:cookie".into(),
            redact: RedactOptions::default(),
        };
        assert!(grant_ok(
            &share,
            &share.id,
            UNIX_EPOCH + std::time::Duration::from_secs(99)
        ));
        assert!(!grant_ok(
            &share,
            &share.id,
            UNIX_EPOCH + std::time::Duration::from_secs(100)
        ));
        assert!(!grant_ok(&share, "other", UNIX_EPOCH));
    }

    #[test]
    fn page_escapes_a_script_in_the_body() {
        let json = document_from_body("</script><script>alert(1)</script>");
        let html = page(&json, "", true);
        assert!(!html.contains("</script><script>"), "{html}");
        assert!(html.contains("\\u003c"));
    }

    fn document_from_body(body: &str) -> String {
        let doc = ShareDoc {
            expires: 0,
            filter: String::new(),
            redacted: false,
            omitted: 0,
            rev: 1,
            incremental: false,
            ids: vec![1],
            flows: vec![ShareFlow {
                id: 1,
                method: "GET".into(),
                status: Some(200),
                protocol: "HTTP/1.1".into(),
                host: "example.test".into(),
                path: "/".into(),
                url: "http://example.test/".into(),
                started: "00:00:00.000".into(),
                gap: "0 ms".into(),
                elapsed: "10 ms".into(),
                size: "12 B".into(),
                response_bytes: 12,
                ttfb_ms: Some(4),
                total_ms: Some(10),
                request_headers: vec![("Cookie".into(), "<script>".into())],
                response_body: body.into(),
                ..ShareFlow::default()
            }],
        };
        serde_json::to_string(&doc).unwrap()
    }

    #[test]
    fn redact_flow_sanitizes_headers_query_json_form_and_flags_binary() {
        let mut session = crate::session::Session::default();
        let id = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "POST".into(),
            host: "api.example.test".into(),
            path: "/v1/login?access_token=tok123&page=2&custom_sec=abc".into(),
            url: "https://api.example.test/v1/login?access_token=tok123&page=2&custom_sec=abc"
                .into(),
            request_headers: vec![
                ("Authorization".into(), "Bearer eyJhbGciOiJIUzI1NiJ9".into()),
                ("Cookie".into(), "sid=s1; theme=dark".into()),
                ("X-Custom-Header".into(), "my-secret-header".into()),
                ("Content-Type".into(), "application/json".into()),
            ],
        });
        session.append_body(
            id,
            Direction::Request,
            br#"{"username":"alice","password":"p@ssw0rd","nested":{"custom_sec":"xyz","ok":1}}"#,
        );
        session.set_response(
            id,
            200,
            vec![
                (
                    "Set-Cookie".into(),
                    "session_id=abc999; Path=/; HttpOnly; Secure".into(),
                ),
                ("Content-Type".into(), "application/octet-stream".into()),
            ],
            None,
        );
        session.append_body(id, Direction::Response, &[0xff, 0x00, 0xfe, 0x01]);
        session.mark_finished(id);

        let original = session.hydrated(id).unwrap();
        let opts = RedactOptions::new(true, "custom_sec, X-Custom-Header");
        let (sanitized, report) = redact_flow(&original, &opts);

        // Original capture in session is untouched
        let untouched = session.hydrated(id).unwrap();
        assert!(untouched.url.contains("tok123"));

        // Query parameters
        assert_eq!(
            sanitized.url,
            "https://api.example.test/v1/login?access_token=[REDACTED]&page=2&custom_sec=[REDACTED]"
        );
        assert_eq!(
            sanitized.path,
            "/v1/login?access_token=[REDACTED]&page=2&custom_sec=[REDACTED]"
        );
        assert_eq!(report.redacted_query_params, 2);

        // Headers
        assert_eq!(sanitized.request_headers[0].1, "Bearer [REDACTED]");
        assert_eq!(
            sanitized.request_headers[1].1,
            "sid=[REDACTED]; theme=[REDACTED]"
        );
        assert_eq!(sanitized.request_headers[2].1, "[REDACTED]");
        assert_eq!(
            sanitized.response_headers[0].1,
            "session_id=[REDACTED]; Path=/; HttpOnly; Secure"
        );
        assert_eq!(report.redacted_headers, 4);

        // Request JSON body
        let body_json: serde_json::Value = serde_json::from_slice(&sanitized.request_body).unwrap();
        assert_eq!(body_json["username"], "alice");
        assert_eq!(body_json["password"], "[REDACTED]");
        assert_eq!(body_json["nested"]["custom_sec"], "[REDACTED]");
        assert_eq!(body_json["nested"]["ok"], 1);
        assert_eq!(report.redacted_body_fields, 2);

        // Binary response body flagged as unsanitizable
        assert!(!report.unsanitizable_req_body);
        assert!(report.unsanitizable_res_body);
        assert!(
            sanitized
                .comment
                .contains("[REDACTED; res body format not sanitizable]"),
            "comment={}",
            sanitized.comment
        );
    }

    #[test]
    fn redact_json_cookie_casing_and_header_keys() {
        let opts = RedactOptions::new(true, "");
        let mut val: serde_json::Value = serde_json::from_str(
            r#"{
                "cookie": "sid=123",
                "Cookie": "sid=456",
                "COOKIE": "sid=789",
                "set-cookie": "session=abc; Secure",
                "Set-Cookie": "session=def; HttpOnly",
                "set_cookie": "session=ghi",
                "setCookie": "session=jkl",
                "cookies": ["a", "b"],
                "accessToken": "secret_tok",
                "access-token": "secret_tok_2",
                "clientSecret": "client_sec",
                "x-api-key": "my_api_key",
                "csrfToken": "csrf_val",
                "username": "john_doe"
            }"#,
        )
        .unwrap();

        let count = redact_json_value(&mut val, &opts);
        assert_eq!(count, 13);
        assert_eq!(val["cookie"], "[REDACTED]");
        assert_eq!(val["Cookie"], "[REDACTED]");
        assert_eq!(val["COOKIE"], "[REDACTED]");
        assert_eq!(val["set-cookie"], "[REDACTED]");
        assert_eq!(val["Set-Cookie"], "[REDACTED]");
        assert_eq!(val["set_cookie"], "[REDACTED]");
        assert_eq!(val["setCookie"], "[REDACTED]");
        assert_eq!(val["cookies"], "[REDACTED]");
        assert_eq!(val["accessToken"], "[REDACTED]");
        assert_eq!(val["access-token"], "[REDACTED]");
        assert_eq!(val["clientSecret"], "[REDACTED]");
        assert_eq!(val["x-api-key"], "[REDACTED]");
        assert_eq!(val["csrfToken"], "[REDACTED]");
        assert_eq!(val["username"], "john_doe");
    }

    #[test]
    fn redact_json_url_query_secrets_and_nested_urls() {
        let opts = RedactOptions::new(true, "");
        let mut val: serde_json::Value = serde_json::from_str(
            r#"{
                "url": "https://api.example.com/oauth?token=xyz123&secret=abc456&client_id=my_app",
                "path": "/v1/auth?access_token=secret_tok&user=alice",
                "links": [
                    "https://cdn.example.com/file?api_key=secretkey&download=1"
                ],
                "oauth_redirect": "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Ftoken%3Dsecret_nested"
            }"#,
        )
        .unwrap();

        let count = redact_json_value(&mut val, &opts);
        assert_eq!(count, 5);
        assert_eq!(
            val["url"],
            "https://api.example.com/oauth?token=[REDACTED]&secret=[REDACTED]&client_id=my_app"
        );
        assert_eq!(val["path"], "/v1/auth?access_token=[REDACTED]&user=alice");
        assert_eq!(
            val["links"][0],
            "https://cdn.example.com/file?api_key=[REDACTED]&download=1"
        );
        assert_eq!(
            val["oauth_redirect"],
            "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Ftoken%3D%5BREDACTED%5D"
        );
    }

    #[test]
    fn redact_percent_encoded_nested_redirect_urls() {
        let opts = RedactOptions::new(true, "");
        let mut val: serde_json::Value = serde_json::from_str(
            r#"{
                "standard": "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Fsecret%3Dnested_sec%26ok%3D1",
                "lowercase_hex": "https://auth.com/login?redirect=https%3a%2f%2fapp.com%2fcb%3fsecret%3dnested_sec%26ok%3d1",
                "multi_secrets": "https://auth.com/login?client_id=123&redirect=https%3A%2F%2Fapp.com%2Fcb%3Ftoken%3Dtok_nested%26password%3Dpass_nested%26user%3Dalice&state=xyz",
                "with_fragment": "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Fapi_key%3Dmykey%26step%3D2%23details#outer_frag",
                "standalone_params": "https://api.com/proxy?forward=secret%3Dtopsecret%26client%3Dweb",
                "relative_redirect": "https://app.com/login?next=%2Fdashboard%3Fapi_key%3Dmy_api_key%26tab%3D1"
            }"#,
        )
        .unwrap();

        let count = redact_json_value(&mut val, &opts);
        assert_eq!(count, 7);
        assert_eq!(
            val["standard"],
            "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Fsecret%3D%5BREDACTED%5D%26ok%3D1"
        );
        assert_eq!(
            val["lowercase_hex"],
            "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Fsecret%3D%5BREDACTED%5D%26ok%3D1"
        );
        assert_eq!(
            val["multi_secrets"],
            "https://auth.com/login?client_id=123&redirect=https%3A%2F%2Fapp.com%2Fcb%3Ftoken%3D%5BREDACTED%5D%26password%3D%5BREDACTED%5D%26user%3Dalice&state=xyz"
        );
        assert_eq!(
            val["with_fragment"],
            "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Fapi_key%3D%5BREDACTED%5D%26step%3D2%23details#outer_frag"
        );
        assert_eq!(
            val["standalone_params"],
            "https://api.com/proxy?forward=secret%3D%5BREDACTED%5D%26client%3Dweb"
        );
        assert_eq!(
            val["relative_redirect"],
            "https://app.com/login?next=%2Fdashboard%3Fapi_key%3D%5BREDACTED%5D%26tab%3D1"
        );

        // Pass 2 Idempotency verification
        let count2 = redact_json_value(&mut val, &opts);
        assert_eq!(count2, 0);
        assert_eq!(
            val["standard"],
            "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Fsecret%3D%5BREDACTED%5D%26ok%3D1"
        );
        assert_eq!(
            val["with_fragment"],
            "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Fapi_key%3D%5BREDACTED%5D%26step%3D2%23details#outer_frag"
        );
    }

    #[test]
    fn redact_json_stringified_nested_json() {
        let opts = RedactOptions::new(true, "");
        let mut val: serde_json::Value = serde_json::from_str(
            r#"{
                "event": "login",
                "body": "{\"token\":\"abc12345\",\"password\":\"hunter2\",\"user\":\"alice\"}",
                "nested": "{\"redirect\":\"https://service.internal/oauth?secret=nested_sec&ok=true\"}"
            }"#,
        )
        .unwrap();

        let count = redact_json_value(&mut val, &opts);
        assert_eq!(count, 3);

        // Unpack inner body and verify
        let inner_body: serde_json::Value =
            serde_json::from_str(val["body"].as_str().unwrap()).unwrap();
        assert_eq!(inner_body["token"], "[REDACTED]");
        assert_eq!(inner_body["password"], "[REDACTED]");
        assert_eq!(inner_body["user"], "alice");

        // Unpack inner nested url and verify
        let inner_nested: serde_json::Value =
            serde_json::from_str(val["nested"].as_str().unwrap()).unwrap();
        assert_eq!(
            inner_nested["redirect"],
            "https://service.internal/oauth?secret=[REDACTED]&ok=true"
        );
    }

    #[test]
    fn redact_json_malformed_strings_and_robustness() {
        let opts = RedactOptions::new(true, "");
        let mut val: serde_json::Value = serde_json::from_str(
            r#"{
                "broken_json": "{\"token\": incomplete",
                "not_url": "How are you? I'm fine.",
                "broken_url": "https://example.com/??==&token=xyz",
                "empty": ""
            }"#,
        )
        .unwrap();

        // Must not panic
        let count = redact_json_value(&mut val, &opts);
        assert_eq!(val["broken_json"], "{\"token\": incomplete");
        assert_eq!(val["not_url"], "How are you? I'm fine.");
        assert_eq!(val["empty"], "");
        assert!(
            val["broken_url"]
                .as_str()
                .unwrap()
                .contains("token=[REDACTED]")
        );
        assert_eq!(count, 1);
    }

    #[test]
    fn redact_flow_idempotent() {
        let mut session = crate::session::Session::default();
        let id = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "POST".into(),
            host: "api.example.test".into(),
            path: "/api?token=secret".into(),
            url: "https://api.example.test/api?token=secret".into(),
            request_headers: vec![("Cookie".into(), "sid=secret123".into())],
        });
        session.append_body(id, Direction::Request, br#"{"password":"secret"}"#);
        session.set_response(id, 200, vec![], None);
        session.mark_finished(id);

        let flow = session.hydrated(id).unwrap();
        let opts = RedactOptions::new(true, "");
        let (sanitized1, report1) = redact_flow(&flow, &opts);
        assert_eq!(report1.total_redacted(), 3);

        // Second pass on already-redacted flow
        let (sanitized2, report2) = redact_flow(&sanitized1, &opts);
        assert_eq!(sanitized1.url, sanitized2.url);
        assert_eq!(sanitized1.request_body, sanitized2.request_body);
        assert_eq!(report2.total_redacted(), 0);
    }

    #[test]
    fn redact_flow_and_json_idempotency_with_fragments_multi_pass() {
        let mut session = crate::session::Session::default();
        let target_url = "https://api.example.com:8443/auth/v2?user=alice&token=tok123&env=prod&cOoKiE=sess456#tab1?extra=ignored";
        let target_path =
            "/auth/v2?user=alice&token=tok123&env=prod&cOoKiE=sess456#tab1?extra=ignored";

        let id = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "POST".into(),
            host: "api.example.com".into(),
            path: target_path.into(),
            url: target_url.into(),
            request_headers: vec![
                ("Cookie".into(), "sid=secret123; user=alice".into()),
                ("Authorization".into(), "Bearer my_jwt_token".into()),
            ],
        });
        session.append_body(
            id,
            Direction::Request,
            br#"{
                "callback": "https://api.example.com:8443/auth/v2?user=alice&token=tok123&env=prod&cOoKiE=sess456#tab1?extra=ignored",
                "nested_endpoint": "https://service.internal/path?api_key=secretkey&page=1#heading?section=sec2",
                "plain_query": "token=tok999&user=bob"
            }"#,
        );
        session.set_response(id, 200, vec![], None);
        session.mark_finished(id);

        let flow = session.hydrated(id).unwrap();
        let opts = RedactOptions::new(true, "");

        // --- Pass 1: First redaction pass ---
        let (pass1_flow, rep1) = redact_flow(&flow, &opts);
        // Query params in URL & path (token + cOoKiE = 2 each; report counts URL query params = 2)
        assert_eq!(rep1.redacted_query_params, 2);
        // Headers: Cookie sid + Authorization = 2
        assert_eq!(rep1.redacted_headers, 2);
        // Body fields: callback (token + cOoKiE = 2), nested_endpoint (api_key = 1), plain_query (token = 1) -> 4
        assert_eq!(rep1.redacted_body_fields, 4);
        assert_eq!(rep1.total_redacted(), 8);

        // Verify URL & Path: sensitive parameters redacted, fragments strictly preserved
        assert_eq!(
            pass1_flow.url,
            "https://api.example.com:8443/auth/v2?user=alice&token=[REDACTED]&env=prod&cOoKiE=[REDACTED]#tab1?extra=ignored"
        );
        assert_eq!(
            pass1_flow.path,
            "/auth/v2?user=alice&token=[REDACTED]&env=prod&cOoKiE=[REDACTED]#tab1?extra=ignored"
        );

        // Verify JSON body
        let parsed1: serde_json::Value = serde_json::from_slice(&pass1_flow.request_body).unwrap();
        assert_eq!(
            parsed1["callback"],
            "https://api.example.com:8443/auth/v2?user=alice&token=[REDACTED]&env=prod&cOoKiE=[REDACTED]#tab1?extra=ignored"
        );
        assert_eq!(
            parsed1["nested_endpoint"],
            "https://service.internal/path?api_key=[REDACTED]&page=1#heading?section=sec2"
        );
        assert_eq!(parsed1["plain_query"], "token=[REDACTED]&user=bob");

        // --- Pass 2: Redacting already-redacted flow ---
        let (pass2_flow, rep2) = redact_flow(&pass1_flow, &opts);
        assert_eq!(rep2.total_redacted(), 0, "Pass 2 must produce 0 redactions");
        assert_eq!(rep2.redacted_query_params, 0);
        assert_eq!(rep2.redacted_headers, 0);
        assert_eq!(rep2.redacted_body_fields, 0);

        // Exact byte-for-byte and string identity between Pass 1 and Pass 2
        assert_eq!(pass2_flow.url, pass1_flow.url);
        assert_eq!(pass2_flow.path, pass1_flow.path);
        assert_eq!(pass2_flow.request_headers, pass1_flow.request_headers);
        assert_eq!(pass2_flow.request_body, pass1_flow.request_body);

        let parsed2: serde_json::Value = serde_json::from_slice(&pass2_flow.request_body).unwrap();
        assert_eq!(parsed2["callback"], parsed1["callback"]);
        assert!(
            parsed2["callback"]
                .as_str()
                .unwrap()
                .ends_with("#tab1?extra=ignored"),
            "Fragment was destroyed on Pass 2!"
        );

        // --- Pass 3: Verification of stability across 3 passes ---
        let (pass3_flow, rep3) = redact_flow(&pass2_flow, &opts);
        assert_eq!(rep3.total_redacted(), 0, "Pass 3 must produce 0 redactions");
        assert_eq!(pass3_flow.url, pass2_flow.url);
        assert_eq!(pass3_flow.path, pass2_flow.path);
        assert_eq!(pass3_flow.request_headers, pass2_flow.request_headers);
        assert_eq!(pass3_flow.request_body, pass2_flow.request_body);
    }

    #[test]
    fn redact_plain_unencoded_nested_urls_and_mixed_params() {
        let opts = RedactOptions::new(true, "");
        let mut val: serde_json::Value = serde_json::from_str(
            r#"{
                "single_nested": "https://auth.com/login?redirect=https://app.com/cb?token=secret",
                "mixed_outer_inner": "https://auth.com/login?app_id=42&redirect=https://app.com/cb?token=secret&env=staging",
                "nested_with_frag": "https://auth.com/login?redirect=https://app.com/cb?token=secret#section1?extra=ignored"
            }"#,
        )
        .unwrap();

        let count = redact_json_value(&mut val, &opts);
        assert_eq!(count, 3);

        // Verification of single nested URL
        let single = val["single_nested"].as_str().unwrap().to_string();
        assert!(!single.contains("secret"));
        assert_eq!(
            single,
            "https://auth.com/login?redirect=https://app.com/cb?token=[REDACTED]"
        );

        // Verification of mixed outer and inner query parameters
        let mixed = val["mixed_outer_inner"].as_str().unwrap().to_string();
        assert!(!mixed.contains("secret"));
        assert!(mixed.contains("app_id=42"));
        assert!(mixed.contains("env=staging"));
        assert_eq!(
            mixed,
            "https://auth.com/login?app_id=42&redirect=https://app.com/cb?token=[REDACTED]&env=staging"
        );

        // Verification of nested URL with fragment
        let with_frag = val["nested_with_frag"].as_str().unwrap().to_string();
        assert!(!with_frag.contains("secret"));
        assert_eq!(
            with_frag,
            "https://auth.com/login?redirect=https://app.com/cb?token=[REDACTED]#section1?extra=ignored"
        );

        // Idempotency on Pass 2 and Pass 3
        let count2 = redact_json_value(&mut val, &opts);
        assert_eq!(count2, 0);
        assert_eq!(val["single_nested"], single);
        assert_eq!(val["mixed_outer_inner"], mixed);
        assert_eq!(val["nested_with_frag"], with_frag);

        let count3 = redact_json_value(&mut val, &opts);
        assert_eq!(count3, 0);
    }

    #[test]
    fn redact_query_param_permutations_and_edge_cases() {
        let opts = RedactOptions::new(true, "");

        // 1. Sensitive at start, middle, end
        let (out1, c1) = redact_url_query("https://example.com/api?token=sec1&keep=1", &opts);
        assert_eq!(c1, 1);
        assert_eq!(out1, "https://example.com/api?token=[REDACTED]&keep=1");

        let (out2, c2) = redact_url_query("https://example.com/api?keep=1&token=sec2", &opts);
        assert_eq!(c2, 1);
        assert_eq!(out2, "https://example.com/api?keep=1&token=[REDACTED]");

        let (out3, c3) = redact_url_query("https://example.com/api?a=1&password=sec3&b=2", &opts);
        assert_eq!(c3, 1);
        assert_eq!(out3, "https://example.com/api?a=1&password=[REDACTED]&b=2");

        // 2. Contiguous sensitive parameters
        let (out4, c4) = redact_url_query(
            "https://example.com/api?access_token=t1&refresh_token=t2&secret=s3",
            &opts,
        );
        assert_eq!(c4, 3);
        assert_eq!(
            out4,
            "https://example.com/api?access_token=[REDACTED]&refresh_token=[REDACTED]&secret=[REDACTED]"
        );

        // 3. Casing and delimiter variants in query string
        let (out5, c5) = redact_url_query(
            "https://example.com/api?X-Api-Key=k1&cOoKiE=c1&clientSecret=s1&session-id=sid1",
            &opts,
        );
        assert_eq!(c5, 4);
        assert_eq!(
            out5,
            "https://example.com/api?X-Api-Key=[REDACTED]&cOoKiE=[REDACTED]&clientSecret=[REDACTED]&session-id=[REDACTED]"
        );

        // 4. Empty value in sensitive parameter
        let (out6, c6) = redact_url_query("https://example.com/api?token=&user=alice", &opts);
        assert_eq!(c6, 1);
        assert_eq!(out6, "https://example.com/api?token=[REDACTED]&user=alice");

        // 5. Flag parameters without '='
        let (out7, c7) =
            redact_url_query("https://example.com/api?flag1&token=secret&flag2", &opts);
        assert_eq!(c7, 1);
        assert_eq!(out7, "https://example.com/api?flag1&token=[REDACTED]&flag2");

        // 6. Duplicate keys
        let (out8, c8) =
            redact_url_query("https://example.com/api?token=s1&user=bob&token=s2", &opts);
        assert_eq!(c8, 2);
        assert_eq!(
            out8,
            "https://example.com/api?token=[REDACTED]&user=bob&token=[REDACTED]"
        );

        // 7. Percent-encoded sensitive key (%74%6f%6b%65%6e == "token")
        let (out9, c9) = redact_url_query(
            "https://example.com/api?%74%6f%6b%65%6e=secret&other=val",
            &opts,
        );
        assert_eq!(c9, 1);
        assert_eq!(
            out9,
            "https://example.com/api?%74%6f%6b%65%6e=[REDACTED]&other=val"
        );

        // Idempotency: redacting output again produces 0 count and exact same string
        for out in [
            &out1, &out2, &out3, &out4, &out5, &out6, &out7, &out8, &out9,
        ] {
            let (pass2, count2) = redact_url_query(out, &opts);
            assert_eq!(count2, 0);
            assert_eq!(&pass2, out);
        }
    }

    #[test]
    fn redact_form_urlencoded_request_body() {
        let mut session = crate::session::Session::default();
        let id = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "POST".into(),
            host: "auth.example.com".into(),
            path: "/oauth/token".into(),
            url: "https://auth.example.com/oauth/token".into(),
            request_headers: vec![(
                "Content-Type".into(),
                "application/x-www-form-urlencoded; charset=utf-8".into(),
            )],
        });
        session.append_body(
            id,
            Direction::Request,
            b"grant_type=password&username=alice&password=my_secret_pw&client_secret=sec_val&refresh_token=rt_999&scope=read",
        );
        session.set_response(id, 200, vec![], None);
        session.mark_finished(id);

        let flow = session.hydrated(id).unwrap();
        let opts = RedactOptions::new(true, "");

        let (sanitized1, rep1) = redact_flow(&flow, &opts);
        assert_eq!(rep1.redacted_body_fields, 3);
        assert!(!rep1.unsanitizable_req_body);

        let body_str = std::str::from_utf8(&sanitized1.request_body).unwrap();
        assert!(!body_str.contains("my_secret_pw"));
        assert!(!body_str.contains("sec_val"));
        assert!(!body_str.contains("rt_999"));
        assert_eq!(
            body_str,
            "grant_type=password&username=alice&password=[REDACTED]&client_secret=[REDACTED]&refresh_token=[REDACTED]&scope=read"
        );

        // Pass 2 idempotency
        let (sanitized2, rep2) = redact_flow(&sanitized1, &opts);
        assert_eq!(rep2.total_redacted(), 0);
        assert_eq!(sanitized2.request_body, sanitized1.request_body);
    }

    #[test]
    fn redact_sse_event_stream_sanitizes_data_before_extracting_messages() {
        let mut session = crate::session::Session::default();
        let id = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http2,
            method: "GET".into(),
            host: "stream.example.com".into(),
            path: "/v1/events".into(),
            url: "https://stream.example.com/v1/events".into(),
            request_headers: vec![],
        });
        session.set_response(
            id,
            200,
            vec![(
                "Content-Type".into(),
                "text/event-stream; charset=utf-8".into(),
            )],
            None,
        );
        let sse_raw = concat!(
            "event: auth\n",
            "id: 1\n",
            "data: {\"access_token\":\"tok_live_123\",\"user\":\"alice\"}\n\n",
            "event: redirect\n",
            "data: https://app.example.com/cb?api_key=key_999&ok=1\n\n",
            "event: multiline\n",
            "data: {\n",
            "data:   \"secret\": \"multi_sec\",\n",
            "data:   \"step\": 2\n",
            "data: }\n\n",
            "data: [DONE]\n\n"
        );
        session.append_body(id, Direction::Response, sse_raw.as_bytes());
        session.mark_finished(id);

        let flow = session.hydrated(id).unwrap();
        let opts = RedactOptions::new(true, "");
        let (sanitized, rep) = redact_flow(&flow, &opts);
        assert_eq!(rep.redacted_body_fields, 3);
        assert!(!rep.unsanitizable_res_body);

        let body_text = std::str::from_utf8(&sanitized.response_body).unwrap();
        assert!(!body_text.contains("tok_live_123"), "{body_text}");
        assert!(!body_text.contains("key_999"), "{body_text}");
        assert!(!body_text.contains("multi_sec"), "{body_text}");
        assert!(body_text.contains("data: [DONE]"), "{body_text}");

        // Extracted SSE messages in share JSON & body doc must already be redacted.
        let doc_json =
            document_with_redact(std::slice::from_ref(&flow), SystemTime::now(), "", &opts);
        assert!(!doc_json.contains("tok_live_123"), "{doc_json}");
        assert!(!doc_json.contains("key_999"), "{doc_json}");
        assert!(!doc_json.contains("multi_sec"), "{doc_json}");
        let parsed_doc: InboundShare = parse_inbound_share(doc_json.as_bytes()).unwrap();
        assert_eq!(parsed_doc.flows.len(), 1);
        assert!(
            parsed_doc.flows[0]
                .messages
                .iter()
                .any(|m| m.contains("[REDACTED]") && m.contains("alice")),
            "messages: {:?}",
            parsed_doc.flows[0].messages
        );

        let body_doc = flow_body_document_with_redact(&flow, &opts);
        assert!(!body_doc.contains("tok_live_123"), "{body_doc}");
        assert!(!body_doc.contains("multi_sec"), "{body_doc}");

        // Idempotency on Pass 2
        let (sanitized2, rep2) = redact_flow(&sanitized, &opts);
        assert_eq!(rep2.total_redacted(), 0);
        assert_eq!(sanitized2.response_body, sanitized.response_body);
    }

    #[test]
    fn share_page_js_supports_note_graphql_op_messages_body_and_comment_search() {
        let html = page("{\"flows\":[]}", "", false);
        assert!(html.contains("function extractGraphqlOps(flow) {"));
        assert!(html.contains("tok.startsWith(\"note:\") || tok.startsWith(\"comment:\")"));
        assert!(html.contains(
            "tok.startsWith(\"op:\") || tok.startsWith(\"gql:\") || tok.startsWith(\"operation:\")"
        ));
        assert!(html.contains("flow.messages || []"));
        assert!(html.contains("host.includes(p) || path.includes(p) || comment.includes(p)"));
    }

    #[test]
    fn redact_options_parses_at_strict_token_and_with_strict() {
        let opts = RedactOptions::new(true, "phone, @strict, X-Custom-Sec");
        assert!(opts.enabled);
        assert!(opts.strict);
        assert_eq!(opts.extra_fields, vec!["phone", "x-custom-sec"]);
        assert!(!opts.is_sensitive_field("@strict"));
        assert!(opts.is_sensitive_field("phone"));
        assert!(opts.is_sensitive_header("x-custom-sec"));

        let roundtrip = RedactOptions::new(true, &opts.to_wire_fields());
        assert_eq!(roundtrip, opts);

        let via_builder = RedactOptions::new(true, "phone").with_strict(true);
        assert!(via_builder.strict);
        assert_eq!(via_builder.extra_fields, vec!["phone"]);
    }

    #[test]
    fn redact_multipart_form_data_fields_filenames_and_nested_parts() {
        let boundary = "----ZniffBoundary7MA4YWxkTrZu0gW";
        let multipart_body = format!(
            "--{boundary}\r\n\
Content-Disposition: form-data; name=\"username\"\r\n\r\n\
alice\r\n\
--{boundary}\r\n\
Content-Disposition: form-data; name=\"password\"\r\n\r\n\
hunter2\r\n\
--{boundary}\r\n\
Content-Disposition: form-data; name=\"upload\"; filename=\"client_secret.pem\"\r\n\
Content-Type: application/x-pem-file\r\n\r\n\
PRIVATE_KEY_MATERIAL\r\n\
--{boundary}\r\n\
Content-Disposition: form-data; name=\"metadata\"\r\n\
Content-Type: application/json\r\n\r\n\
{{\"access_token\":\"tok_999\",\"role\":\"admin\"}}\r\n\
--{boundary}\r\n\
Content-Disposition: form-data; name=\"params\"\r\n\
Content-Type: application/x-www-form-urlencoded\r\n\r\n\
api_key=sec_key_1&lang=zh\r\n\
--{boundary}--\r\n"
        );

        let mut session = crate::session::Session::default();
        let id = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "POST".into(),
            host: "upload.example.com".into(),
            path: "/v1/submit".into(),
            url: "https://upload.example.com/v1/submit".into(),
            request_headers: vec![(
                "Content-Type".into(),
                format!("multipart/form-data; boundary=\"{boundary}\""),
            )],
        });
        session.append_body(id, Direction::Request, multipart_body.as_bytes());
        session.set_response(id, 200, vec![], None);
        session.mark_finished(id);

        let flow = session.hydrated(id).unwrap();
        let opts = RedactOptions::new(true, "");
        let (sanitized, rep) = redact_flow(&flow, &opts);

        // password + client_secret.pem + metadata.access_token + params.api_key = 4
        assert_eq!(rep.redacted_body_fields, 4);
        assert!(!rep.unsanitizable_req_body);
        assert!(!rep.strict_masked_req_body);

        let text = std::str::from_utf8(&sanitized.request_body).unwrap();
        assert!(text.contains("alice"), "{text}");
        assert!(text.contains("\"role\":\"admin\""), "{text}");
        assert!(text.contains("lang=zh"), "{text}");
        assert!(!text.contains("hunter2"), "{text}");
        assert!(!text.contains("PRIVATE_KEY_MATERIAL"), "{text}");
        assert!(!text.contains("tok_999"), "{text}");
        assert!(!text.contains("sec_key_1"), "{text}");
        assert!(text.contains(&format!("--{boundary}--")), "{text}");

        // Valid multipart structure still parses through present::multipart_pairs
        let ct = crate::present::content_type(&sanitized.request_headers);
        let pairs = crate::present::multipart_pairs(ct, &sanitized.request_body).unwrap();
        assert!(pairs.iter().any(|(k, v)| k == "username" && v == "alice"));
        assert!(
            pairs
                .iter()
                .any(|(k, v)| k == "password" && v == "[REDACTED]")
        );

        // Pass 2 idempotency
        let (sanitized2, rep2) = redact_flow(&sanitized, &opts);
        assert_eq!(rep2.total_redacted(), 0);
        assert_eq!(sanitized2.request_body, sanitized.request_body);
    }

    #[test]
    fn redact_strict_masks_binary_and_unstructured_bodies_idempotently() {
        let mut session = crate::session::Session::default();
        let id = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "POST".into(),
            host: "files.example.com".into(),
            path: "/raw".into(),
            url: "https://files.example.com/raw".into(),
            request_headers: vec![("Content-Type".into(), "text/plain".into())],
        });
        session.append_body(id, Direction::Request, b"unstructured secret prose");
        session.set_response(
            id,
            200,
            vec![("Content-Type".into(), "application/octet-stream".into())],
            None,
        );
        session.append_body(id, Direction::Response, &[0xff, 0x00, 0xfe, 0x01]);
        session.mark_finished(id);

        let flow = session.hydrated(id).unwrap();
        let strict_opts = RedactOptions::new(true, "@strict");
        let (sanitized, rep) = redact_flow(&flow, &strict_opts);

        assert_eq!(sanitized.request_body, STRICT_REDACTED_BODY);
        assert_eq!(sanitized.response_body, STRICT_REDACTED_BODY);
        assert_eq!(rep.redacted_body_fields, 2);
        assert!(!rep.unsanitizable_req_body);
        assert!(!rep.unsanitizable_res_body);
        assert!(rep.strict_masked_req_body);
        assert!(rep.strict_masked_res_body);
        assert!(rep.has_strict_masked_body());
        assert!(
            sanitized
                .comment
                .contains("[REDACTED; req/res body omitted in strict mode]"),
            "comment={}",
            sanitized.comment
        );

        // Pass 2 idempotency
        let (sanitized2, rep2) = redact_flow(&sanitized, &strict_opts);
        assert_eq!(rep2.total_redacted(), 0);
        assert_eq!(sanitized2.request_body, sanitized.request_body);
        assert_eq!(sanitized2.response_body, sanitized.response_body);
        assert_eq!(sanitized2.comment, sanitized.comment);
    }

    #[test]
    fn share_document_includes_rich_parity_fields_and_stays_inbound_compatible() {
        let mut session = crate::session::Session::default();
        let id = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::WebSocket,
            method: "POST".into(),
            host: "api.example.com".into(),
            path: "/graphql".into(),
            url: "https://api.example.com/graphql".into(),
            request_headers: vec![("Content-Type".into(), "application/json".into())],
        });
        session.set_client_ip(id, "127.0.0.1 (curl)");
        session.set_conn_info(
            id,
            crate::session::ConnInfo {
                dns_ms: Some(12),
                tcp_ms: Some(18),
                tls_ms: Some(25),
                reused: false,
                server_addr: "93.184.216.34:443".into(),
                tls_version: "TLSv1.3".into(),
                client_cert: String::new(),
                dialed_before_start: false,
            },
        );
        session.set_comment(id, "checkout step");
        session.set_rule_hit(id, "map-local 200");
        session.append_body(
            id,
            Direction::Request,
            br#"{"operationName":"GetUserProfile","query":"query GetUserProfile($id: ID!) { user(id: $id) { id } }","variables":{"id":"42"}}"#,
        );
        session.set_response(
            id,
            200,
            vec![("Content-Type".into(), "application/json".into())],
            None,
        );
        session.append_body(id, Direction::Response, br#"{"data":{"user":{"id":"42"}}}"#);
        for idx in 0..205 {
            let dir = if idx % 2 == 0 {
                Direction::Request
            } else {
                Direction::Response
            };
            let kind = match idx % 3 {
                0 => "文本",
                1 => "二进制",
                _ => "Ping",
            };
            session.add_message(id, dir, kind, format!("frame-{idx}"));
        }
        session.mark_finished(id);

        let mut flow = session.hydrated(id).unwrap();
        flow.pinned = true;
        flow.response_at = Some(flow.started + std::time::Duration::from_millis(90));
        flow.finished_at = Some(flow.started + std::time::Duration::from_millis(120));

        let doc_str = document_with_redact(
            std::slice::from_ref(&flow),
            SystemTime::now(),
            "",
            &RedactOptions::default(),
        );
        let doc_val: serde_json::Value = serde_json::from_str(&doc_str).unwrap();
        let f = &doc_val["flows"][0];
        assert_eq!(f["client_ip"], "127.0.0.1 (curl)");
        assert_eq!(f["pinned"], true);
        assert_eq!(f["graphql_op"], "GetUserProfile");
        assert_eq!(f["request_body_kind"], "graphql");
        assert_eq!(f["response_body_kind"], "json");
        assert_eq!(f["conn_info"]["reused"], false);
        assert_eq!(f["conn_info"]["tls_version"], "TLSv1.3");
        assert_eq!(f["conn_info"]["server_addr"], "93.184.216.34:443");
        assert_eq!(f["conn_info"]["dns_ms"], 12);
        assert_eq!(f["conn_info"]["tcp_ms"], 18);
        assert_eq!(f["conn_info"]["tls_ms"], 25);
        assert_eq!(f["conn_info"]["wait_ms"], 35);
        assert_eq!(f["conn_info"]["download_ms"], 30);
        assert_eq!(f["messages"].as_array().unwrap().len(), 200);
        assert_eq!(f["message_items"].as_array().unwrap().len(), 200);
        assert_eq!(f["messages_truncated"], true);
        assert_eq!(f["message_items"][0]["dir"], "up");
        assert_eq!(f["message_items"][0]["kind"], "text");
        assert_eq!(f["message_items"][1]["dir"], "down");
        assert_eq!(f["message_items"][1]["kind"], "binary");
        assert_eq!(f["message_items"][2]["kind"], "control");

        // InboundShare & InboundBody remain 100% backwards-compatible
        let inbound = parse_inbound_share(doc_str.as_bytes()).expect("valid InboundShare");
        assert_eq!(inbound.flows.len(), 1);
        assert_eq!(inbound.flows[0].messages.len(), 200);

        let body_str = flow_body_document_with_redact(&flow, &RedactOptions::default());
        let inbound_body = parse_inbound_body(body_str.as_bytes()).expect("valid InboundBody");
        assert_eq!(inbound_body.messages.len(), 200);
        assert!(inbound_body.request_body.contains("GetUserProfile"));
    }

    #[test]
    fn share_flow_decodes_protobuf_msgpack_form_hex_and_safe_images_with_redaction() {
        let mut session = crate::session::Session::default();

        // 1. Form + MessagePack with redaction
        let f_msgpack = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "POST".into(),
            host: "rpc.example.com".into(),
            path: "/login".into(),
            url: "https://rpc.example.com/login".into(),
            request_headers: vec![(
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            )],
        });
        session.append_body(
            f_msgpack,
            Direction::Request,
            b"user=alice&password=super_secret_pw",
        );
        // MessagePack fixmap(2): {"ok": true, "access_token": "mp_secret_123"}
        let mp_bytes: &[u8] = &[
            0x82, 0xa2, b'o', b'k', 0xc3, 0xac, b'a', b'c', b'c', b'e', b's', b's', b'_', b't',
            b'o', b'k', b'e', b'n', 0xad, b'm', b'p', b'_', b's', b'e', b'c', b'r', b'e', b't',
            b'_', b'1', b'2', b'3',
        ];
        session.set_response(
            f_msgpack,
            200,
            vec![("Content-Type".into(), "application/x-msgpack".into())],
            None,
        );
        session.append_body(f_msgpack, Direction::Response, mp_bytes);
        session.mark_finished(f_msgpack);

        // 2. Protobuf response + Hex preview
        let f_pb = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http2,
            method: "POST".into(),
            host: "grpc.example.com".into(),
            path: "/pkg.Svc/Call".into(),
            url: "https://grpc.example.com/pkg.Svc/Call".into(),
            request_headers: vec![("Content-Type".into(), "application/grpc".into())],
        });
        // gRPC frame: 0x00 (uncompressed) + len 7 + field 1 string "hello" (0x0a 0x05 "hello")
        let grpc_frame: &[u8] = &[
            0x00, 0x00, 0x00, 0x00, 0x07, 0x0a, 0x05, b'h', b'e', b'l', b'l', b'o',
        ];
        session.set_response(
            f_pb,
            200,
            vec![("Content-Type".into(), "application/grpc+proto".into())],
            None,
        );
        session.append_body(f_pb, Direction::Response, grpc_frame);
        session.mark_finished(f_pb);

        // 3. Safe raster PNG vs unsafe SVG
        let f_png = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "GET".into(),
            host: "cdn.example.com".into(),
            path: "/dot.png".into(),
            url: "https://cdn.example.com/dot.png".into(),
            request_headers: vec![],
        });
        let mut png_bytes = vec![
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
            b'D', b'R', 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x08,
        ];
        png_bytes.extend_from_slice(&[0u8; 12]);
        session.set_response(
            f_png,
            200,
            vec![("Content-Type".into(), "image/png".into())],
            None,
        );
        session.append_body(f_png, Direction::Response, &png_bytes);
        session.mark_finished(f_png);

        let f_svg = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "GET".into(),
            host: "cdn.example.com".into(),
            path: "/xss.svg".into(),
            url: "https://cdn.example.com/xss.svg".into(),
            request_headers: vec![],
        });
        session.set_response(
            f_svg,
            200,
            vec![("Content-Type".into(), "image/svg+xml".into())],
            None,
        );
        session.append_body(
            f_svg,
            Direction::Response,
            br#"<svg onload="alert(1)"></svg>"#,
        );
        session.mark_finished(f_svg);

        let flows = [f_msgpack, f_pb, f_png, f_svg]
            .into_iter()
            .map(|id| session.hydrated(id).unwrap())
            .collect::<Vec<_>>();

        let redact = RedactOptions::new(true, "");
        let doc_str = document_with_redact(&flows, SystemTime::now(), "", &redact);
        let v: serde_json::Value = serde_json::from_str(&doc_str).unwrap();

        // Check form rows & redacted MessagePack pretty JSON
        let j_mp = &v["flows"][0];
        assert_eq!(j_mp["request_body_kind"], "form");
        let form_rows = j_mp["request_form_rows"].as_array().unwrap();
        assert!(form_rows.iter().any(|r| r[0] == "user" && r[1] == "alice"));
        assert!(
            form_rows
                .iter()
                .any(|r| r[0] == "password" && r[1] == "[REDACTED]")
        );
        assert_eq!(j_mp["response_body_kind"], "msgpack");
        let mp_pretty = j_mp["response_body_pretty"].as_str().unwrap();
        assert!(mp_pretty.contains("\"ok\": true"), "{mp_pretty}");
        assert!(mp_pretty.contains("[REDACTED]"), "{mp_pretty}");
        assert!(!mp_pretty.contains("mp_secret_123"), "{mp_pretty}");

        // Check Protobuf wire decode + hex b64
        let j_pb = &v["flows"][1];
        assert_eq!(j_pb["response_body_kind"], "protobuf");
        assert!(
            j_pb["response_body_pretty"]
                .as_str()
                .unwrap()
                .contains("hello")
        );
        assert!(!j_pb["response_hex_b64"].as_str().unwrap().is_empty());

        // Check PNG allowed vs SVG blocked from inline data URL
        let j_png = &v["flows"][2];
        assert_eq!(j_png["response_body_kind"], "image");
        assert!(
            j_png["response_image_data_url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        assert_eq!(j_png["response_image_dims"], serde_json::json!([16, 8]));

        let j_svg = &v["flows"][3];
        assert!(j_svg.get("response_image_data_url").is_none());

        // Strict mode omits binary image data URL and hex bytes completely
        let strict = RedactOptions::new(true, "@strict");
        let strict_str = document_with_redact(&flows, SystemTime::now(), "", &strict);
        let sv: serde_json::Value = serde_json::from_str(&strict_str).unwrap();
        assert!(sv["flows"][2].get("response_image_data_url").is_none());
        assert!(sv["flows"][2].get("response_hex_b64").is_none());
        assert!(!strict_str.contains("mp_secret_123"));
    }

    #[test]
    fn share_spilled_and_evicted_bodies_serialize_cleanly() {
        let mut session = crate::session::Session::default();
        let id = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "POST".into(),
            host: "spill.example.com".into(),
            path: "/upload".into(),
            url: "https://spill.example.com/upload".into(),
            request_headers: vec![("Content-Type".into(), "application/json".into())],
        });
        let big_json = format!("{{\"payload\":\"{}\"}}", "a".repeat(24 * 1024));
        session.append_body(id, Direction::Request, big_json.as_bytes());
        session.set_response(id, 200, vec![], None);
        session.mark_finished(id);

        // Raw flow in session has request_spill.is_some() and empty inline request_body
        let raw_flow = session.get(id).unwrap().clone();
        assert!(raw_flow.request_spill.is_some());
        let lazy_doc = document_since_with_redact(
            std::slice::from_ref(&raw_flow),
            SystemTime::now(),
            "",
            1,
            None,
            &RedactOptions::default(),
        );
        let lv: serde_json::Value = serde_json::from_str(&lazy_doc).unwrap();
        assert_eq!(lv["flows"][0]["request_body_spilled"], true);
        assert_eq!(lv["flows"][0]["request_body"], "");

        // Hydrated flow body endpoint includes full pretty JSON & hex preview
        let hydrated = session.hydrated(id).unwrap();
        let body_doc = flow_body_document_with_redact(&hydrated, &RedactOptions::default());
        let bv: serde_json::Value = serde_json::from_str(&body_doc).unwrap();
        assert!(
            bv["request_body_pretty"]
                .as_str()
                .unwrap()
                .contains("\"payload\"")
        );
        assert_eq!(bv["request_raw_len"], big_json.len());
        assert!(!bv["request_hex_b64"].as_str().unwrap().is_empty());

        // Simulated rotated spill ring (expired) marks request_body_spilled = false and surfaces note
        let mut evicted = raw_flow;
        evicted.request_expired = Some(big_json.len());
        let ev_doc = document_since_with_redact(
            &[evicted],
            SystemTime::now(),
            "",
            2,
            None,
            &RedactOptions::default(),
        );
        let ev: serde_json::Value = serde_json::from_str(&ev_doc).unwrap();
        assert_eq!(ev["flows"][0]["request_body_spilled"], false);
        assert_eq!(ev["flows"][0]["request_expired"], big_json.len());
        assert!(
            ev["flows"][0]["request_body"]
                .as_str()
                .unwrap()
                .contains("reclaimed")
        );
    }

    #[test]
    fn share_page_html_has_parity_controls_and_strict_sandbox_iframe() {
        let html_zh = export_html(&[], "", &RedactOptions::default(), true);
        assert!(html_zh.contains("lang=\"zh-CN\""));
        assert!(html_zh.contains("--fg-muted: #8e9bb3"));
        assert!(html_zh.contains("--fg-muted: #526075"));
        assert!(html_zh.contains("id=\"theme-seg\""));
        assert!(html_zh.contains("id=\"lang-seg\""));
        assert!(html_zh.contains("id=\"summary-btn\""));
        assert!(html_zh.contains("id=\"seq-top-btn\""));
        assert!(html_zh.contains("id=\"sub-filter-input\""));
        assert!(html_zh.contains("id=\"merge-sub-btn\""));
        assert!(html_zh.contains("id=\"facet-btn\""));
        assert!(html_zh.contains("id=\"col-btn\""));
        assert!(html_zh.contains("function buildFetch(flow)"));
        assert!(html_zh.contains("function buildPythonRequests(flow)"));
        assert!(html_zh.contains("function buildRustReqwest(flow)"));
        assert!(html_zh.contains("function buildGoHttp(flow)"));
        assert!(html_zh.contains("function buildTsInterface(flow)"));
        assert!(html_zh.contains("function buildRustStruct(flow)"));
        assert!(html_zh.contains("function annotateValue(rawVal)"));
        assert!(html_zh.contains("function formatHexDump(b64, cols)"));
        assert!(html_zh.contains("sandbox=\"\" referrerpolicy=\"no-referrer\""));
        assert!(!html_zh.contains("allow-scripts"));
        assert!(!html_zh.contains("allow-same-origin"));

        let html_en = page(
            "{\"flows\":[]}",
            "/s/550e8400-e29b-41d4-a716-446655440000/flows.json",
            false,
        );
        assert!(html_en.contains("lang=\"en\""));
        assert!(html_en.contains("const DEFAULT_ZH = false;"));
    }

    #[test]
    fn share_page_js_uses_rev_incremental_ids_protocol_and_rust_keyword_escapes() {
        let html = page(
            "{\"flows\":[]}",
            "/s/550e8400-e29b-41d4-a716-446655440000/flows.json",
            true,
        );
        assert!(html.contains("function applyShareUpdate(prevData, next)"));
        assert!(html.contains("state.data.rev"));
        assert!(html.contains("${LIVE}${sep}since=${curRev}"));
        assert!(html.contains("next.incremental"));
        assert!(html.contains("Array.isArray(next.ids)"));
        assert!(html.contains("if (upd.changed) renderAll();"));
        assert!(!html.contains("state.data.seq"));
        assert!(!html.contains("next.seq"));
        assert!(html.contains("id=\"table-colgroup\""));
        assert!(html.contains("const RUST_KEYWORDS = new Set("));
        assert!(html.contains("ident = `r#${ident}`"));
        assert!(html.contains("onlyHostFilter(flow.host)"));
        assert!(html.contains(".code-viewer.hex-dump"));
        assert!(html.contains("w >= 980 ? 32 : 16"));
    }
}
