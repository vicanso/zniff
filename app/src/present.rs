//! Display helpers that do not touch the window: cURL, pretty bodies, query
//! strings, and the remembered listen port.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::i18n::{t, Lang};
use crate::session::Flow;

pub const DEFAULT_PORT: u16 = 8877;

pub fn parse_port(text: &str) -> Option<u16> {
    let port: u16 = text.trim().parse().ok()?;
    if port == 0 { None } else { Some(port) }
}

pub fn curl_command(flow: &Flow) -> String {
    let mut command = format!(
        "curl -X {} {}",
        shell_quote(&flow.method),
        shell_quote(&flow.url)
    );
    for (name, value) in &flow.request_headers {
        if name.eq_ignore_ascii_case("content-length") || name.eq_ignore_ascii_case("host") {
            continue;
        }
        command.push_str(" -H ");
        command.push_str(&shell_quote(&format!("{name}: {value}")));
    }
    if !flow.request_body.is_empty() {
        if let Ok(text) = std::str::from_utf8(&flow.request_body) {
            command.push_str(" --data-binary ");
            command.push_str(&shell_quote(text));
        }
    }
    command
}

pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

pub fn query_pairs(url: &str) -> Vec<(String, String)> {
    let Some(query) = url.split_once('?').map(|(_, query)| query) else {
        return Vec::new();
    };
    let query = query.split('#').next().unwrap_or(query);
    if query.is_empty() {
        return Vec::new();
    }
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(key), percent_decode(value))
        })
        .collect()
}

pub fn pretty_body(content_type: Option<&str>, body: &str) -> String {
    let kind = content_type.unwrap_or("").to_ascii_lowercase();
    if kind.contains("json") || looks_like_json(body) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
            if let Ok(pretty) = serde_json::to_string_pretty(&value) {
                return pretty;
            }
        }
    }
    if kind.contains("x-www-form-urlencoded") || (!kind.contains('/') && body.contains('=') && body.contains('&'))
    {
        return pretty_form(body);
    }
    if kind.contains("xml") || kind.contains("html") {
        return pretty_markup(body);
    }
    body.to_string()
}

pub fn content_type<'a>(headers: &'a [(String, String)]) -> Option<&'a str> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.as_str())
}

pub fn image_extension(content_type: &str) -> Option<&'static str> {
    let kind = content_type.to_ascii_lowercase();
    if kind.contains("png") {
        Some("png")
    } else if kind.contains("jpeg") || kind.contains("jpg") {
        Some("jpg")
    } else if kind.contains("webp") {
        Some("webp")
    } else if kind.contains("gif") {
        Some("gif")
    } else if kind.contains("svg") {
        Some("svg")
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// Splits a `text/event-stream` body into events. A blank line ends one.
pub fn sse_events(body: &str) -> Vec<SseEvent> {
    let mut events = Vec::new();
    let mut event = "message".to_string();
    let mut data: Vec<String> = Vec::new();
    for line in body.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            if !data.is_empty() {
                events.push(SseEvent {
                    event: event.clone(),
                    data: data.join("\n"),
                });
                data.clear();
                event = "message".to_string();
            }
            continue;
        }
        if line.starts_with(':') {
            continue;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => event = value.to_string(),
            "data" => data.push(value.to_string()),
            _ => {}
        }
    }
    if !data.is_empty() {
        events.push(SseEvent {
            event,
            data: data.join("\n"),
        });
    }
    events
}

pub fn format_timing(
    started: SystemTime,
    ttfb: Option<Duration>,
    total: Option<Duration>,
    request_bytes: u64,
    response_bytes: u64,
    lang: Lang,
) -> String {
    let mut parts = vec![format_clock(started)];
    if let Some(ttfb) = ttfb {
        parts.push(format!("TTFB {}", format_duration(ttfb)));
    }
    if let Some(total) = total {
        parts.push(format!(
            "{} {}",
            t(lang, "耗时", "elapsed"),
            format_duration(total)
        ));
    } else {
        parts.push(t(lang, "进行中", "in progress").into());
    }
    parts.push(format!(
        "{} {}",
        t(lang, "请求", "request"),
        format_count(request_bytes)
    ));
    parts.push(format!(
        "{} {}",
        t(lang, "响应", "response"),
        format_count(response_bytes)
    ));
    parts.join("  ·  ")
}

pub const SLOW_AFTER: Duration = Duration::from_secs(1);

pub fn is_slow(started: SystemTime, finished: Option<SystemTime>, now: SystemTime) -> bool {
    let end = finished.unwrap_or(now);
    end.duration_since(started)
        .is_ok_and(|elapsed| elapsed >= SLOW_AFTER)
}

/// Shell exports so curl, Node, Python, and Git trust this proxy.
pub fn proxy_env(port: u16, cert_path: &str) -> String {
    let proxy = format!("http://127.0.0.1:{port}");
    let cert = shell_quote(cert_path);
    [
        format!("export http_proxy={proxy}"),
        format!("export https_proxy={proxy}"),
        format!("export HTTP_PROXY={proxy}"),
        format!("export HTTPS_PROXY={proxy}"),
        format!("export SSL_CERT_FILE={cert}"),
        format!("export NODE_EXTRA_CA_CERTS={cert}"),
        format!("export REQUESTS_CA_BUNDLE={cert}"),
        format!("export CURL_CA_BUNDLE={cert}"),
        format!("export GIT_SSL_CAINFO={cert}"),
    ]
    .join("\n")
}

pub fn fetch_snippet(flow: &Flow) -> String {
    let headers = copy_headers(&flow.request_headers);
    let mut lines = vec![
        format!("fetch({}, {{", js_string(&flow.url)),
        format!("  method: {},", js_string(&flow.method)),
    ];
    if headers.is_empty() {
        lines.push("  headers: {},".into());
    } else {
        lines.push("  headers: {".into());
        for (name, value) in &headers {
            lines.push(format!("    {}: {},", js_string(name), js_string(value)));
        }
        lines.push("  },".into());
    }
    if let Ok(text) = std::str::from_utf8(&flow.request_body) {
        if !text.is_empty() {
            lines.push(format!("  body: {},", js_string(text)));
        }
    }
    lines.push("});".into());
    lines.join("\n")
}

pub fn python_requests(flow: &Flow) -> String {
    let headers = copy_headers(&flow.request_headers);
    let mut lines = vec![
        "import requests".into(),
        "requests.request(".into(),
        format!("    {},", py_string(&flow.method)),
        format!("    {},", py_string(&flow.url)),
    ];
    if !headers.is_empty() {
        lines.push("    headers={".into());
        for (name, value) in &headers {
            lines.push(format!("        {}: {},", py_string(name), py_string(value)));
        }
        lines.push("    },".into());
    }
    if let Ok(text) = std::str::from_utf8(&flow.request_body) {
        if !text.is_empty() {
            lines.push(format!("    data={},", py_string(text)));
        }
    }
    lines.push(")".into());
    lines.join("\n")
}

/// A shallow TypeScript interface for a JSON object or array. Nested objects
/// become sibling interfaces. Anything else returns `None`.
pub fn typescript_interface(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json.trim()).ok()?;
    if !value.is_object() && !value.is_array() {
        return None;
    }
    let mut types = TsGen::default();
    if value.is_array() {
        let root = types.emit("Item", &value);
        types.blocks.push(format!("export type Response = {root};"));
    } else {
        types.emit("Response", &value);
    }
    Some(types.blocks.join("\n"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurlRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// Parses a single curl command. Supports `-X`, `-H`, `--url`, and
/// `--data` / `--data-raw` / `--data-binary` / `-d`, with quotes.
/// A body without `-X` becomes POST. Unknown flags are ignored.
pub fn parse_curl(text: &str) -> Option<CurlRequest> {
    let tokens = shell_tokens(text);
    if tokens.is_empty() {
        return None;
    }
    let mut index = 0;
    if tokens[0] == "curl" {
        index = 1;
    }
    let mut method = String::new();
    let mut url = String::new();
    let mut headers = Vec::new();
    let mut body = String::new();
    let mut saw_body = false;
    while index < tokens.len() {
        let token = tokens[index].as_str();
        let (flag, inline) = split_flag(token);
        let arg = |index: &mut usize| -> Option<String> {
            if let Some(inline) = inline {
                return Some(inline.to_string());
            }
            *index += 1;
            tokens.get(*index).cloned()
        };
        match flag {
            "-X" | "--request" => method = arg(&mut index)?,
            "-H" | "--header" => {
                let header = arg(&mut index)?;
                let (name, value) = header.split_once(':')?;
                headers.push((name.trim().to_string(), value.trim().to_string()));
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-ascii" => {
                body = arg(&mut index)?;
                saw_body = true;
            }
            "--url" => url = arg(&mut index)?,
            "-A" | "--user-agent" => {
                headers.push(("User-Agent".into(), arg(&mut index)?));
            }
            "-e" | "--referer" => {
                headers.push(("Referer".into(), arg(&mut index)?));
            }
            "-b" | "--cookie" => {
                headers.push(("Cookie".into(), arg(&mut index)?));
            }
            "-u" | "--user" | "-x" | "--proxy" => {
                let _ = arg(&mut index)?;
            }
            other if other.starts_with('-') => {}
            other if url.is_empty() => url = other.to_string(),
            _ => {}
        }
        index += 1;
    }
    if url.is_empty() {
        return None;
    }
    if method.is_empty() {
        method = if saw_body { "POST".into() } else { "GET".into() };
    }
    Some(CurlRequest {
        method,
        url,
        headers,
        body,
    })
}

/// Extra readable text for a header value: JWT payload, a unix time, or
/// printable base64. The original value stays intact.
pub fn annotate_value(name: &str, value: &str) -> Option<String> {
    if let Some(text) = jwt_hint(name, value) {
        return Some(text);
    }
    if let Some(text) = timestamp_hint(value.trim()) {
        return Some(text);
    }
    base64_hint(value.trim())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Same,
    Add,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub text: String,
}

/// Line diff capped at 200 lines on each side.
pub fn line_diff(before: &str, after: &str) -> Vec<DiffLine> {
    let left: Vec<&str> = before.lines().take(200).collect();
    let right: Vec<&str> = after.lines().take(200).collect();
    let rows = left.len();
    let cols = right.len();
    let mut score = vec![vec![0u16; cols + 1]; rows + 1];
    for i in (0..rows).rev() {
        for j in (0..cols).rev() {
            score[i][j] = if left[i] == right[j] {
                score[i + 1][j + 1].saturating_add(1)
            } else {
                score[i + 1][j].max(score[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < rows && j < cols {
        if left[i] == right[j] {
            out.push(DiffLine {
                kind: DiffKind::Same,
                text: left[i].to_string(),
            });
            i += 1;
            j += 1;
        } else if score[i + 1][j] >= score[i][j + 1] {
            out.push(DiffLine {
                kind: DiffKind::Remove,
                text: left[i].to_string(),
            });
            i += 1;
        } else {
            out.push(DiffLine {
                kind: DiffKind::Add,
                text: right[j].to_string(),
            });
            j += 1;
        }
    }
    while i < rows {
        out.push(DiffLine {
            kind: DiffKind::Remove,
            text: left[i].to_string(),
        });
        i += 1;
    }
    while j < cols {
        out.push(DiffLine {
            kind: DiffKind::Add,
            text: right[j].to_string(),
        });
        j += 1;
    }
    out
}

pub fn flow_diff(base: &Flow, current: &Flow, lang: Lang) -> String {
    let mut out = String::new();
    push_diff(
        &mut out,
        t(lang, "URL", "URL"),
        &base.url,
        &current.url,
    );
    push_diff(
        &mut out,
        t(lang, "请求头", "Request headers"),
        &header_block(&base.request_headers),
        &header_block(&current.request_headers),
    );
    push_diff(
        &mut out,
        t(lang, "请求 body", "Request body"),
        &body_text(&base.request_body),
        &body_text(&current.request_body),
    );
    push_diff(
        &mut out,
        t(lang, "响应头", "Response headers"),
        &header_block(&base.response_headers),
        &header_block(&current.response_headers),
    );
    push_diff(
        &mut out,
        t(lang, "响应 body", "Response body"),
        &body_text(&base.response_body),
        &body_text(&current.response_body),
    );
    out
}

fn push_diff(out: &mut String, title: &str, before: &str, after: &str) {
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(title);
    out.push('\n');
    if before == after {
        out.push_str("  =\n");
        return;
    }
    for line in line_diff(before, after) {
        let mark = match line.kind {
            DiffKind::Same => " ",
            DiffKind::Add => "+",
            DiffKind::Remove => "-",
        };
        out.push(mark.chars().next().unwrap_or(' '));
        out.push(' ');
        out.push_str(&line.text);
        out.push('\n');
    }
}

fn header_block(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn body_text(body: &[u8]) -> String {
    match std::str::from_utf8(body) {
        Ok(text) => text.to_string(),
        Err(_) => format!("<{len} binary bytes>", len = body.len()),
    }
}

fn copy_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(name, _)| {
            !name.eq_ignore_ascii_case("host")
                && !name.eq_ignore_ascii_case("content-length")
                && !name.eq_ignore_ascii_case("connection")
                && !name.eq_ignore_ascii_case("proxy-connection")
        })
        .cloned()
        .collect()
}

fn js_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}

fn py_string(text: &str) -> String {
    format!("'''{}'''", text.replace('\\', "\\\\").replace("'''", "\\'\\'\\'"))
}

#[derive(Default)]
struct TsGen {
    blocks: Vec<String>,
    used: Vec<String>,
}

impl TsGen {
    fn emit(&mut self, name: &str, value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::Object(map) => {
                let iface = self.fresh(name);
                let mut fields = String::new();
                for (key, child) in map {
                    let child_name = pascal(key);
                    let ty = self.emit(&child_name, child);
                    fields.push_str(&format!("  {}: {ty};\n", ts_key(key)));
                }
                self.blocks
                    .push(format!("export interface {iface} {{\n{fields}}}"));
                iface
            }
            serde_json::Value::Array(items) => {
                let inner = items
                    .first()
                    .map(|item| self.emit(name, item))
                    .unwrap_or_else(|| "unknown".into());
                format!("{inner}[]")
            }
            serde_json::Value::String(_) => "string".into(),
            serde_json::Value::Number(_) => "number".into(),
            serde_json::Value::Bool(_) => "boolean".into(),
            serde_json::Value::Null => "null".into(),
        }
    }

    fn fresh(&mut self, name: &str) -> String {
        let base = if name.is_empty() { "Value" } else { name };
        let mut candidate = base.to_string();
        let mut n = 2;
        while self.used.iter().any(|used| used == &candidate) {
            candidate = format!("{base}{n}");
            n += 1;
        }
        self.used.push(candidate.clone());
        candidate
    }
}

fn pascal(key: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for ch in key.chars() {
        if ch.is_ascii_alphanumeric() {
            if upper {
                out.extend(ch.to_uppercase());
                upper = false;
            } else {
                out.push(ch);
            }
        } else {
            upper = true;
        }
    }
    if out.is_empty() {
        "Value".into()
    } else {
        out
    }
}

fn ts_key(key: &str) -> String {
    if !key.is_empty()
        && key
            .chars()
            .enumerate()
            .all(|(index, ch)| ch.is_ascii_alphanumeric() || ch == '_' || (ch == '$' && index == 0))
        && !key.chars().next().is_some_and(|ch| ch.is_ascii_digit())
    {
        key.to_string()
    } else {
        js_string(key)
    }
}

fn shell_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(ch) = chars.next() {
        if let Some(mark) = quote {
            if ch == mark {
                quote = None;
            } else if ch == '\\' && mark == '"' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            } else {
                current.push(ch);
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            ch if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn split_flag(token: &str) -> (&str, Option<&str>) {
    const FLAGS: &[&str] = &[
        "--data-binary",
        "--data-raw",
        "--data-ascii",
        "--request",
        "--header",
        "--user-agent",
        "--referer",
        "--cookie",
        "--proxy",
        "--data",
        "--url",
        "--user",
        "-X",
        "-H",
        "-d",
        "-A",
        "-e",
        "-b",
        "-u",
        "-x",
    ];
    for flag in FLAGS {
        if token == *flag {
            return (flag, None);
        }
        if let Some(rest) = token.strip_prefix(flag) {
            if flag.starts_with("--") {
                if let Some(rest) = rest.strip_prefix('=') {
                    return (flag, Some(rest));
                }
            } else if !rest.is_empty() && !rest.starts_with('-') {
                return (flag, Some(rest));
            }
        }
    }
    (token, None)
}

fn jwt_hint(name: &str, value: &str) -> Option<String> {
    let token = value.trim();
    let token = if name.eq_ignore_ascii_case("authorization") {
        token
            .strip_prefix("Bearer ")
            .or_else(|| token.strip_prefix("bearer "))?
    } else {
        token
    };
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    let _sig = parts.next()?;
    if parts.next().is_some() || payload.is_empty() {
        return None;
    }
    let bytes = base64url_decode(payload)?;
    let text = String::from_utf8(bytes).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let pretty = serde_json::to_string_pretty(&value).ok()?;
    let mut out = format!("JWT\n{pretty}");
    if let Some(exp) = value.get("exp").and_then(|item| item.as_i64()) {
        out.push_str(&format!("\nexp {}", format_unix_utc(exp)));
    }
    Some(out)
}

fn timestamp_hint(value: &str) -> Option<String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let (secs, label) = if value.len() == 13 {
        (value.parse::<i64>().ok()? / 1000, "unix ms")
    } else if value.len() == 10 {
        (value.parse::<i64>().ok()?, "unix")
    } else {
        return None;
    };
    let year = format_unix_utc(secs);
    if !year.starts_with("20") {
        return None;
    }
    Some(format!("{label} {year}"))
}

fn base64_hint(value: &str) -> Option<String> {
    if value.len() < 16 || value.len() % 4 != 0 || value.contains('.') {
        return None;
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/' || byte == b'=')
    {
        return None;
    }
    let bytes = base64_decode(value)?;
    let text = String::from_utf8(bytes).ok()?;
    if text.len() < 8 || text.chars().any(|ch| ch.is_control() && ch != '\n' && ch != '\t') {
        return None;
    }
    Some(format!("base64 {text}"))
}

fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    let mut standard = text.replace('-', "+").replace('_', "/");
    while standard.len() % 4 != 0 {
        standard.push('=');
    }
    base64_decode(&standard)
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    fn val(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let chunk: Vec<u8> = bytes[index..].iter().take(4).copied().collect();
        if chunk.len() < 4 {
            return None;
        }
        index += 4;
        let pads = chunk.iter().filter(|byte| **byte == b'=').count();
        if pads > 2 {
            return None;
        }
        let a = val(chunk[0])?;
        let b = val(chunk[1])?;
        let c = if chunk[2] == b'=' { 0 } else { val(chunk[2])? };
        let d = if chunk[3] == b'=' { 0 } else { val(chunk[3])? };
        out.push((a << 2) | (b >> 4));
        if pads < 2 {
            out.push((b << 4) | (c >> 2));
        }
        if pads < 1 {
            out.push((c << 6) | d);
        }
    }
    Some(out)
}

fn format_unix_utc(secs: i64) -> String {
    let tod = secs.rem_euclid(86_400);
    let hour = tod / 3600;
    let minute = (tod % 3600) / 60;
    let second = tod % 60;
    let z = secs.div_euclid(86_400) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {hour:02}:{minute:02}:{second:02} UTC")
}

pub fn format_clock(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs() as i64;
    let millis = duration.subsec_millis();
    let local = secs + local_offset_secs();
    let day = local.rem_euclid(86_400);
    let hour = day / 3600;
    let minute = (day % 3600) / 60;
    let second = day % 60;
    format!("{hour:02}:{minute:02}:{second:02}.{millis:03}")
}

/// Gap from the previous arrival. The first flow, and a clock that steps
/// backwards, both read as `+0 ms`.
pub fn format_gap(previous: Option<SystemTime>, started: SystemTime) -> String {
    let Some(previous) = previous else {
        return "+0 ms".into();
    };
    match started.duration_since(previous) {
        Ok(duration) => format_delta(duration),
        Err(_) => "+0 ms".into(),
    }
}

fn format_delta(duration: Duration) -> String {
    let ms = duration.as_millis();
    if ms < 1_000 {
        return format!("+{ms} ms");
    }
    if ms < 10_000 {
        let whole = ms / 1_000;
        let frac = (ms % 1_000) / 10;
        return format!("+{whole}.{frac:02} s");
    }
    if ms < 60_000 {
        let whole = ms / 1_000;
        let frac = (ms % 1_000) / 100;
        return format!("+{whole}.{frac} s");
    }
    if ms < 3_600_000 {
        // Hundredths of a minute. 60_000 ms is `+1.00 m`.
        let hundredths = ms / 600;
        if hundredths < 1_000 {
            return format!("+{}.{:02} m", hundredths / 100, hundredths % 100);
        }
        return format!("+{} m", hundredths / 100);
    }
    let tenths = ms / 360_000;
    if tenths < 100 {
        format!("+{}.{} h", tenths / 10, tenths % 10)
    } else {
        format!("+{} h", tenths / 10)
    }
}

fn local_offset_secs() -> i64 {
    use std::sync::OnceLock;
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        let Ok(output) = std::process::Command::new("date").arg("+%z").output() else {
            return 0;
        };
        let text = String::from_utf8_lossy(&output.stdout);
        let text = text.trim();
        let (sign, rest) = match text.as_bytes().first() {
            Some(b'+') => (1, &text[1..]),
            Some(b'-') => (-1, &text[1..]),
            _ => return 0,
        };
        if rest.len() < 4 {
            return 0;
        }
        let hours: i64 = rest[..2].parse().unwrap_or(0);
        let minutes: i64 = rest[2..4].parse().unwrap_or(0);
        sign * (hours * 3600 + minutes * 60)
    })
}

fn format_duration(duration: Duration) -> String {
    let ms = duration.as_millis();
    if ms >= 1000 {
        format!("{:.2} s", ms as f64 / 1000.0)
    } else {
        format!("{ms} ms")
    }
}

fn format_count(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn looks_like_json(body: &str) -> bool {
    let trimmed = body.trim_start();
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

fn pretty_form(body: &str) -> String {
    body.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            format!("{}={}", percent_decode(key), percent_decode(value))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn pretty_markup(body: &str) -> String {
    let mut out = String::new();
    let mut depth: i32 = 0;
    let mut at_line_start = true;
    for line in body.split_inclusive('>') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let closing = trimmed.starts_with("</");
        let declaration = trimmed.starts_with("<!") || trimmed.starts_with("<?");
        if closing {
            depth = depth.saturating_sub(1);
        }
        if !at_line_start {
            out.push('\n');
        }
        for _ in 0..depth.max(0) {
            out.push_str("  ");
        }
        out.push_str(trimmed);
        at_line_start = false;
        let self_closing = trimmed.ends_with("/>") || declaration;
        if !closing && !self_closing && trimmed.starts_with('<') {
            depth += 1;
        }
    }
    if out.is_empty() { body.to_string() } else { out }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &text[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    index += 3;
                } else {
                    out.push(b'%');
                    index += 1;
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|err| String::from_utf8_lossy(err.as_bytes()).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{FlowDraft, Protocol, Session};

    fn flow() -> Flow {
        let mut session = Session::default();
        let id = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "POST".into(),
            host: "example.com".into(),
            path: "/v1?q=a%20b&n=1".into(),
            url: "https://example.com/v1?q=a%20b&n=1".into(),
            request_headers: vec![
                ("Host".into(), "example.com".into()),
                ("Content-Type".into(), "application/json".into()),
            ],
        });
        session.append_body(id, crate::session::Direction::Request, br#"{"a":1}"#);
        session.get(id).unwrap().clone()
    }

    #[test]
    fn curl_quotes_and_skips_host() {
        let command = curl_command(&flow());
        assert!(command.starts_with("curl -X 'POST' 'https://example.com/v1?q=a%20b&n=1'"));
        assert!(!command.contains("Host:"));
        assert!(command.contains("--data-binary '{\"a\":1}'"));
    }

    #[test]
    fn query_decodes_pairs() {
        let pairs = query_pairs("https://example.com/v1?q=a%20b&n=1");
        assert_eq!(
            pairs,
            vec![("q".into(), "a b".into()), ("n".into(), "1".into())]
        );
    }

    #[test]
    fn pretty_json_and_form() {
        let json = pretty_body(Some("application/json"), r#"{"a":1}"#);
        assert!(json.contains("\n"));
        assert!(json.contains("\"a\""));
        let form = pretty_body(Some("application/x-www-form-urlencoded"), "a=1&b=c+d");
        assert_eq!(form, "a=1\nb=c d");
    }

    #[test]
    fn sse_splits_on_blank_lines() {
        let events = sse_events("event: tick\ndata: one\ndata: two\n\ndata: plain\n\n");
        assert_eq!(
            events,
            vec![
                SseEvent {
                    event: "tick".into(),
                    data: "one\ntwo".into(),
                },
                SseEvent {
                    event: "message".into(),
                    data: "plain".into(),
                },
            ]
        );
    }

    #[test]
    fn gap_and_clock_keep_milliseconds() {
        let start = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(format_gap(None, start), "+0 ms");
        assert_eq!(format_gap(Some(start + Duration::from_secs(2)), start), "+0 ms");
        assert_eq!(format_gap(Some(start), start + Duration::from_millis(45)), "+45 ms");
        assert_eq!(
            format_gap(Some(start), start + Duration::from_millis(1_240)),
            "+1.24 s"
        );
        assert_eq!(
            format_gap(Some(start), start + Duration::from_millis(10_500)),
            "+10.5 s"
        );
        assert_eq!(
            format_gap(Some(start), start + Duration::from_millis(90_000)),
            "+1.50 m"
        );
        let clock = format_clock(start + Duration::from_millis(123));
        assert!(clock.ends_with(".123"), "{clock}");
        assert_eq!(clock.len(), 12);
    }

    #[test]
    fn port_rejects_zero_and_blank() {
        assert_eq!(parse_port("8877"), Some(8877));
        assert_eq!(parse_port(" 80 "), Some(80));
        assert_eq!(parse_port("0"), None);
        assert_eq!(parse_port(""), None);
        assert_eq!(parse_port("65536"), None);
    }

    #[test]
    fn snippets_env_and_curl_round_trip() {
        let flow = flow();
        let fetch = fetch_snippet(&flow);
        assert!(fetch.contains("fetch(\"https://example.com/v1?q=a%20b&n=1\""));
        assert!(fetch.contains("\"Content-Type\""));
        assert!(!fetch.contains("Host"));
        let python = python_requests(&flow);
        assert!(python.contains("import requests"));
        assert!(python.contains("'''POST'''"));
        let env = proxy_env(8877, "/tmp/zniff/ca.crt");
        assert!(env.contains("export https_proxy=http://127.0.0.1:8877"));
        assert!(env.contains("export NODE_EXTRA_CA_CERTS='/tmp/zniff/ca.crt'"));
        let parsed = parse_curl(
            "curl -X POST 'https://example.com/v1' -H 'Accept: text/plain' --data-raw '{\"a\":1}'",
        )
        .unwrap();
        assert_eq!(parsed.method, "POST");
        assert_eq!(parsed.url, "https://example.com/v1");
        assert_eq!(parsed.headers, vec![("Accept".into(), "text/plain".into())]);
        assert_eq!(parsed.body, "{\"a\":1}");
        let implied = parse_curl("curl https://example.com/v1 -d hi").unwrap();
        assert_eq!(implied.method, "POST");
        assert_eq!(implied.body, "hi");
    }

    #[test]
    fn types_jwt_and_diff() {
        let ts = typescript_interface(r#"{"user":{"id":1},"tags":["a"]}"#).unwrap();
        assert!(ts.contains("export interface Response"));
        assert!(ts.contains("user: User;"));
        assert!(ts.contains("tags: string[];"));
        let payload = base64url_encode(br#"{"sub":"a","exp":1700000000}"#);
        let header = base64url_encode(br#"{"alg":"none"}"#);
        let hint = annotate_value("Authorization", &format!("Bearer {header}.{payload}.x")).unwrap();
        assert!(hint.contains("\"sub\": \"a\""));
        assert!(hint.contains("2023-11-14 22:13:20 UTC"));
        let stamp = annotate_value("X-Time", "1700000000").unwrap();
        assert!(stamp.contains("2023-11-14 22:13:20 UTC"));
        let encoded = base64_encode(b"hello-zniff");
        let decoded = annotate_value("X-Data", &encoded).unwrap();
        assert!(decoded.contains("hello-zniff"));
        let diff = line_diff("a\nb\nc", "a\nc\nd");
        assert!(diff.iter().any(|line| line.kind == DiffKind::Remove && line.text == "b"));
        assert!(diff.iter().any(|line| line.kind == DiffKind::Add && line.text == "d"));
    }

    fn base64url_encode(bytes: &[u8]) -> String {
        base64_encode(bytes).replace('+', "-").replace('/', "_").replace('=', "")
    }

    fn base64_encode(bytes: &[u8]) -> String {
        const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        let mut index = 0;
        while index + 3 <= bytes.len() {
            let n = ((bytes[index] as u32) << 16) | ((bytes[index + 1] as u32) << 8) | bytes[index + 2] as u32;
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
            out.push(TABLE[(n & 63) as usize] as char);
            index += 3;
        }
        if index < bytes.len() {
            let left = bytes.len() - index;
            let mut n = (bytes[index] as u32) << 16;
            if left == 2 {
                n |= (bytes[index + 1] as u32) << 8;
            }
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            if left == 2 {
                out.push(TABLE[((n >> 6) & 63) as usize] as char);
                out.push('=');
            } else {
                out.push('=');
                out.push('=');
            }
        }
        out
    }
}
