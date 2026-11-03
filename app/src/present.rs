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

fn format_clock(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let local = secs + local_offset_secs();
    let day = local.rem_euclid(86_400);
    let hour = day / 3600;
    let minute = (day % 3600) / 60;
    let second = day % 60;
    format!("{hour:02}:{minute:02}:{second:02}")
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
    fn port_rejects_zero_and_blank() {
        assert_eq!(parse_port("8877"), Some(8877));
        assert_eq!(parse_port(" 80 "), Some(80));
        assert_eq!(parse_port("0"), None);
        assert_eq!(parse_port(""), None);
        assert_eq!(parse_port("65536"), None);
    }
}
