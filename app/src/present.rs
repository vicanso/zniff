//! Display helpers that do not touch the window: cURL, pretty bodies, query
//! strings, and the remembered listen port.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::i18n::{Lang, t};
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
    if !flow.request_body.is_empty()
        && let Ok(text) = std::str::from_utf8(&flow.request_body)
    {
        command.push_str(" --data-binary ");
        command.push_str(&shell_quote(text));
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
    if (kind.contains("json") || kind.contains("graphql") || looks_like_json(body))
        && let Some(gql) = pretty_graphql_json(body)
    {
        return gql;
    }
    if (kind.contains("json") || looks_like_json(body))
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(body)
        && let Ok(pretty) = serde_json::to_string_pretty(&value)
    {
        return pretty;
    }
    if let Some(pairs) = multipart_pairs(content_type, body.as_bytes()) {
        return pairs
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("\n");
    }
    if kind.contains("x-www-form-urlencoded")
        || (!kind.contains('/') && body.contains('=') && body.contains('&'))
    {
        return pretty_form(body);
    }
    if kind.contains("xml") || kind.contains("html") {
        return pretty_markup(body);
    }
    body.to_string()
}

/// Decodes binary Protobuf (including gRPC length-prefixed frames) or MessagePack
/// payloads into a human-readable string when `pretty` mode is active.
pub fn pretty_binary_body(content_type: Option<&str>, body: &[u8]) -> Option<String> {
    if is_msgpack(content_type) {
        return decode_msgpack_to_json(body);
    }
    if is_protobuf(content_type) {
        return decode_protobuf_wire(content_type, body);
    }
    None
}

/// Case-insensitive keyword search inside a body text. Returns `(match_count, filtered_text)`
/// where each matching line is prefixed with `[L<n>] ` so its original line number stays visible.
pub fn filter_body_lines(text: &str, query: &str) -> (usize, String) {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() || text.is_empty() {
        return (0, text.to_string());
    }
    let mut hits = 0usize;
    let mut matched_lines = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let lower = line.to_lowercase();
        let line_hits = lower.matches(&needle).count();
        if line_hits > 0 {
            hits += line_hits;
            let line_no = idx + 1;
            matched_lines.push(format!("[L{line_no}] {line}"));
        }
    }
    (hits, matched_lines.join("\n"))
}

pub fn content_type(headers: &[(String, String)]) -> Option<&str> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.as_str())
}

/// Pretty mode draws the image and a byte count. Raw keeps the hex text.
pub struct ImageBody {
    pub text: String,
    pub note: String,
    pub caption: String,
}

pub fn image_body(
    pretty: bool,
    has_preview: bool,
    text: String,
    note: String,
    bytes: usize,
    lang: Lang,
) -> ImageBody {
    if !pretty || !has_preview {
        return ImageBody {
            text,
            note,
            caption: String::new(),
        };
    }
    let caption = match lang {
        Lang::Zh => format!("共 {bytes} 字节"),
        Lang::En => format!("{bytes} bytes"),
    };
    ImageBody {
        text: String::new(),
        note: without_hex_cap_note(&note),
        caption,
    }
}

/// The hex-window note belongs on the raw dump. A decoded preview drops it
/// and keeps every other note, such as a failed decompression.
pub fn without_hex_cap_note(note: &str) -> String {
    note.lines()
        .filter(|line| !is_hex_cap_note(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_hex_cap_note(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("十六进制只显示前 ") || line.starts_with("Hex shows the first ")
}

/// Pixel size of a PNG, JPEG, GIF, or WebP body. Unknown bytes are skipped.
pub fn image_pixel_size(bytes: &[u8]) -> Option<(u32, u32)> {
    png_size(bytes)
        .or_else(|| gif_size(bytes))
        .or_else(|| webp_size(bytes))
        .or_else(|| jpeg_size(bytes))
        .filter(|&(w, h)| w > 0 && h > 0 && w <= 32_768 && h <= 32_768)
}

/// Display size. A small icon stays at its own pixels. A larger picture
/// shrinks so neither side passes 200.
pub fn preview_box(width: u32, height: u32) -> (u32, u32) {
    const MAX: f32 = 200.0;
    if width == 0 || height == 0 {
        return (160, 160);
    }
    let scale = (MAX / width as f32).min(MAX / height as f32).min(1.0);
    let w = ((width as f32) * scale).round().max(1.0) as u32;
    let h = ((height as f32) * scale).round().max(1.0) as u32;
    (w, h)
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    Some((
        u32::from_be_bytes(bytes[16..20].try_into().ok()?),
        u32::from_be_bytes(bytes[20..24].try_into().ok()?),
    ))
}

fn gif_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 10 || !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return None;
    }
    Some((
        u16::from_le_bytes(bytes[6..8].try_into().ok()?) as u32,
        u16::from_le_bytes(bytes[8..10].try_into().ok()?) as u32,
    ))
}

fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 30 || !bytes.starts_with(b"RIFF") || &bytes[8..12] != b"WEBP" {
        return None;
    }
    match &bytes[12..16] {
        b"VP8X" => Some((
            1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]),
            1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]),
        )),
        b"VP8 " if bytes.len() >= 30 && bytes[23..26] == [0x9d, 0x01, 0x2a] => Some((
            u16::from_le_bytes(bytes[26..28].try_into().ok()?) as u32 & 0x3fff,
            u16::from_le_bytes(bytes[28..30].try_into().ok()?) as u32 & 0x3fff,
        )),
        b"VP8L" if bytes.len() >= 25 && bytes[20] == 0x2f => {
            let bits = u32::from_le_bytes(bytes[21..25].try_into().ok()?);
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        _ => None,
    }
}

fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xff || bytes[1] != 0xd8 {
        return None;
    }
    let mut i = 2;
    while i + 8 < bytes.len() {
        if bytes[i] != 0xff {
            i += 1;
            continue;
        }
        while i < bytes.len() && bytes[i] == 0xff {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let marker = bytes[i];
        i += 1;
        if marker == 0xd8 || marker == 0xd9 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        if i + 1 >= bytes.len() {
            break;
        }
        let len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
        if len < 2 || i + len > bytes.len() {
            break;
        }
        let sof = (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc);
        if sof && len >= 7 {
            let height = u16::from_be_bytes([bytes[i + 3], bytes[i + 4]]) as u32;
            let width = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            return Some((width, height));
        }
        i += len;
    }
    None
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
    parts.join(" · ")
}

pub fn is_slow(started: SystemTime, finished: Option<SystemTime>, now: SystemTime) -> bool {
    crate::session::is_slow(started, finished, now)
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
    if let Ok(text) = std::str::from_utf8(&flow.request_body)
        && !text.is_empty()
    {
        lines.push(format!("  body: {},", js_string(text)));
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
            lines.push(format!(
                "        {}: {},",
                py_string(name),
                py_string(value)
            ));
        }
        lines.push("    },".into());
    }
    if let Ok(text) = std::str::from_utf8(&flow.request_body)
        && !text.is_empty()
    {
        lines.push(format!("    data={},", py_string(text)));
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
        method = if saw_body {
            "POST".into()
        } else {
            "GET".into()
        };
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
/// Splits a `Cookie` or `Set-Cookie` value on `;`. A piece without `=` is a flag.
pub fn cookie_pairs(value: &str) -> Vec<(String, String)> {
    value
        .split(';')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            let (key, val) = part.split_once('=').unwrap_or((part, ""));
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), val.trim().to_string()))
        })
        .collect()
}

/// Case-insensitive `a–z`. Equal names keep their original order and case.
pub fn sort_headers_pretty(headers: &mut [(String, String)]) {
    headers.sort_by(|left, right| {
        left.0
            .to_ascii_lowercase()
            .cmp(&right.0.to_ascii_lowercase())
    });
}

/// Pretty rows for one `Cookie` header. Other headers (including `Set-Cookie`) return `None`.
pub fn pretty_cookie_rows(header: &str, value: &str) -> Option<Vec<(String, String)>> {
    if !header.eq_ignore_ascii_case("cookie") {
        return None;
    }
    let pairs = cookie_pairs(value);
    if pairs.is_empty() {
        return None;
    }
    Some(
        pairs
            .into_iter()
            .map(|(key, val)| {
                let shown = if val.is_empty() {
                    String::new()
                } else if let Some(hint) = annotate_value(&key, &val) {
                    format!("{val}\n{hint}")
                } else {
                    val
                };
                (format!("{header} · {key}"), shown)
            })
            .collect(),
    )
}

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
#[cfg(test)]
pub enum DiffKind {
    Same,
    Add,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(test)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub text: String,
}

/// Line diff capped at 200 lines on each side.
#[cfg(test)]
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

/// Language for the read-only body editor. Short or binary text stays a
/// plain block. JSON, HTML, and XML use their highlighter; other long
/// multiline text uses plain text with line numbers.
pub fn editor_language(content_type: Option<&str>, text: &str) -> Option<&'static str> {
    if text.is_empty() {
        return None;
    }
    let kind = content_type.unwrap_or("").to_ascii_lowercase();
    if is_msgpack(content_type) && looks_like_json(text) {
        return Some("json");
    }
    if is_protobuf(content_type) && text.contains('\n') {
        return Some("plaintext");
    }
    if kind.contains("json") && !looks_like_json(text) && text.contains('\n') {
        return Some("plaintext");
    }
    if kind.contains("json")
        || ((kind.is_empty() || kind.contains("text/plain")) && looks_like_json(text))
    {
        return Some("json");
    }
    if kind.contains("html") {
        return Some("html");
    }
    if kind.contains("markdown") {
        return Some("markdown");
    }
    if kind.contains("xml") {
        return Some("xml");
    }
    if kind.contains("javascript") {
        return Some("javascript");
    }
    let textual = kind.is_empty()
        || kind.contains("text/")
        || kind.contains("yaml")
        || kind.contains("csv")
        || kind.contains("graphql");
    if textual && text.contains('\n') && text.len() >= 80 {
        return Some(if kind.contains("yaml") {
            "yaml"
        } else {
            "plaintext"
        });
    }
    None
}

/// Parses HTML or Markdown bodies into an [`nsh_base::text_view::Document`] when
/// `content_type` indicates `html` or `markdown` and the parsed document has at
/// least one visible block (empty SPA shells return `None`).
pub fn parse_text_view_doc(
    content_type: Option<&str>,
    bytes: &[u8],
) -> Option<nsh_base::text_view::Document> {
    let text = std::str::from_utf8(bytes).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    let kind = content_type.unwrap_or("").to_ascii_lowercase();
    let doc = if kind.contains("markdown") {
        nsh_base::text_view::Parser::new().markdown(text)
    } else if kind.contains("html") {
        nsh_base::text_view::Parser::new().html(text)
    } else {
        return None;
    };
    (!doc.blocks.is_empty()).then_some(doc)
}

/// One block the diff view can compare: URL, both header lists, both bodies.
pub fn flow_snapshot(flow: &Flow, lang: Lang) -> String {
    let mut out = String::new();
    push_section(&mut out, t(lang, "URL", "URL"), &flow.url);
    push_section(
        &mut out,
        t(lang, "请求头", "Request headers"),
        &header_block(&flow.request_headers),
    );
    push_section(
        &mut out,
        t(lang, "请求 body", "Request body"),
        &body_text(&flow.request_body),
    );
    push_section(
        &mut out,
        t(lang, "响应头", "Response headers"),
        &header_block(&flow.response_headers),
    );
    push_section(
        &mut out,
        t(lang, "响应 body", "Response body"),
        &body_text(&flow.response_body),
    );
    out
}

fn push_section(out: &mut String, title: &str, body: &str) {
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(title);
    out.push('\n');
    out.push_str(body);
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
    format!(
        "'''{}'''",
        text.replace('\\', "\\\\").replace("'''", "\\'\\'\\'")
    )
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
    if out.is_empty() { "Value".into() } else { out }
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
    if value.len() < 16 || !value.len().is_multiple_of(4) || value.contains('.') {
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
    if text.len() < 8
        || text
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
    {
        return None;
    }
    Some(format!("base64 {text}"))
}

fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    let mut standard = text.replace('-', "+").replace('_', "/");
    while !standard.len().is_multiple_of(4) {
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
/// backwards, both read as `0 ms`.
pub fn format_gap(previous: Option<SystemTime>, started: SystemTime) -> String {
    let Some(previous) = previous else {
        return "0 ms".into();
    };
    match started.duration_since(previous) {
        Ok(duration) => format_delta(duration),
        Err(_) => "0 ms".into(),
    }
}

fn format_delta(duration: Duration) -> String {
    let ms = duration.as_millis();
    if ms < 1_000 {
        return format!("{ms} ms");
    }
    if ms < 10_000 {
        let whole = ms / 1_000;
        let frac = (ms % 1_000) / 10;
        return format!("{whole}.{frac:02} s");
    }
    if ms < 60_000 {
        let whole = ms / 1_000;
        let frac = (ms % 1_000) / 100;
        return format!("{whole}.{frac} s");
    }
    if ms < 3_600_000 {
        // Hundredths of a minute. 60_000 ms is `1.00 m`.
        let hundredths = ms / 600;
        if hundredths < 1_000 {
            return format!("{}.{:02} m", hundredths / 100, hundredths % 100);
        }
        return format!("{} m", hundredths / 100);
    }
    let tenths = ms / 360_000;
    if tenths < 100 {
        format!("{}.{} h", tenths / 10, tenths % 10)
    } else {
        format!("{} h", tenths / 10)
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

/// From the request start until the full response is stored. An unfinished
/// flow has no total yet.
pub fn format_elapsed(started: SystemTime, finished: Option<SystemTime>) -> String {
    let Some(finished) = finished else {
        return "—".into();
    };
    match finished.duration_since(started) {
        Ok(duration) => format_duration(duration),
        Err(_) => "—".into(),
    }
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

pub fn is_form(content_type: Option<&str>, body: &str) -> bool {
    let kind = content_type.unwrap_or("").to_ascii_lowercase();
    kind.contains("x-www-form-urlencoded")
        || (!kind.contains('/') && body.contains('=') && body.contains('&'))
}

pub fn form_pairs(body: &str) -> Vec<(String, String)> {
    body.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(key), percent_decode(value))
        })
        .collect()
}

/// Parses a `multipart/form-data` body into `(field_name, summary_or_value)` pairs
/// when the `Content-Type` header supplies a valid `boundary` parameter.
pub fn multipart_pairs(content_type: Option<&str>, body: &[u8]) -> Option<Vec<(String, String)>> {
    let ct = content_type?;
    if !ct.to_ascii_lowercase().contains("multipart/form-data") {
        return None;
    }
    let boundary = extract_multipart_boundary(ct)?;
    if boundary.is_empty() || body.is_empty() {
        return None;
    }
    let marker = format!("--{boundary}");
    let marker_bytes = marker.as_bytes();
    let mut parts = Vec::new();
    let mut cursor = 0usize;
    while let Some(pos) = find_subslice(&body[cursor..], marker_bytes) {
        let start = cursor + pos + marker_bytes.len();
        if body[start..].starts_with(b"--") {
            break;
        }
        let after_line = if body[start..].starts_with(b"\r\n") {
            start + 2
        } else if body[start..].starts_with(b"\n") {
            start + 1
        } else {
            cursor = start;
            continue;
        };
        let Some(next_rel) = find_subslice(&body[after_line..], marker_bytes) else {
            break;
        };
        let mut part_end = after_line + next_rel;
        if part_end >= after_line + 2 && &body[part_end - 2..part_end] == b"\r\n" {
            part_end -= 2;
        } else if part_end > after_line && body[part_end - 1] == b'\n' {
            part_end -= 1;
        }
        let part = &body[after_line..part_end];
        if let Some(pair) = parse_multipart_part(part) {
            parts.push(pair);
        }
        cursor = after_line + next_rel;
    }
    if parts.is_empty() { None } else { Some(parts) }
}

fn extract_multipart_boundary(content_type: &str) -> Option<String> {
    for segment in content_type.split(';').skip(1) {
        let segment = segment.trim();
        let Some((key, value)) = segment.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("boundary") {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn parse_multipart_part(part: &[u8]) -> Option<(String, String)> {
    let (head_bytes, payload) = if let Some(pos) = find_subslice(part, b"\r\n\r\n") {
        (&part[..pos], &part[pos + 4..])
    } else {
        let pos = find_subslice(part, b"\n\n")?;
        (&part[..pos], &part[pos + 2..])
    };
    let head_str = std::str::from_utf8(head_bytes).ok()?;
    let mut name = String::new();
    let mut filename: Option<String> = None;
    let mut part_ct: Option<String> = None;
    for line in head_str.lines() {
        let line = line.trim();
        let Some((h_name, h_val)) = line.split_once(':') else {
            continue;
        };
        let h_name = h_name.trim();
        let h_val = h_val.trim();
        if h_name.eq_ignore_ascii_case("content-disposition") {
            for param in h_val.split(';').skip(1) {
                let Some((k, v)) = param.trim().split_once('=') else {
                    continue;
                };
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                if k.eq_ignore_ascii_case("name") {
                    name = v.to_string();
                } else if k.eq_ignore_ascii_case("filename") {
                    filename = Some(v.to_string());
                }
            }
        } else if h_name.eq_ignore_ascii_case("content-type") && !h_val.is_empty() {
            part_ct = Some(h_val.to_string());
        }
    }
    if name.is_empty() {
        name = filename.clone().unwrap_or_else(|| "part".to_string());
    }
    let value = match (filename, std::str::from_utf8(payload)) {
        (Some(fname), Ok(text)) if payload.len() <= 256 && !text.contains('\0') => {
            let ct_note = part_ct.as_deref().unwrap_or("text/plain");
            if text.is_empty() {
                format!("[{fname} · {ct_note} · 0 B]")
            } else {
                format!(
                    "[{fname} · {ct_note} · {}] {text}",
                    format_count(payload.len() as u64)
                )
            }
        }
        (Some(fname), _) => {
            let ct_note = part_ct.as_deref().unwrap_or("application/octet-stream");
            format!(
                "[{fname} · {ct_note} · {}]",
                format_count(payload.len() as u64)
            )
        }
        (None, Ok(text)) => text.to_string(),
        (None, Err(_)) => format!(
            "[{} · {}]",
            part_ct.as_deref().unwrap_or("binary"),
            format_count(payload.len() as u64)
        ),
    };
    Some((name, value))
}

fn pretty_form(body: &str) -> String {
    form_pairs(body)
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
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
    if out.is_empty() {
        body.to_string()
    } else {
        out
    }
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
    String::from_utf8(out)
        .unwrap_or_else(|err| String::from_utf8_lossy(err.as_bytes()).into_owned())
}

/// Formats a GraphQL-over-JSON request payload (`{"query": "...", "variables": {...}}`)
/// into a readable multi-line GraphQL block followed by pretty-printed variables.
pub fn pretty_graphql_json(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    let map = value.as_object()?;
    let query = map.get("query")?.as_str()?.trim();
    if !looks_like_graphql_query(query) {
        return None;
    }
    let only_gql_keys = map.keys().all(|k| {
        matches!(
            k.as_str(),
            "query" | "operationName" | "variables" | "extensions"
        )
    });
    if !only_gql_keys {
        return None;
    }
    let mut out = String::new();
    if let Some(op) = map
        .get("operationName")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push_str("# operationName: ");
        out.push_str(op);
        out.push('\n');
    }
    out.push_str(&format_graphql_query(query));
    if let Some(vars) = map.get("variables")
        && !vars.is_null()
        && vars.as_object().is_none_or(|o| !o.is_empty())
        && let Ok(pretty_vars) = serde_json::to_string_pretty(vars)
    {
        out.push_str("\n\n# variables\n");
        out.push_str(&pretty_vars);
    }
    if let Some(ext) = map.get("extensions")
        && !ext.is_null()
        && ext.as_object().is_none_or(|o| !o.is_empty())
        && let Ok(pretty_ext) = serde_json::to_string_pretty(ext)
    {
        out.push_str("\n\n# extensions\n");
        out.push_str(&pretty_ext);
    }
    Some(out)
}

fn looks_like_graphql_query(query: &str) -> bool {
    let trimmed = query.trim_start();
    if !trimmed.contains('{') || !trimmed.contains('}') {
        return false;
    }
    trimmed.starts_with('{')
        || trimmed.starts_with('#')
        || ["query", "mutation", "subscription", "fragment"]
            .iter()
            .any(|kw| {
                trimmed.strip_prefix(kw).is_some_and(|rest| {
                    rest.starts_with(|c: char| c.is_whitespace() || c == '{' || c == '(')
                })
            })
}

fn format_graphql_query(query: &str) -> String {
    let mut out = String::new();
    let mut depth: usize = 0;
    let mut paren_depth: usize = 0;
    let mut in_string = false;
    let mut escaped = false;
    let mut in_comment = false;
    let mut at_line_start = true;
    let mut pending_space = false;

    let push_indent = |buf: &mut String, d: usize| {
        for _ in 0..d {
            buf.push_str("  ");
        }
    };

    for ch in query.chars() {
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                out.push('\n');
                at_line_start = true;
                pending_space = false;
            } else if ch != '\r' {
                out.push(ch);
            }
            continue;
        }
        if in_string {
            if at_line_start {
                push_indent(&mut out, depth);
                at_line_start = false;
            } else if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => {
                if at_line_start {
                    push_indent(&mut out, depth);
                    at_line_start = false;
                } else if pending_space {
                    out.push(' ');
                    pending_space = false;
                }
                in_string = true;
                out.push('"');
            }
            '#' => {
                if at_line_start {
                    push_indent(&mut out, depth);
                    at_line_start = false;
                } else if pending_space {
                    out.push(' ');
                    pending_space = false;
                }
                in_comment = true;
                out.push('#');
            }
            '(' => {
                if at_line_start {
                    push_indent(&mut out, depth);
                    at_line_start = false;
                }
                pending_space = false;
                paren_depth += 1;
                out.push('(');
            }
            ')' => {
                paren_depth = paren_depth.saturating_sub(1);
                pending_space = false;
                out.push(')');
            }
            '{' if paren_depth == 0 => {
                if at_line_start {
                    push_indent(&mut out, depth);
                } else if !out.ends_with(' ') && !out.ends_with('\n') {
                    out.push(' ');
                }
                out.push_str("{\n");
                depth += 1;
                at_line_start = true;
                pending_space = false;
            }
            '}' if paren_depth == 0 => {
                depth = depth.saturating_sub(1);
                if !at_line_start {
                    out.push('\n');
                }
                push_indent(&mut out, depth);
                out.push_str("}\n");
                at_line_start = true;
                pending_space = false;
            }
            '\n' | ',' if paren_depth == 0 => {
                if !at_line_start {
                    out.push('\n');
                    at_line_start = true;
                }
                pending_space = false;
            }
            ',' => {
                out.push_str(", ");
                pending_space = false;
            }
            c if c.is_whitespace() => {
                if !at_line_start {
                    pending_space = true;
                }
            }
            c => {
                if at_line_start {
                    push_indent(&mut out, depth);
                    at_line_start = false;
                } else if pending_space {
                    out.push(' ');
                    pending_space = false;
                }
                out.push(c);
            }
        }
    }
    out.trim().to_string()
}

pub fn is_protobuf(content_type: Option<&str>) -> bool {
    let Some(ct) = content_type else {
        return false;
    };
    let lower = ct.to_ascii_lowercase();
    lower.contains("protobuf")
        || lower.contains("application/grpc")
        || lower.contains("x-google-protobuf")
}

/// Decodes a binary Protobuf or gRPC wire payload without requiring a `.proto` schema.
pub fn decode_protobuf_wire(content_type: Option<&str>, body: &[u8]) -> Option<String> {
    if !is_protobuf(content_type) || body.is_empty() {
        return None;
    }
    let lower = content_type.unwrap_or("").to_ascii_lowercase();
    let is_grpc = lower.contains("grpc");
    if is_grpc && body.len() >= 5 && (body[0] == 0 || body[0] == 1) {
        let mut cursor = 0usize;
        let mut frames = Vec::new();
        let mut valid_frames = true;
        while cursor + 5 <= body.len() {
            let compressed = body[cursor];
            if compressed > 1 {
                valid_frames = false;
                break;
            }
            let frame_len =
                u32::from_be_bytes(body[cursor + 1..cursor + 5].try_into().ok()?) as usize;
            if cursor + 5 + frame_len > body.len() {
                valid_frames = false;
                break;
            }
            let payload = &body[cursor + 5..cursor + 5 + frame_len];
            frames.push((compressed == 1, payload));
            cursor += 5 + frame_len;
        }
        if valid_frames && cursor == body.len() && !frames.is_empty() {
            let mut out = String::new();
            for (idx, (compressed, payload)) in frames.into_iter().enumerate() {
                if idx > 0 {
                    out.push_str("\n\n");
                }
                if compressed {
                    out.push_str(&format!(
                        "# gRPC frame #{} (compressed, {} B)",
                        idx + 1,
                        payload.len()
                    ));
                } else if payload.is_empty() {
                    out.push_str(&format!("# gRPC frame #{} (0 B)", idx + 1));
                } else {
                    let mut decoded = String::new();
                    if decode_proto_fields(payload, 0, &mut decoded) {
                        if idx > 0 {
                            out.push_str(&format!("# gRPC frame #{}\n", idx + 1));
                        }
                        out.push_str(decoded.trim_end());
                    } else {
                        return None;
                    }
                }
            }
            return (!out.is_empty()).then_some(out);
        }
    }
    let mut out = String::new();
    if decode_proto_fields(body, 0, &mut out) && !out.is_empty() {
        Some(out.trim_end().to_string())
    } else {
        None
    }
}

fn decode_proto_fields(bytes: &[u8], depth: usize, out: &mut String) -> bool {
    if depth > 6 || bytes.is_empty() {
        return false;
    }
    let mut cursor = 0usize;
    let mut count = 0usize;
    while cursor < bytes.len() {
        let Some((tag, next)) = read_varint(bytes, cursor) else {
            return false;
        };
        cursor = next;
        let field_num = tag >> 3;
        let wire_type = (tag & 0x07) as u8;
        if field_num == 0 || field_num > 536_870_911 {
            return false;
        }
        let indent = "  ".repeat(depth);
        match wire_type {
            0 => {
                let Some((val, next)) = read_varint(bytes, cursor) else {
                    return false;
                };
                cursor = next;
                out.push_str(&format!("{indent}field #{field_num} (varint): {val}\n"));
            }
            1 => {
                if cursor + 8 > bytes.len() {
                    return false;
                }
                let raw: [u8; 8] = bytes[cursor..cursor + 8].try_into().unwrap();
                cursor += 8;
                let u_val = u64::from_le_bytes(raw);
                out.push_str(&format!("{indent}field #{field_num} (fixed64): {u_val}\n"));
            }
            2 => {
                let Some((len_u64, next)) = read_varint(bytes, cursor) else {
                    return false;
                };
                let len = len_u64 as usize;
                cursor = next;
                if cursor + len > bytes.len() {
                    return false;
                }
                let sub = &bytes[cursor..cursor + len];
                cursor += len;
                if let Ok(s) = std::str::from_utf8(sub)
                    && !s.is_empty()
                    && s.chars()
                        .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
                {
                    out.push_str(&format!("{indent}field #{field_num} (string): {s:?}\n"));
                } else if !sub.is_empty() && depth < 5 {
                    let mut nested = String::new();
                    if decode_proto_fields(sub, depth + 1, &mut nested) && !nested.is_empty() {
                        out.push_str(&format!(
                            "{indent}field #{field_num} (message): {{\n{nested}{indent}}}\n"
                        ));
                    } else {
                        out.push_str(&format!(
                            "{indent}field #{field_num} (bytes, {len} B): {}\n",
                            hex_preview(sub, 24)
                        ));
                    }
                } else {
                    out.push_str(&format!(
                        "{indent}field #{field_num} (bytes, {len} B): {}\n",
                        hex_preview(sub, 24)
                    ));
                }
            }
            5 => {
                if cursor + 4 > bytes.len() {
                    return false;
                }
                let raw: [u8; 4] = bytes[cursor..cursor + 4].try_into().unwrap();
                cursor += 4;
                let u_val = u32::from_le_bytes(raw);
                out.push_str(&format!("{indent}field #{field_num} (fixed32): {u_val}\n"));
            }
            _ => return false,
        }
        count += 1;
        if out.len() > 256 * 1024 {
            break;
        }
    }
    count > 0 && cursor == bytes.len()
}

fn read_varint(bytes: &[u8], mut cursor: usize) -> Option<(u64, usize)> {
    let mut result = 0u64;
    let mut shift = 0u32;
    while cursor < bytes.len() {
        let b = bytes[cursor];
        cursor += 1;
        result |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some((result, cursor));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

fn hex_preview(bytes: &[u8], limit: usize) -> String {
    if bytes.is_empty() {
        return "\"\"".into();
    }
    let shown = bytes.len().min(limit);
    let mut hex = bytes[..shown]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    if bytes.len() > shown {
        hex.push_str(" …");
    }
    hex
}

pub fn is_msgpack(content_type: Option<&str>) -> bool {
    content_type.is_some_and(|ct| ct.to_ascii_lowercase().contains("msgpack"))
}

/// Decodes a binary MessagePack payload into pretty-printed JSON.
pub fn decode_msgpack_to_json(body: &[u8]) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    let mut pos = 0usize;
    let val = read_msgpack_value(body, &mut pos, 0)?;
    if pos != body.len() {
        return None;
    }
    serde_json::to_string_pretty(&val).ok()
}

fn read_msgpack_value(bytes: &[u8], pos: &mut usize, depth: usize) -> Option<serde_json::Value> {
    if depth > 16 || *pos >= bytes.len() {
        return None;
    }
    let marker = bytes[*pos];
    *pos += 1;
    let take = |pos: &mut usize, n: usize| -> Option<&[u8]> {
        if *pos + n > bytes.len() {
            None
        } else {
            let slice = &bytes[*pos..*pos + n];
            *pos += n;
            Some(slice)
        }
    };
    match marker {
        0x00..=0x7f => Some(serde_json::Value::from(marker)),
        0x80..=0x8f => read_msgpack_map(bytes, pos, (marker & 0x0f) as usize, depth + 1),
        0x90..=0x9f => read_msgpack_array(bytes, pos, (marker & 0x0f) as usize, depth + 1),
        0xa0..=0xbf => {
            let s = take(pos, (marker & 0x1f) as usize)?;
            Some(serde_json::Value::String(
                String::from_utf8_lossy(s).into_owned(),
            ))
        }
        0xc0 => Some(serde_json::Value::Null),
        0xc2 => Some(serde_json::Value::Bool(false)),
        0xc3 => Some(serde_json::Value::Bool(true)),
        0xc4 => {
            let n = take(pos, 1)?[0] as usize;
            let b = take(pos, n)?;
            Some(serde_json::Value::String(format!(
                "<bin {n} B: {}>",
                hex_preview(b, 16)
            )))
        }
        0xc5 => {
            let n = u16::from_be_bytes(take(pos, 2)?.try_into().ok()?) as usize;
            let b = take(pos, n)?;
            Some(serde_json::Value::String(format!(
                "<bin {n} B: {}>",
                hex_preview(b, 16)
            )))
        }
        0xc6 => {
            let n = u32::from_be_bytes(take(pos, 4)?.try_into().ok()?) as usize;
            let b = take(pos, n)?;
            Some(serde_json::Value::String(format!(
                "<bin {n} B: {}>",
                hex_preview(b, 16)
            )))
        }
        0xca => {
            let f = f32::from_be_bytes(take(pos, 4)?.try_into().ok()?) as f64;
            serde_json::Number::from_f64(f).map(serde_json::Value::Number)
        }
        0xcb => {
            let f = f64::from_be_bytes(take(pos, 8)?.try_into().ok()?);
            serde_json::Number::from_f64(f).map(serde_json::Value::Number)
        }
        0xcc => Some(serde_json::Value::from(take(pos, 1)?[0])),
        0xcd => Some(serde_json::Value::from(u16::from_be_bytes(
            take(pos, 2)?.try_into().ok()?,
        ))),
        0xce => Some(serde_json::Value::from(u32::from_be_bytes(
            take(pos, 4)?.try_into().ok()?,
        ))),
        0xcf => Some(serde_json::Value::from(u64::from_be_bytes(
            take(pos, 8)?.try_into().ok()?,
        ))),
        0xd0 => Some(serde_json::Value::from(take(pos, 1)?[0] as i8)),
        0xd1 => Some(serde_json::Value::from(i16::from_be_bytes(
            take(pos, 2)?.try_into().ok()?,
        ))),
        0xd2 => Some(serde_json::Value::from(i32::from_be_bytes(
            take(pos, 4)?.try_into().ok()?,
        ))),
        0xd3 => Some(serde_json::Value::from(i64::from_be_bytes(
            take(pos, 8)?.try_into().ok()?,
        ))),
        0xd9 => {
            let n = take(pos, 1)?[0] as usize;
            let s = take(pos, n)?;
            Some(serde_json::Value::String(
                String::from_utf8_lossy(s).into_owned(),
            ))
        }
        0xda => {
            let n = u16::from_be_bytes(take(pos, 2)?.try_into().ok()?) as usize;
            let s = take(pos, n)?;
            Some(serde_json::Value::String(
                String::from_utf8_lossy(s).into_owned(),
            ))
        }
        0xdb => {
            let n = u32::from_be_bytes(take(pos, 4)?.try_into().ok()?) as usize;
            let s = take(pos, n)?;
            Some(serde_json::Value::String(
                String::from_utf8_lossy(s).into_owned(),
            ))
        }
        0xdc => {
            let n = u16::from_be_bytes(take(pos, 2)?.try_into().ok()?) as usize;
            read_msgpack_array(bytes, pos, n, depth + 1)
        }
        0xdd => {
            let n = u32::from_be_bytes(take(pos, 4)?.try_into().ok()?) as usize;
            read_msgpack_array(bytes, pos, n, depth + 1)
        }
        0xde => {
            let n = u16::from_be_bytes(take(pos, 2)?.try_into().ok()?) as usize;
            read_msgpack_map(bytes, pos, n, depth + 1)
        }
        0xdf => {
            let n = u32::from_be_bytes(take(pos, 4)?.try_into().ok()?) as usize;
            read_msgpack_map(bytes, pos, n, depth + 1)
        }
        0xe0..=0xff => Some(serde_json::Value::from(marker as i8)),
        _ => None,
    }
}

fn read_msgpack_array(
    bytes: &[u8],
    pos: &mut usize,
    len: usize,
    depth: usize,
) -> Option<serde_json::Value> {
    if len > 10_000 {
        return None;
    }
    let mut items = Vec::with_capacity(len.min(256));
    for _ in 0..len {
        items.push(read_msgpack_value(bytes, pos, depth)?);
    }
    Some(serde_json::Value::Array(items))
}

fn read_msgpack_map(
    bytes: &[u8],
    pos: &mut usize,
    len: usize,
    depth: usize,
) -> Option<serde_json::Value> {
    if len > 10_000 {
        return None;
    }
    let mut map = serde_json::Map::with_capacity(len.min(256));
    for _ in 0..len {
        let key_val = read_msgpack_value(bytes, pos, depth)?;
        let key = match key_val {
            serde_json::Value::String(s) => s,
            other => other.to_string(),
        };
        let val = read_msgpack_value(bytes, pos, depth)?;
        map.insert(key, val);
    }
    Some(serde_json::Value::Object(map))
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
    fn pretty_headers_sort_without_changing_case() {
        let mut headers = vec![
            ("X-Trace".into(), "1".into()),
            ("accept".into(), "a".into()),
            ("Content-Type".into(), "b".into()),
            ("Accept".into(), "c".into()),
        ];
        sort_headers_pretty(&mut headers);
        assert_eq!(
            headers
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            vec!["accept", "Accept", "Content-Type", "X-Trace"]
        );
        assert_eq!(headers[0].1, "a");
        assert_eq!(headers[1].1, "c");
    }

    #[test]
    fn pretty_cookie_splits_pairs_and_flags() {
        assert_eq!(
            cookie_pairs("session=abc; theme=dark; HttpOnly"),
            vec![
                ("session".into(), "abc".into()),
                ("theme".into(), "dark".into()),
                ("HttpOnly".into(), String::new()),
            ]
        );
        assert_eq!(
            cookie_pairs("id=a; Path=/; Secure"),
            vec![
                ("id".into(), "a".into()),
                ("Path".into(), "/".into()),
                ("Secure".into(), String::new()),
            ]
        );
        let payload = base64url_encode(br#"{"sub":"a"}"#);
        let header = base64url_encode(br#"{"alg":"none"}"#);
        let token = format!("{header}.{payload}.x");
        let rows = pretty_cookie_rows("Cookie", &format!("session={token}; theme=dark")).unwrap();
        assert_eq!(rows[0].0, "Cookie · session");
        assert!(rows[0].1.starts_with(&token));
        assert!(rows[0].1.contains("\"sub\": \"a\""));
        assert_eq!(rows[1].0, "Cookie · theme");
        assert_eq!(rows[1].1, "dark");
        assert!(pretty_cookie_rows("Accept", "text/plain").is_none());
        assert!(pretty_cookie_rows("Set-Cookie", "id=a; Path=/; HttpOnly").is_none());
        assert!(pretty_cookie_rows("Cookie", "   ").is_none());
    }

    #[test]
    fn pretty_image_hides_hex_until_raw() {
        let hex = "89 50 4e 47 ".to_string();
        let note = "解压失败，以下是原始字节。boom\n十六进制只显示前 4096 字节，共 8000 字节。";
        let pretty = image_body(true, true, hex.clone(), note.into(), 128, Lang::Zh);
        assert!(pretty.text.is_empty());
        assert_eq!(pretty.caption, "共 128 字节");
        assert!(pretty.note.contains("解压失败"), "{}", pretty.note);
        assert!(!pretty.note.contains("十六进制"), "{}", pretty.note);
        let raw = image_body(
            false,
            true,
            hex.clone(),
            "Hex shows the first 4096 of 8000 bytes.".into(),
            128,
            Lang::En,
        );
        assert_eq!(raw.text, hex);
        assert!(raw.caption.is_empty());
        assert!(raw.note.starts_with("Hex shows the first "));
        let plain = image_body(true, false, "{ }".into(), String::new(), 3, Lang::En);
        assert_eq!(plain.text, "{ }");
        assert!(plain.caption.is_empty());
    }

    #[test]
    fn form_pairs_decode_and_keep_flags() {
        assert!(is_form(Some("application/x-www-form-urlencoded"), "a=1"));
        assert!(!is_form(Some("application/json"), "a=1&b=2"));
        assert_eq!(
            form_pairs("name=Ada+Lovelace&on=1"),
            vec![
                ("name".into(), "Ada Lovelace".into()),
                ("on".into(), "1".into()),
            ]
        );
    }

    #[test]
    fn preview_keeps_a_small_icon_and_shrinks_a_large_one() {
        let mut png = vec![0u8; 24];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[16..20].copy_from_slice(&80u32.to_be_bytes());
        png[20..24].copy_from_slice(&40u32.to_be_bytes());
        assert_eq!(image_pixel_size(&png), Some((80, 40)));
        assert_eq!(preview_box(80, 40), (80, 40));
        assert_eq!(preview_box(400, 200), (200, 100));
        assert_eq!(preview_box(100, 400), (50, 200));
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&16u16.to_le_bytes());
        gif.extend_from_slice(&32u16.to_le_bytes());
        assert_eq!(image_pixel_size(&gif), Some((16, 32)));
    }

    #[test]
    fn editor_language_follows_the_content_type() {
        assert_eq!(
            editor_language(Some("application/json; charset=utf-8"), "{\"a\":1}"),
            Some("json")
        );
        assert_eq!(editor_language(Some("text/html"), "<p>"), Some("html"));
        assert_eq!(
            editor_language(Some("text/markdown"), "# hi"),
            Some("markdown")
        );
        assert_eq!(editor_language(Some("text/xml"), "<a/>"), Some("xml"));
        assert!(parse_text_view_doc(Some("text/html"), b"<h1>Hello</h1><p>World</p>").is_some());
        assert!(parse_text_view_doc(Some("text/markdown"), b"# Hello\n\n- item").is_some());
        assert!(
            parse_text_view_doc(
                Some("text/html"),
                b"<!doctype html><html><head><script src='a.js'></script></head><body><div id='app'></div></body></html>"
            )
            .is_none()
        );
        assert!(parse_text_view_doc(Some("application/json"), b"{\"a\":1}").is_none());
        let long = "a\n".repeat(50);
        assert_eq!(
            editor_language(Some("text/plain"), &long),
            Some("plaintext")
        );
        assert_eq!(editor_language(Some("image/png"), "not text"), None);
        assert_eq!(editor_language(Some("text/plain"), "short"), None);
        assert_eq!(
            editor_language(Some("text/plain"), "{\"a\":1}"),
            Some("json")
        );
        assert_eq!(editor_language(None, "[\n  1\n]"), Some("json"));
        assert_eq!(editor_language(Some("image/png"), "{\"a\":1}"), None);
        assert_eq!(editor_language(None, ""), None);
    }

    #[test]
    fn gap_and_clock_keep_milliseconds() {
        let start = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(format_elapsed(start, None), "—");
        assert_eq!(
            format_elapsed(start, Some(start + Duration::from_millis(45))),
            "45 ms"
        );
        assert_eq!(
            format_elapsed(start, Some(start + Duration::from_millis(1_240))),
            "1.24 s"
        );
        assert_eq!(format_gap(None, start), "0 ms");
        assert_eq!(
            format_gap(Some(start + Duration::from_secs(2)), start),
            "0 ms"
        );
        assert_eq!(
            format_gap(Some(start), start + Duration::from_millis(45)),
            "45 ms"
        );
        assert_eq!(
            format_gap(Some(start), start + Duration::from_millis(1_240)),
            "1.24 s"
        );
        assert_eq!(
            format_gap(Some(start), start + Duration::from_millis(10_500)),
            "10.5 s"
        );
        assert_eq!(
            format_gap(Some(start), start + Duration::from_millis(90_000)),
            "1.50 m"
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
        let hint =
            annotate_value("Authorization", &format!("Bearer {header}.{payload}.x")).unwrap();
        assert!(hint.contains("\"sub\": \"a\""));
        assert!(hint.contains("2023-11-14 22:13:20 UTC"));
        let stamp = annotate_value("X-Time", "1700000000").unwrap();
        assert!(stamp.contains("2023-11-14 22:13:20 UTC"));
        let encoded = base64_encode(b"hello-zniff");
        let decoded = annotate_value("X-Data", &encoded).unwrap();
        assert!(decoded.contains("hello-zniff"));
        let diff = line_diff("a\nb\nc", "a\nc\nd");
        assert!(
            diff.iter()
                .any(|line| line.kind == DiffKind::Remove && line.text == "b")
        );
        assert!(
            diff.iter()
                .any(|line| line.kind == DiffKind::Add && line.text == "d")
        );
    }

    fn base64url_encode(bytes: &[u8]) -> String {
        base64_encode(bytes)
            .replace('+', "-")
            .replace('/', "_")
            .replace('=', "")
    }

    fn base64_encode(bytes: &[u8]) -> String {
        const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        let mut index = 0;
        while index + 3 <= bytes.len() {
            let n = ((bytes[index] as u32) << 16)
                | ((bytes[index + 1] as u32) << 8)
                | bytes[index + 2] as u32;
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

    #[test]
    fn multipart_form_data_extracts_text_and_file_parts() {
        let ct = Some("multipart/form-data; boundary=----ZniffBoundary7MA4YWxkTrZu0gW");
        let raw = b"------ZniffBoundary7MA4YWxkTrZu0gW\r\n\
Content-Disposition: form-data; name=\"scene\"\r\n\r\n\
avatar\r\n\
------ZniffBoundary7MA4YWxkTrZu0gW\r\n\
Content-Disposition: form-data; name=\"file\"; filename=\"logo.png\"\r\n\
Content-Type: image/png\r\n\r\n\
\x89PNG\r\n\x1a\n\r\n\
------ZniffBoundary7MA4YWxkTrZu0gW--\r\n";
        let pairs = multipart_pairs(ct, raw).expect("multipart pairs");
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0], ("scene".to_string(), "avatar".to_string()));
        assert_eq!(pairs[1].0, "file");
        assert!(pairs[1].1.contains("logo.png"));
        assert!(pairs[1].1.contains("image/png"));
    }

    #[test]
    fn graphql_protobuf_msgpack_and_body_search_work() {
        let gql_json = r#"{"operationName":"GetUser","query":"query GetUser($id: ID!) { user(id: $id) { id name profile { avatar } } }","variables":{"id":"42"}}"#;
        let pretty = pretty_body(Some("application/json"), gql_json);
        assert!(pretty.contains("# operationName: GetUser"));
        assert!(pretty.contains("query GetUser($id: ID!) {"));
        assert!(pretty.contains("  user(id: $id) {"));
        assert!(pretty.contains("    id"));
        assert!(pretty.contains("# variables\n{\n  \"id\": \"42\"\n}"));

        // Protobuf wire: field #1 varint 150 (0x08 0x96 0x01), field #2 string "zniff" (0x12 0x05 b"zniff")
        let proto = b"\x08\x96\x01\x12\x05zniff";
        let decoded_proto =
            pretty_binary_body(Some("application/x-protobuf"), proto).expect("protobuf decode");
        assert!(decoded_proto.contains("field #1 (varint): 150"));
        assert!(decoded_proto.contains("field #2 (string): \"zniff\""));

        // gRPC frame: 0x00 (uncompressed) + 0x0000000a (10 bytes) + proto
        let mut grpc = vec![0x00, 0x00, 0x00, 0x00, proto.len() as u8];
        grpc.extend_from_slice(proto);
        let decoded_grpc =
            pretty_binary_body(Some("application/grpc+proto"), &grpc).expect("grpc decode");
        assert!(decoded_grpc.contains("field #1 (varint): 150"));
        assert!(decoded_grpc.contains("field #2 (string): \"zniff\""));

        // MessagePack: {"ok": true, "n": 7} -> 0x82 (map 2), 0xa2 "ok", 0xc3 (true), 0xa1 "n", 0x07
        let msgpack = b"\x82\xa2ok\xc3\xa1n\x07";
        let decoded_mp =
            pretty_binary_body(Some("application/msgpack"), msgpack).expect("msgpack decode");
        assert!(decoded_mp.contains("\"ok\": true"));
        assert!(decoded_mp.contains("\"n\": 7"));

        let (hits, filtered) = filter_body_lines("alpha\nBeta match\ngamma MATCH two", "match");
        assert_eq!(hits, 2);
        assert_eq!(filtered, "[L2] Beta match\n[L3] gamma MATCH two");
    }
}
