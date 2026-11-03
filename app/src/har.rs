//! HAR 1.2 export and import. Binary bodies are base64.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::session::{Flow, Protocol};

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
}

pub fn export_har(flows: &[Flow]) -> Value {
    json!({
        "log": {
            "version": "1.2",
            "creator": {"name": "zniff", "version": "0.1.0"},
            "entries": flows.iter().map(entry).collect::<Vec<_>>(),
        }
    })
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
    json!({
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
            "dns": -1,
            "connect": -1,
            "ssl": -1,
            "send": 0,
            "wait": wait,
            "receive": total.saturating_sub(wait),
        },
    })
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
        .unwrap_or_else(SystemTime::now);
    let wait = non_negative_ms(entry.get("timings").and_then(|timings| timings.get("wait")));
    let receive = non_negative_ms(entry.get("timings").and_then(|timings| timings.get("receive")));
    let total = non_negative_ms(entry.get("time")).or_else(|| match (wait, receive) {
        (Some(wait), Some(receive)) => Some(wait.saturating_add(receive)),
        (Some(wait), None) => Some(wait),
        _ => None,
    });
    Some(Imported {
        protocol: protocol_from(version),
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
        finished_at: total.map(|ms| started + Duration::from_millis(ms)),
        request_bytes: non_negative_ms(request.get("bodySize")),
        response_bytes: response.and_then(|response| non_negative_ms(response.get("bodySize"))),
    })
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
    let text = post.get("text").and_then(|text| text.as_str()).unwrap_or("");
    if post.get("encoding").and_then(|encoding| encoding.as_str()) == Some("base64") {
        b64_decode(text)
    } else {
        text.as_bytes().to_vec()
    }
}

fn body_from_content(content: &Value) -> Vec<u8> {
    let text = content.get("text").and_then(|text| text.as_str()).unwrap_or("");
    if content.get("encoding").and_then(|encoding| encoding.as_str()) == Some("base64") {
        b64_decode(text)
    } else {
        text.as_bytes().to_vec()
    }
}

fn split_url(url: &str) -> (String, String) {
    let rest = url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url);
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = authority.split('@').next_back().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host);
    let path = if path.is_empty() {
        "/".to_string()
    } else {
        format!("/{path}")
    };
    (host.to_string(), path)
}

fn protocol_from(version: &str) -> Protocol {
    if version.eq_ignore_ascii_case("HTTP/2") || version.eq_ignore_ascii_case("h2") {
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
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let time_of_day = secs % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60
    )
}

fn parse_iso(text: &str) -> Option<SystemTime> {
    let text = text.trim().trim_end_matches('Z');
    let (date, time) = text.split_once('T')?;
    let mut date = date.split('-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: u32 = date.next()?.parse().ok()?;
    let day: u32 = date.next()?.parse().ok()?;
    let time = time.split('.').next().unwrap_or(time);
    let mut clock = time.split(':');
    let hour: u64 = clock.next()?.parse().ok()?;
    let minute: u64 = clock.next()?.parse().ok()?;
    let second: u64 = clock.next()?.parse().ok()?;
    let days = days_from_civil(year, month, day)?;
    let secs = days as u64 * 86_400 + hour * 3600 + minute * 60 + second;
    Some(UNIX_EPOCH + std::time::Duration::from_secs(secs))
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
        let n = ((bytes[index] as u32) << 16) | ((bytes[index + 1] as u32) << 8) | bytes[index + 2] as u32;
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
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index + 4 <= bytes.len() {
        if bytes[index] == b'=' {
            break;
        }
        let Some(a) = val(bytes[index]) else { break };
        let Some(b) = val(bytes[index + 1]) else { break };
        let c = val(bytes[index + 2]);
        let d = val(bytes[index + 3]);
        let n = ((a as u32) << 18) | ((b as u32) << 12) | ((c.unwrap_or(0) as u32) << 6) | d.unwrap_or(0) as u32;
        out.push((n >> 16) as u8);
        if bytes[index + 2] != b'=' {
            out.push((n >> 8) as u8);
        }
        if bytes[index + 3] != b'=' {
            out.push(n as u8);
        }
        index += 4;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Direction, FlowDraft, Session};

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
        let flow = session.get(id).unwrap().clone();
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
        let mut har = export_har(&[flow]);
        har["log"]["entries"][0]["request"]["bodySize"] = json!(18);
        har["log"]["entries"][0]["response"]["bodySize"] = json!(4);
        let imported = import_har(&har);
        assert_eq!(imported.len(), 1);
        let started = imported[0].started;
        assert_eq!(
            imported[0].response_at.unwrap().duration_since(started).unwrap(),
            Duration::from_millis(15)
        );
        assert_eq!(
            imported[0].finished_at.unwrap().duration_since(started).unwrap(),
            Duration::from_millis(40)
        );
        assert_eq!(imported[0].request_bytes, Some(18));
        assert_eq!(imported[0].response_bytes, Some(4));
        assert_eq!(imported[0].request_body, b"hi");
    }
}
