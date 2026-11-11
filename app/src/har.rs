//! HAR 1.2 export and import. Binary bodies are base64.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::session::{ConnInfo, Direction, Flow, Message, Protocol};

const WS_MESSAGE_CAP: usize = 200;
const WS_BYTE_CAP: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct Imported {
    pub protocol: Protocol,
    pub method: String,
    pub host: String,
    pub path: String,
    pub url: String,
    pub status: Option<u16>,
    pub error: Option<String>,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
    pub request_body: Vec<u8>,
    pub response_body: Vec<u8>,
    pub started: SystemTime,
    pub response_at: Option<SystemTime>,
    pub finished_at: Option<SystemTime>,
    /// `None` when HAR `bodySize` is missing or negative.
    pub request_bytes: Option<u64>,
    pub response_bytes: Option<u64>,
    pub comment: Option<String>,
    pub conn_info: ConnInfo,
    #[cfg_attr(not(test), allow(dead_code))]
    pub messages: Vec<Message>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub messages_truncated: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    pub message_times: Vec<Option<SystemTime>>,
}

#[cfg(test)]
pub fn export_har(flows: &[Flow]) -> Value {
    export_har_with_redact(flows, &crate::share::RedactOptions::default())
}

pub fn export_har_with_redact(flows: &[Flow], redact: &crate::share::RedactOptions) -> Value {
    let entries: Vec<Value> = if redact.enabled {
        flows
            .iter()
            .map(|f| {
                let (sanitized, _) = crate::share::redact_flow(f, redact);
                entry(&sanitized)
            })
            .collect()
    } else {
        flows.iter().map(entry).collect()
    };
    let mut log = json!({
        "version": "1.2",
        "creator": {"name": "zniff", "version": env!("CARGO_PKG_VERSION")},
        "entries": entries,
    });
    if redact.enabled {
        log["_redacted"] = json!(true);
    }
    json!({ "log": log })
}

pub fn import_har(value: &Value) -> Vec<Imported> {
    value
        .get("log")
        .and_then(|log| log.get("entries"))
        .and_then(|entries| entries.as_array())
        .map(|entries| entries.iter().filter_map(import_entry).collect())
        .unwrap_or_default()
}

fn entry(flow: &Flow) -> Value {
    let wait = flow
        .response_at
        .and_then(|at| at.duration_since(flow.started).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    let total = flow
        .finished_at
        .and_then(|at| at.duration_since(flow.started).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(wait);
    let mut obj = json!({
        "startedDateTime": iso_utc(flow.started),
        "time": total,
        "request": message(
            &flow.method,
            &flow.url,
            flow.protocol,
            &flow.request_headers,
            &flow.request_body,
            true,
        ),
        "response": {
            "status": flow.status.unwrap_or(0),
            "statusText": flow.error.clone().unwrap_or_default(),
            "httpVersion": http_version(flow.protocol),
            "headers": header_json(&flow.response_headers),
            "content": content(&flow.response_headers, &flow.response_body),
            "headersSize": -1,
            "bodySize": flow.response_body.len() as i64,
        },
        "cache": {},
        "timings": {
            "blocked": -1,
            "dns": flow.conn_info.dns_ms.map(|v| v as i64).unwrap_or(-1),
            "connect": flow.conn_info.tcp_ms.map(|v| v as i64).unwrap_or(-1),
            "ssl": flow.conn_info.tls_ms.map(|v| v as i64).unwrap_or(-1),
            "send": 0,
            "wait": wait,
            "receive": total.saturating_sub(wait),
        },
    });
    if !flow.conn_info.server_addr.is_empty() {
        let ip_only = flow
            .conn_info
            .server_addr
            .parse::<std::net::SocketAddr>()
            .map(|a| a.ip().to_string())
            .unwrap_or_else(|_| flow.conn_info.server_addr.clone());
        obj["serverIPAddress"] = json!(ip_only);
        obj["_serverAddress"] = json!(flow.conn_info.server_addr);
    }
    if !flow.conn_info.tls_version.is_empty() {
        obj["_tlsVersion"] = json!(flow.conn_info.tls_version);
    }
    if flow.conn_info.reused {
        obj["_reusedConnection"] = json!(true);
    }
    if flow.conn_info.dialed_before_start {
        obj["_dialedBeforeStart"] = json!(true);
    }
    if !flow.comment.is_empty() {
        obj["comment"] = json!(flow.comment);
    }
    if should_export_ws_messages(flow) {
        let base_time = epoch_secs_ms(flow.response_at.unwrap_or(flow.started));
        let ws_messages: Vec<Value> = flow
            .messages
            .iter()
            .map(|msg| {
                json!({
                    "type": match msg.direction {
                        Direction::Request => "send",
                        Direction::Response => "receive",
                    },
                    "time": base_time,
                    "opcode": ws_opcode_from_kind(&msg.kind),
                    "data": msg.text,
                })
            })
            .collect();
        obj["_webSocketMessages"] = Value::Array(ws_messages);
        if flow.messages_truncated {
            obj["_webSocketMessagesTruncated"] = json!(true);
        }
    }
    obj
}

fn message(
    method: &str,
    url: &str,
    protocol: Protocol,
    headers: &[(String, String)],
    body: &[u8],
    request: bool,
) -> Value {
    let mut value = json!({
        "method": method,
        "url": url,
        "httpVersion": http_version(protocol),
        "headers": header_json(headers),
        "queryString": crate::present::query_pairs(url)
            .into_iter()
            .map(|(name, value)| json!({"name": name, "value": value}))
            .collect::<Vec<_>>(),
        "headersSize": -1,
        "bodySize": body.len() as i64,
    });
    if request && !body.is_empty() {
        let mime = crate::present::content_type(headers).unwrap_or("");
        value["postData"] = if let Ok(text) = std::str::from_utf8(body) {
            json!({"mimeType": mime, "text": text})
        } else {
            json!({"mimeType": mime, "text": b64(body), "encoding": "base64"})
        };
    }
    value
}

fn content(headers: &[(String, String)], body: &[u8]) -> Value {
    let mime = crate::present::content_type(headers).unwrap_or("");
    if let Ok(text) = std::str::from_utf8(body) {
        json!({"mimeType": mime, "size": body.len(), "text": text})
    } else {
        json!({"mimeType": mime, "size": body.len(), "text": b64(body), "encoding": "base64"})
    }
}

fn header_json(headers: &[(String, String)]) -> Value {
    headers
        .iter()
        .map(|(name, value)| json!({"name": name, "value": value}))
        .collect::<Vec<_>>()
        .into()
}

fn import_entry(entry: &Value) -> Option<Imported> {
    let request = entry.get("request")?;
    let url = request.get("url")?.as_str()?.to_string();
    let (host, path) = split_url(&url);
    let response = entry.get("response");
    let status = response
        .and_then(|response| response.get("status"))
        .and_then(|status| status.as_u64())
        .filter(|status| *status > 0)
        .map(|status| status as u16);
    let version = request
        .get("httpVersion")
        .and_then(|version| version.as_str())
        .unwrap_or("HTTP/1.1");
    let error = response
        .and_then(|response| response.get("statusText"))
        .and_then(|text| text.as_str())
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    let started = entry
        .get("startedDateTime")
        .and_then(|value| value.as_str())
        .and_then(parse_iso)
        .unwrap_or_else(crate::clock::now);
    let timings = entry.get("timings");
    let wait = non_negative_ms(timings.and_then(|timings| timings.get("wait")));
    let receive = non_negative_ms(timings.and_then(|timings| timings.get("receive")));
    let dns_ms = non_negative_ms(timings.and_then(|t| t.get("dns")));
    let tcp_ms = non_negative_ms(timings.and_then(|t| t.get("connect")));
    let tls_ms = non_negative_ms(timings.and_then(|t| t.get("ssl")));
    let server_addr = entry
        .get("_serverAddress")
        .or_else(|| entry.get("serverIPAddress"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
        .to_string();
    let tls_version = entry
        .get("_tlsVersion")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
        .to_string();
    let reused = entry
        .get("_reusedConnection")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let dialed_before_start = entry
        .get("_dialedBeforeStart")
        .and_then(Value::as_bool)
        .unwrap_or(entry.get("_serverAddress").is_none());
    let total = non_negative_ms(entry.get("time")).or_else(|| match (wait, receive) {
        (Some(wait), Some(receive)) => Some(wait.saturating_add(receive)),
        (Some(wait), None) => Some(wait),
        _ => None,
    });
    let ws = import_ws_messages(entry);
    let protocol = if status == Some(101) || is_ws_url(&url) || ws.has_entries {
        Protocol::WebSocket
    } else {
        protocol_from(version)
    };
    let base_finished = total.map(|ms| started + Duration::from_millis(ms));
    let finished_at = match (base_finished, ws.latest_time) {
        (Some(fin), Some(last)) if last > fin => Some(last),
        (None, Some(last)) if last >= started => Some(last),
        (fin, _) => fin,
    };
    Some(Imported {
        protocol,
        method: request
            .get("method")
            .and_then(|method| method.as_str())
            .unwrap_or("GET")
            .to_string(),
        host,
        path,
        url,
        status,
        error,
        request_headers: headers_from(request),
        response_headers: response.map(headers_from).unwrap_or_default(),
        request_body: request
            .get("postData")
            .map(body_from_post)
            .unwrap_or_default(),
        response_body: response
            .and_then(|response| response.get("content"))
            .map(body_from_content)
            .unwrap_or_default(),
        started,
        response_at: wait.map(|ms| started + Duration::from_millis(ms)),
        finished_at,
        request_bytes: non_negative_ms(request.get("bodySize")),
        response_bytes: response.and_then(|response| non_negative_ms(response.get("bodySize"))),
        comment: entry
            .get("comment")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        conn_info: ConnInfo {
            dns_ms,
            tcp_ms,
            tls_ms,
            reused,
            server_addr,
            tls_version,
            dialed_before_start,
        },
        messages: ws.messages,
        messages_truncated: ws.truncated,
        message_times: ws.message_times,
    })
}

struct ImportedWs {
    has_entries: bool,
    messages: Vec<Message>,
    message_times: Vec<Option<SystemTime>>,
    truncated: bool,
    latest_time: Option<SystemTime>,
}

fn import_ws_messages(entry: &Value) -> ImportedWs {
    let explicit_truncated = entry
        .get("_webSocketMessagesTruncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let Some(arr) = entry.get("_webSocketMessages").and_then(Value::as_array) else {
        return ImportedWs {
            has_entries: false,
            messages: Vec::new(),
            message_times: Vec::new(),
            truncated: explicit_truncated,
            latest_time: None,
        };
    };
    let has_entries = !arr.is_empty();
    let mut messages = Vec::with_capacity(arr.len().min(WS_MESSAGE_CAP));
    let mut message_times = Vec::with_capacity(arr.len().min(WS_MESSAGE_CAP));
    let mut truncated = explicit_truncated;
    let mut used_bytes = 0usize;
    let mut latest_time: Option<SystemTime> = None;

    for item in arr {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let msg_time = parse_ws_time(obj.get("time"));
        if let Some(t) = msg_time {
            latest_time = Some(latest_time.map_or(t, |prev| prev.max(t)));
        }
        if messages.len() >= WS_MESSAGE_CAP || used_bytes >= WS_BYTE_CAP {
            truncated = true;
            continue;
        }
        let direction = match obj.get("type").and_then(Value::as_str).unwrap_or("receive") {
            s if s.eq_ignore_ascii_case("send") => Direction::Request,
            _ => Direction::Response,
        };
        let opcode = obj.get("opcode").and_then(Value::as_u64).unwrap_or(1);
        let kind = ws_kind_from_opcode(opcode).to_string();
        let mut text = obj
            .get("data")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let room = WS_BYTE_CAP - used_bytes;
        if text.len() > room {
            let end = text.floor_char_boundary(room);
            text.truncate(end);
            truncated = true;
        }
        used_bytes += text.len();
        messages.push(Message {
            direction,
            kind,
            text,
        });
        message_times.push(msg_time);
    }

    ImportedWs {
        has_entries,
        messages,
        message_times,
        truncated,
        latest_time,
    }
}

fn should_export_ws_messages(flow: &Flow) -> bool {
    !flow.messages.is_empty()
        && (flow.protocol == Protocol::WebSocket
            || flow.status == Some(101)
            || is_ws_url(&flow.url)
            || flow
                .messages
                .iter()
                .any(|m| m.direction == Direction::Request || is_ws_frame_kind(&m.kind)))
}

fn is_ws_url(url: &str) -> bool {
    let trimmed = url.trim_start();
    trimmed
        .get(..5)
        .is_some_and(|p| p.eq_ignore_ascii_case("ws://"))
        || trimmed
            .get(..6)
            .is_some_and(|p| p.eq_ignore_ascii_case("wss://"))
}

fn is_ws_frame_kind(kind: &str) -> bool {
    matches!(
        kind.trim(),
        "文本" | "二进制" | "关闭" | "Ping" | "Pong" | "续帧" | "帧"
    ) || matches!(
        kind.trim().to_ascii_lowercase().as_str(),
        "text" | "binary" | "close" | "ping" | "pong" | "continuation" | "frame"
    )
}

fn ws_opcode_from_kind(kind: &str) -> u8 {
    match kind.trim() {
        "文本" => 1,
        "二进制" => 2,
        "关闭" => 8,
        "Ping" => 9,
        "Pong" => 10,
        other => match other.to_ascii_lowercase().as_str() {
            "binary" => 2,
            "close" => 8,
            "ping" => 9,
            "pong" => 10,
            _ => 1,
        },
    }
}

fn ws_kind_from_opcode(opcode: u64) -> &'static str {
    match opcode {
        1 => "文本",
        2 => "二进制",
        8 => "关闭",
        9 => "Ping",
        10 => "Pong",
        _ => "文本",
    }
}

fn epoch_secs_ms(time: SystemTime) -> f64 {
    let ms = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    (ms as f64) / 1000.0
}

fn parse_ws_time(value: Option<&Value>) -> Option<SystemTime> {
    let secs = value?.as_f64()?;
    if !secs.is_finite() || secs < 0.0 {
        return None;
    }
    let millis = (secs * 1000.0).round();
    if millis > u64::MAX as f64 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_millis(millis as u64))
}

fn non_negative_ms(value: Option<&Value>) -> Option<u64> {
    let number = value?.as_f64()?;
    if !number.is_finite() || number < 0.0 {
        None
    } else {
        Some(number.round() as u64)
    }
}

fn headers_from(value: &Value) -> Vec<(String, String)> {
    value
        .get("headers")
        .and_then(|headers| headers.as_array())
        .map(|headers| {
            headers
                .iter()
                .filter_map(|header| {
                    Some((
                        header.get("name")?.as_str()?.to_string(),
                        header.get("value")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn body_from_post(post: &Value) -> Vec<u8> {
    let text = post
        .get("text")
        .and_then(|text| text.as_str())
        .unwrap_or("");
    if post.get("encoding").and_then(|encoding| encoding.as_str()) == Some("base64") {
        b64_decode(text)
    } else {
        text.as_bytes().to_vec()
    }
}

fn body_from_content(content: &Value) -> Vec<u8> {
    let text = content
        .get("text")
        .and_then(|text| text.as_str())
        .unwrap_or("");
    if content
        .get("encoding")
        .and_then(|encoding| encoding.as_str())
        == Some("base64")
    {
        b64_decode(text)
    } else {
        text.as_bytes().to_vec()
    }
}

fn split_url(url: &str) -> (String, String) {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let (authority, path) = match (rest.find('/'), rest.find('?')) {
        (Some(slash), Some(q)) if q < slash => (&rest[..q], format!("/{}", &rest[q..])),
        (Some(slash), _) => {
            let tail = &rest[slash + 1..];
            let p = if tail.is_empty() {
                "/".to_string()
            } else {
                format!("/{tail}")
            };
            (&rest[..slash], p)
        }
        (None, Some(q)) => (&rest[..q], format!("/{}", &rest[q..])),
        (None, None) => (rest, "/".to_string()),
    };
    let host_port = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    let host = if let Some(after_open) = host_port.strip_prefix('[') {
        after_open
            .split_once(']')
            .map(|(ipv6, _)| ipv6)
            .unwrap_or(host_port)
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };
    (host.to_string(), path)
}

fn protocol_from(version: &str) -> Protocol {
    let version = version.trim();
    if version.eq_ignore_ascii_case("h2")
        || version.eq_ignore_ascii_case("HTTP/2")
        || version.eq_ignore_ascii_case("HTTP/2.0")
    {
        Protocol::Http2
    } else {
        Protocol::Http11
    }
}

fn http_version(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Http2 => "HTTP/2",
        Protocol::WebSocket => "HTTP/1.1",
        Protocol::Tunnel => "HTTP/1.1",
        Protocol::Http11 => "HTTP/1.1",
    }
}

fn iso_utc(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();
    let ms = duration.subsec_millis();
    let days = secs / 86_400;
    let time_of_day = secs % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{ms:03}Z",
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60
    )
}

fn parse_tz_offset_secs(tz: &str) -> Option<i64> {
    if tz.is_empty() || tz.eq_ignore_ascii_case("Z") {
        return Some(0);
    }
    let (sign, rest) = if let Some(r) = tz.strip_prefix('+') {
        (1i64, r)
    } else {
        let r = tz.strip_prefix('-')?;
        (-1i64, r)
    };
    let (hours, mins): (u64, u64) = if let Some((h, m)) = rest.split_once(':') {
        if h.len() != 2 || m.len() != 2 {
            return None;
        }
        (h.parse().ok()?, m.parse().ok()?)
    } else if rest.len() == 4 {
        (rest[..2].parse().ok()?, rest[2..].parse().ok()?)
    } else if rest.len() == 2 {
        (rest.parse().ok()?, 0)
    } else {
        return None;
    };
    if hours > 23 || mins > 59 {
        return None;
    }
    Some(sign * (hours as i64 * 3600 + mins as i64 * 60))
}

fn parse_iso(text: &str) -> Option<SystemTime> {
    let text = text.trim();
    let sep = text.find(['T', 't', ' '])?;
    let date = &text[..sep];
    let rest = text[sep + 1..].trim();

    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() {
        return None;
    }

    let (clock_and_frac, offset_secs) =
        if let Some(stripped) = rest.strip_suffix('Z').or_else(|| rest.strip_suffix('z')) {
            (stripped, 0i64)
        } else if let Some(tz_pos) = rest[1..].rfind(['+', '-']).map(|idx| idx + 1) {
            let offset = parse_tz_offset_secs(&rest[tz_pos..])?;
            (&rest[..tz_pos], offset)
        } else {
            (rest, 0i64)
        };

    let (clock_str, nanos) = match clock_and_frac.split_once(['.', ',']) {
        Some((c, frac)) => {
            if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let mut ns = 0u32;
            let mut scale = 100_000_000u32;
            for b in frac.bytes().take(9) {
                ns += (b - b'0') as u32 * scale;
                scale /= 10;
            }
            (c, ns)
        }
        None => (clock_and_frac, 0u32),
    };

    let mut clock = clock_str.split(':');
    let hour: u64 = clock.next()?.parse().ok()?;
    let minute: u64 = clock.next()?.parse().ok()?;
    let second: u64 = clock.next()?.parse().ok()?;
    if clock.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let days = days_from_civil(year, month, day)?;
    let local_secs = days * 86_400 + (hour * 3600 + minute * 60 + second) as i64;
    let utc_secs = local_secs.checked_sub(offset_secs)?;
    if utc_secs < 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::new(utc_secs as u64, nanos))
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    (year as i32, month as u32, day as u32)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = (year - era * 400) as u64;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp as u64 + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe as i64 - 719_468)
}

fn b64(bytes: &[u8]) -> String {
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
        let rest = &bytes[index..];
        let n = match rest.len() {
            1 => (rest[0] as u32) << 16,
            _ => ((rest[0] as u32) << 16) | ((rest[1] as u32) << 8),
        };
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        if rest.len() == 2 {
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
            out.push('=');
        } else {
            out.push('=');
            out.push('=');
        }
    }
    out
}

fn b64_decode(text: &str) -> Vec<u8> {
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
    let clean: Vec<u8> = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    let mut index = 0;
    while index + 4 <= clean.len() {
        if clean[index] == b'=' {
            break;
        }
        let Some(a) = val(clean[index]) else { break };
        let Some(b) = val(clean[index + 1]) else {
            break;
        };
        let c = val(clean[index + 2]);
        let d = val(clean[index + 3]);
        let n = ((a as u32) << 18)
            | ((b as u32) << 12)
            | ((c.unwrap_or(0) as u32) << 6)
            | d.unwrap_or(0) as u32;
        out.push((n >> 16) as u8);
        if clean[index + 2] != b'=' {
            let Some(_) = c else { break };
            out.push((n >> 8) as u8);
        }
        if clean[index + 3] != b'=' {
            let Some(_) = d else { break };
            out.push(n as u8);
        }
        index += 4;
    }
    let rem = &clean[index..];
    if rem.len() == 2 {
        if let (Some(a), Some(b)) = (val(rem[0]), val(rem[1])) {
            let n = ((a as u32) << 18) | ((b as u32) << 12);
            out.push((n >> 16) as u8);
        }
    } else if rem.len() == 3
        && let (Some(a), Some(b), Some(c)) = (val(rem[0]), val(rem[1]), val(rem[2]))
    {
        let n = ((a as u32) << 18) | ((b as u32) << 12) | ((c as u32) << 6);
        out.push((n >> 16) as u8);
        out.push((n >> 8) as u8);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Direction, FlowDraft, Session};

    #[test]
    fn http2_versions_from_chrome_stay_http2() {
        assert_eq!(protocol_from("HTTP/2"), Protocol::Http2);
        assert_eq!(protocol_from("h2"), Protocol::Http2);
        assert_eq!(protocol_from("http/2.0"), Protocol::Http2);
        assert_eq!(protocol_from("HTTP/2.0"), Protocol::Http2);
        assert_eq!(protocol_from("HTTP/1.1"), Protocol::Http11);
    }

    #[test]
    fn round_trip_keeps_method_url_and_json_body() {
        let mut session = Session::default();
        let id = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "POST".into(),
            host: "example.com".into(),
            path: "/v1?q=1".into(),
            url: "https://example.com/v1?q=1".into(),
            request_headers: vec![("Content-Type".into(), "application/json".into())],
        });
        session.append_body(id, Direction::Request, br#"{"a":1}"#);
        session.set_response(
            id,
            201,
            vec![("Content-Type".into(), "application/json".into())],
            None,
        );
        session.append_body(id, Direction::Response, br#"{"ok":true}"#);
        let mut flow = session.get(id).unwrap().clone();
        flow.response_at = Some(flow.started);
        let har = export_har(&[flow]);
        let imported = import_har(&har);
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].method, "POST");
        assert_eq!(imported[0].url, "https://example.com/v1?q=1");
        assert_eq!(imported[0].status, Some(201));
        assert_eq!(imported[0].request_body, br#"{"a":1}"#);
        assert_eq!(imported[0].response_body, br#"{"ok":true}"#);
        assert_eq!(
            imported[0].request_bytes,
            Some(imported[0].request_body.len() as u64)
        );
        assert_eq!(
            imported[0].response_bytes,
            Some(imported[0].response_body.len() as u64)
        );
        assert_eq!(imported[0].response_at, Some(imported[0].started));
        assert_eq!(imported[0].finished_at, Some(imported[0].started));
    }

    #[test]
    fn import_reads_timings_and_body_size() {
        let mut session = Session::default();
        let id = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "example.com".into(),
            path: "/".into(),
            url: "http://example.com/".into(),
            request_headers: Vec::new(),
        });
        session.append_body(id, Direction::Request, b"hi");
        session.append_body(id, Direction::Response, b"ok");
        let mut flow = session.get(id).unwrap().clone();
        flow.response_at = Some(flow.started + Duration::from_millis(15));
        flow.finished_at = Some(flow.started + Duration::from_millis(40));
        flow.comment = "my-flow-note".into();
        let mut har = export_har(&[flow]);
        har["log"]["entries"][0]["request"]["bodySize"] = json!(18);
        har["log"]["entries"][0]["response"]["bodySize"] = json!(4);
        let imported = import_har(&har);
        assert_eq!(imported.len(), 1);
        let started = imported[0].started;
        assert_eq!(
            imported[0]
                .response_at
                .unwrap()
                .duration_since(started)
                .unwrap(),
            Duration::from_millis(15)
        );
        assert_eq!(
            imported[0]
                .finished_at
                .unwrap()
                .duration_since(started)
                .unwrap(),
            Duration::from_millis(40)
        );
        assert_eq!(imported[0].request_bytes, Some(18));
        assert_eq!(imported[0].response_bytes, Some(4));
        assert_eq!(imported[0].request_body, b"hi");
        assert_eq!(imported[0].comment.as_deref(), Some("my-flow-note"));
    }

    #[test]
    fn round_trip_preserves_conn_info_and_phase_timings() {
        let mut session = Session::default();
        let id = session.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "GET".into(),
            host: "api.example.com".into(),
            path: "/items".into(),
            url: "https://api.example.com/items".into(),
            request_headers: Vec::new(),
        });
        session.set_conn_info(
            id,
            ConnInfo {
                dns_ms: Some(12),
                tcp_ms: Some(25),
                tls_ms: Some(48),
                reused: false,
                server_addr: "93.184.216.34:443".into(),
                tls_version: "TLS 1.3".into(),
                dialed_before_start: true,
            },
        );
        let flow = session.get(id).unwrap().clone();
        let har = export_har(std::slice::from_ref(&flow));
        assert_eq!(har["log"]["entries"][0]["timings"]["dns"], json!(12));
        assert_eq!(har["log"]["entries"][0]["timings"]["connect"], json!(25));
        assert_eq!(har["log"]["entries"][0]["timings"]["ssl"], json!(48));
        assert_eq!(
            har["log"]["entries"][0]["serverIPAddress"],
            json!("93.184.216.34")
        );
        let imported = import_har(&har);
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].conn_info, flow.conn_info);
    }

    #[test]
    fn iso_utc_and_parse_iso_preserve_milliseconds_and_timezones() {
        let ts = UNIX_EPOCH + Duration::from_millis(1_731_295_205_789);
        let formatted = iso_utc(ts);
        assert!(formatted.ends_with(".789Z"), "got {formatted}");
        assert_eq!(parse_iso(&formatted), Some(ts));

        // Same instant expressed in +08:00, -05:00, and +0530
        let utc = parse_iso("2026-11-11T03:20:05.250Z").unwrap();
        let cst = parse_iso("2026-11-11T11:20:05.25+08:00").unwrap();
        let est = parse_iso("2026-11-10T22:20:05.250000-05:00").unwrap();
        let ist = parse_iso("2026-11-11T08:50:05.250+0530").unwrap();
        assert_eq!(cst, utc);
        assert_eq!(est, utc);
        assert_eq!(ist, utc);

        let mut session = Session::default();
        let id = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "example.com".into(),
            path: "/".into(),
            url: "https://example.com/".into(),
            request_headers: Vec::new(),
        });
        let mut flow = session.get(id).unwrap().clone();
        flow.response_at = Some(flow.started);
        let har = export_har(&[flow]);
        assert_eq!(
            har["log"]["creator"]["version"],
            json!(env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn b64_decode_skips_ascii_whitespace_and_split_url_handles_ipv6() {
        let raw: Vec<u8> = (0u8..96).collect();
        let encoded = b64(&raw);
        let wrapped = format!(
            "  {}\r\n\t{}\n {}  ",
            &encoded[..40],
            &encoded[40..80],
            &encoded[80..]
        );
        assert_eq!(b64_decode(&wrapped), raw);

        assert_eq!(
            split_url("https://[::1]:8080/v1/items?a=1"),
            ("::1".into(), "/v1/items?a=1".into())
        );
        assert_eq!(
            split_url("http://user:pw@[2001:db8::1]:9090"),
            ("2001:db8::1".into(), "/".into())
        );
        assert_eq!(
            split_url("https://[fe80::1]?ping=1"),
            ("fe80::1".into(), "/?ping=1".into())
        );
    }

    #[test]
    fn websocket_messages_round_trip_losslessly_across_all_opcodes() {
        let mut session = Session::default();
        let id = session.open(FlowDraft {
            protocol: Protocol::WebSocket,
            method: "GET".into(),
            host: "ws.example.com".into(),
            path: "/realtime".into(),
            url: "wss://ws.example.com/realtime".into(),
            request_headers: vec![
                ("Upgrade".into(), "websocket".into()),
                ("Connection".into(), "Upgrade".into()),
            ],
        });
        session.set_response(
            id,
            101,
            vec![("Upgrade".into(), "websocket".into())],
            Some(Protocol::WebSocket),
        );
        session.add_message(id, Direction::Request, "文本", r#"{"sub":"ticker"}"#);
        session.add_message(id, Direction::Response, "二进制", "deadbeef");
        session.add_message(id, Direction::Request, "Ping", "hb");
        session.add_message(id, Direction::Response, "Pong", "hb");
        session.add_message(id, Direction::Response, "关闭", "1000 normal closure");

        let mut flow = session.get(id).unwrap().clone();
        let started_ms = UNIX_EPOCH + Duration::from_millis(1_731_295_200_125);
        let response_ms = started_ms + Duration::from_millis(25);
        flow.started = started_ms;
        flow.response_at = Some(response_ms);
        flow.finished_at = Some(started_ms + Duration::from_millis(900));

        let har = export_har(std::slice::from_ref(&flow));
        let ws_arr = har["log"]["entries"][0]["_webSocketMessages"]
            .as_array()
            .expect("_webSocketMessages emitted");
        assert_eq!(ws_arr.len(), 5);
        assert_eq!(ws_arr[0]["type"], json!("send"));
        assert_eq!(ws_arr[0]["opcode"], json!(1));
        assert_eq!(ws_arr[0]["data"], json!(r#"{"sub":"ticker"}"#));
        assert_eq!(ws_arr[0]["time"], json!(1_731_295_200.15));
        assert_eq!(ws_arr[1]["type"], json!("receive"));
        assert_eq!(ws_arr[1]["opcode"], json!(2));
        assert_eq!(ws_arr[2]["opcode"], json!(9));
        assert_eq!(ws_arr[3]["opcode"], json!(10));
        assert_eq!(ws_arr[4]["opcode"], json!(8));

        let imported = import_har(&har);
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].protocol, Protocol::WebSocket);
        assert_eq!(imported[0].status, Some(101));
        assert_eq!(imported[0].messages, flow.messages);
        assert!(!imported[0].messages_truncated);
        assert_eq!(imported[0].message_times, vec![Some(response_ms); 5]);
    }

    #[test]
    fn import_chrome_devtools_websocket_har_caps_at_200_and_marks_ws_protocol() {
        // 1. Chrome DevTools `_webSocketMessages` on a 101 entry + over-200 cap check
        let mut raw_msgs = Vec::with_capacity(205);
        for idx in 0..205u64 {
            let opcode = match idx {
                0 => 1,
                1 => 2,
                2 => 9,
                3 => 10,
                4 => 99, // unknown opcode falls back to "文本"
                204 => 8,
                _ => 1,
            };
            raw_msgs.push(json!({
                "type": if idx % 2 == 0 { "send" } else { "receive" },
                "time": 1_731_295_300.0 + (idx as f64) * 0.010,
                "opcode": opcode,
                "data": format!("frame-{idx}"),
            }));
        }
        let chrome_har = json!({
            "log": {
                "version": "1.2",
                "entries": [
                    {
                        "startedDateTime": "2024-11-11T03:21:40.000Z",
                        "time": 5,
                        "request": {
                            "method": "GET",
                            "url": "https://chat.example.com/socket",
                            "httpVersion": "http/1.1",
                            "headers": []
                        },
                        "response": {
                            "status": 101,
                            "statusText": "Switching Protocols",
                            "httpVersion": "http/1.1",
                            "headers": [],
                            "content": { "size": 0, "mimeType": "x-unknown" }
                        },
                        "_webSocketMessages": raw_msgs
                    },
                    {
                        "startedDateTime": "2024-11-11T03:21:41.000Z",
                        "time": 2,
                        "request": {
                            "method": "GET",
                            "url": "ws://local.example.com:9000/feed",
                            "httpVersion": "HTTP/1.1",
                            "headers": []
                        },
                        "response": {
                            "status": 0,
                            "statusText": "",
                            "httpVersion": "HTTP/1.1",
                            "headers": [],
                            "content": { "size": 0, "mimeType": "" }
                        }
                    }
                ]
            }
        });

        let imported = import_har(&chrome_har);
        assert_eq!(imported.len(), 2);

        let first = &imported[0];
        assert_eq!(first.protocol, Protocol::WebSocket);
        assert_eq!(first.messages.len(), 200);
        assert!(first.messages_truncated);
        assert_eq!(
            first.messages[0],
            Message {
                direction: Direction::Request,
                kind: "文本".into(),
                text: "frame-0".into(),
            }
        );
        assert_eq!(
            first.messages[1],
            Message {
                direction: Direction::Response,
                kind: "二进制".into(),
                text: "frame-1".into(),
            }
        );
        assert_eq!(first.messages[2].kind, "Ping");
        assert_eq!(first.messages[3].kind, "Pong");
        assert_eq!(first.messages[4].kind, "文本");
        assert_eq!(
            first.message_times[1],
            Some(UNIX_EPOCH + Duration::from_millis(1_731_295_300_010))
        );
        // Latest frame time (idx=204 -> +2040ms) extends finished_at past the 5ms handshake time
        assert_eq!(
            first.finished_at,
            Some(UNIX_EPOCH + Duration::from_millis(1_731_295_302_040))
        );

        // Entry with `ws://` scheme and status 0 (no `_webSocketMessages` yet) is also marked WebSocket
        assert_eq!(imported[1].protocol, Protocol::WebSocket);
    }
}
