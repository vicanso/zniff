//! In-memory capture session.
//!
//! Bodies are already decompressed by the proxy. The 1 MB cap is per
//! direction. The 256 MB cap drops the oldest whole flow.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, SystemTime};

pub const BODY_CAP: usize = 1024 * 1024;
pub const SESSION_CAP: usize = 256 * 1024 * 1024;
/// Pinned flows skip eviction. The cap keeps a pinned set from growing without a bound.
pub const PIN_CAP: usize = 32;
/// Same line as the warning color on a row: elapsed time from start to finish.
pub const SLOW_AFTER: Duration = Duration::from_secs(1);

pub fn is_slow(started: SystemTime, finished: Option<SystemTime>, now: SystemTime) -> bool {
    let end = finished.unwrap_or(now);
    end.duration_since(started)
        .is_ok_and(|elapsed| elapsed >= SLOW_AFTER)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Http11,
    Http2,
    WebSocket,
    Tunnel,
}

impl Protocol {
    pub fn label(self) -> &'static str {
        self.label_in(crate::i18n::Lang::Zh)
    }

    pub fn label_in(self, lang: crate::i18n::Lang) -> &'static str {
        match self {
            Self::Http11 => "HTTP/1.1",
            Self::Http2 => "HTTP/2",
            Self::WebSocket => "WebSocket",
            Self::Tunnel => crate::i18n::t(lang, "隧道", "Tunnel"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Request,
    Response,
}

#[derive(Debug, Clone)]
pub struct Flow {
    #[allow(dead_code)]
    pub id: u64,
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
    pub request_truncated: bool,
    pub response_truncated: bool,
    pub request_encoding_error: Option<String>,
    pub response_encoding_error: Option<String>,
    /// When the request headers were recorded.
    pub started: SystemTime,
    /// When the response headers arrived. `None` until then.
    pub response_at: Option<SystemTime>,
    /// When the response body finished, or the flow failed.
    pub finished_at: Option<SystemTime>,
    /// On-the-wire request body bytes, before decompression.
    pub request_bytes: u64,
    /// On-the-wire response body bytes, before decompression.
    pub response_bytes: u64,
    /// WebSocket frames. They do not count toward the body or session caps.
    pub messages: Vec<Message>,
    pub messages_truncated: bool,
    /// Kept when the session cap drops older flows.
    pub pinned: bool,
    /// Rules that rewrote this flow, joined by spaces. Empty when none did.
    pub rule_hit: String,
    accounted: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub direction: Direction,
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct FlowDraft {
    pub protocol: Protocol,
    pub method: String,
    pub host: String,
    pub path: String,
    pub url: String,
    pub request_headers: Vec<(String, String)>,
}

#[derive(Debug, Default)]
pub struct Session {
    order: VecDeque<u64>,
    flows: HashMap<u64, Flow>,
    bytes: usize,
    dropped: u64,
    next_id: u64,
}

impl Session {
    pub fn open(&mut self, draft: FlowDraft) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let headers_size = headers_size(&draft.request_headers);
        let flow = Flow {
            id,
            protocol: draft.protocol,
            method: draft.method,
            host: draft.host,
            path: draft.path,
            url: draft.url,
            status: None,
            error: None,
            request_headers: draft.request_headers,
            response_headers: Vec::new(),
            request_body: Vec::new(),
            response_body: Vec::new(),
            request_truncated: false,
            response_truncated: false,
            request_encoding_error: None,
            response_encoding_error: None,
            started: SystemTime::now(),
            response_at: None,
            finished_at: None,
            request_bytes: 0,
            response_bytes: 0,
            messages: Vec::new(),
            messages_truncated: false,
            pinned: false,
            rule_hit: String::new(),
            accounted: headers_size,
        };
        self.bytes += headers_size;
        self.order.push_back(id);
        self.flows.insert(id, flow);
        self.evict(id);
        id
    }

    pub fn set_response(
        &mut self,
        id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        protocol: Option<Protocol>,
    ) {
        let extra = headers_size(&headers);
        let Some(flow) = self.flows.get_mut(&id) else {
            return;
        };
        flow.status = Some(status);
        if flow.response_at.is_none() {
            flow.response_at = Some(SystemTime::now());
        }
        flow.response_headers = headers;
        if let Some(protocol) = protocol {
            flow.protocol = protocol;
        }
        flow.accounted += extra;
        self.bytes += extra;
        self.evict(id);
    }

    pub fn set_error(&mut self, id: u64, error: impl Into<String>) {
        if let Some(flow) = self.flows.get_mut(&id) {
            flow.error = Some(error.into());
            if flow.finished_at.is_none() {
                flow.finished_at = Some(SystemTime::now());
            }
        }
    }

    pub fn add_transfer(&mut self, id: u64, direction: Direction, bytes: u64) {
        let Some(flow) = self.flows.get_mut(&id) else {
            return;
        };
        match direction {
            Direction::Request => flow.request_bytes = flow.request_bytes.saturating_add(bytes),
            Direction::Response => flow.response_bytes = flow.response_bytes.saturating_add(bytes),
        }
    }

    pub fn add_message(
        &mut self,
        id: u64,
        direction: Direction,
        kind: impl Into<String>,
        text: impl Into<String>,
    ) {
        const COUNT_CAP: usize = 200;
        const BYTE_CAP: usize = 256 * 1024;
        let Some(flow) = self.flows.get_mut(&id) else {
            return;
        };
        if flow.messages_truncated || flow.messages.len() >= COUNT_CAP {
            flow.messages_truncated = true;
            return;
        }
        let used: usize = flow.messages.iter().map(|message| message.text.len()).sum();
        if used >= BYTE_CAP {
            flow.messages_truncated = true;
            return;
        }
        let mut text = text.into();
        let room = BYTE_CAP - used;
        if text.len() > room {
            let end = text.floor_char_boundary(room);
            text.truncate(end);
            flow.messages_truncated = true;
        }
        flow.messages.push(Message {
            direction,
            kind: kind.into(),
            text,
        });
    }

    pub fn retarget(&mut self, id: u64, host: String, path: String, url: String) {
        if let Some(flow) = self.flows.get_mut(&id) {
            flow.host = host;
            flow.path = path;
            flow.url = url;
        }
    }

    pub fn replace_request(&mut self, id: u64, headers: Vec<(String, String)>, body: Vec<u8>) {
        let Some(flow) = self.flows.get_mut(&id) else {
            return;
        };
        let old = headers_size(&flow.request_headers) + flow.request_body.len();
        let new_size = headers_size(&headers) + body.len();
        flow.request_headers = headers;
        flow.request_body = body;
        flow.accounted = flow.accounted.saturating_sub(old) + new_size;
        self.bytes = self.bytes.saturating_sub(old) + new_size;
    }

    pub fn replace_response(
        &mut self,
        id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) {
        let Some(flow) = self.flows.get_mut(&id) else {
            return;
        };
        let old = headers_size(&flow.response_headers) + flow.response_body.len();
        let new_size = headers_size(&headers) + body.len();
        flow.status = Some(status);
        flow.response_headers = headers;
        flow.response_body = body;
        flow.response_truncated = false;
        if flow.response_at.is_none() {
            flow.response_at = Some(SystemTime::now());
        }
        flow.accounted = flow.accounted.saturating_sub(old) + new_size;
        self.bytes = self.bytes.saturating_sub(old) + new_size;
        self.evict(id);
    }

    /// Pins until [`PIN_CAP`]. Returns false when the flow is missing or the cap is full.
    pub fn set_pinned(&mut self, id: u64, pinned: bool) -> bool {
        if pinned {
            let count = self.flows.values().filter(|flow| flow.pinned).count();
            let Some(flow) = self.flows.get_mut(&id) else {
                return false;
            };
            if flow.pinned {
                return true;
            }
            if count >= PIN_CAP {
                return false;
            }
            flow.pinned = true;
            true
        } else if let Some(flow) = self.flows.get_mut(&id) {
            flow.pinned = false;
            true
        } else {
            false
        }
    }

    pub fn mark_finished(&mut self, id: u64) {
        if let Some(flow) = self.flows.get_mut(&id)
            && flow.finished_at.is_none()
        {
            flow.finished_at = Some(SystemTime::now());
        }
    }

    pub fn set_started(&mut self, id: u64, started: SystemTime) {
        if let Some(flow) = self.flows.get_mut(&id) {
            flow.started = started;
        }
    }

    /// Restores timing and wire sizes after an import. `None` byte counts fall
    /// back to the stored body length. Timestamps overwrite the ones `set_response`
    /// and `mark_finished` just wrote.
    pub fn restore_imported(
        &mut self,
        id: u64,
        response_at: Option<SystemTime>,
        finished_at: Option<SystemTime>,
        request_bytes: Option<u64>,
        response_bytes: Option<u64>,
    ) {
        let Some(flow) = self.flows.get_mut(&id) else {
            return;
        };
        if let Some(at) = response_at {
            flow.response_at = Some(at);
        }
        if let Some(at) = finished_at {
            flow.finished_at = Some(at);
        }
        flow.request_bytes = request_bytes.unwrap_or(flow.request_body.len() as u64);
        flow.response_bytes = response_bytes.unwrap_or(flow.response_body.len() as u64);
    }

    #[cfg(test)]
    pub fn flows(&self) -> Vec<Flow> {
        self.flows_matching("")
    }

    /// Flows that match `filter`, oldest first. An empty filter is the whole session.
    pub fn flows_matching(&self, filter: &str) -> Vec<Flow> {
        self.matching_with_previous_start(filter)
            .into_iter()
            .filter_map(|(id, _)| self.get(id).cloned())
            .collect()
    }

    pub fn clear(&mut self) {
        self.order.clear();
        self.flows.clear();
        self.bytes = 0;
        self.dropped = 0;
    }

    pub fn set_rule_hit(&mut self, id: u64, hit: impl Into<String>) {
        if let Some(flow) = self.flows.get_mut(&id) {
            flow.rule_hit = hit.into();
        }
    }

    #[cfg(test)]
    pub fn ttfb(&self, id: u64) -> Option<Duration> {
        let flow = self.flows.get(&id)?;
        flow.response_at?.duration_since(flow.started).ok()
    }

    #[cfg(test)]
    pub fn duration(&self, id: u64) -> Option<Duration> {
        let flow = self.flows.get(&id)?;
        flow.finished_at?.duration_since(flow.started).ok()
    }

    pub fn set_protocol(&mut self, id: u64, protocol: Protocol) {
        if let Some(flow) = self.flows.get_mut(&id) {
            flow.protocol = protocol;
        }
    }

    pub fn set_encoding_error(&mut self, id: u64, direction: Direction, error: impl Into<String>) {
        let Some(flow) = self.flows.get_mut(&id) else {
            return;
        };
        let slot = match direction {
            Direction::Request => &mut flow.request_encoding_error,
            Direction::Response => &mut flow.response_encoding_error,
        };
        *slot = Some(error.into());
    }

    /// Appends decompressed bytes. Returns whether this direction is now truncated.
    pub fn append_body(&mut self, id: u64, direction: Direction, data: &[u8]) -> bool {
        if data.is_empty() {
            return self
                .flows
                .get(&id)
                .map(|flow| match direction {
                    Direction::Request => flow.request_truncated,
                    Direction::Response => flow.response_truncated,
                })
                .unwrap_or(false);
        }
        let Some(flow) = self.flows.get_mut(&id) else {
            return false;
        };
        let (buf, truncated) = match direction {
            Direction::Request => (&mut flow.request_body, &mut flow.request_truncated),
            Direction::Response => (&mut flow.response_body, &mut flow.response_truncated),
        };
        if *truncated || buf.len() >= BODY_CAP {
            *truncated = true;
            return true;
        }
        let room = BODY_CAP - buf.len();
        let take = room.min(data.len());
        buf.extend_from_slice(&data[..take]);
        let overflow = take < data.len();
        if overflow {
            *truncated = true;
        }
        let added = take;
        flow.accounted += added;
        self.bytes += added;
        let truncated_now = *truncated;
        self.evict(id);
        truncated_now
    }

    pub fn get(&self, id: u64) -> Option<&Flow> {
        self.flows.get(&id)
    }

    pub fn len(&self) -> usize {
        self.flows.len()
    }

    pub fn memory_bytes(&self) -> usize {
        self.bytes
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    #[cfg(test)]
    pub fn ids_matching(&self, filter: &str) -> Vec<u64> {
        self.matching_with_previous_start(filter)
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    /// Matching ids, oldest first. The time is when the previous *visible*
    /// flow started. A row the filter hides does not count.
    pub fn matching_with_previous_start(&self, filter: &str) -> Vec<(u64, Option<SystemTime>)> {
        let query = FlowQuery::parse(filter);
        let mut previous = None;
        let mut out = Vec::new();
        for id in &self.order {
            let Some(flow) = self.flows.get(id) else {
                continue;
            };
            if !query.matches(flow) {
                continue;
            }
            out.push((*id, previous));
            previous = Some(flow.started);
        }
        out
    }

    fn evict(&mut self, keep: u64) {
        while self.bytes > SESSION_CAP && self.order.len() > 1 {
            let Some(pos) = self
                .order
                .iter()
                .position(|id| *id != keep && self.flows.get(id).is_some_and(|flow| !flow.pinned))
            else {
                break;
            };
            let Some(old) = self.order.remove(pos) else {
                break;
            };
            if let Some(flow) = self.flows.remove(&old) {
                self.bytes = self.bytes.saturating_sub(flow.accounted);
                self.dropped += 1;
            }
        }
    }
}

fn headers_size(headers: &[(String, String)]) -> usize {
    headers.iter().map(|(k, v)| k.len() + v.len()).sum()
}

fn body_contains(flow: &Flow, keyword: &str) -> bool {
    let request = String::from_utf8_lossy(&flow.request_body).to_ascii_lowercase();
    let response = String::from_utf8_lossy(&flow.response_body).to_ascii_lowercase();
    request.contains(keyword) || response.contains(keyword)
}

fn header_contains(flow: &Flow, keyword: &str) -> bool {
    flow.request_headers
        .iter()
        .chain(flow.response_headers.iter())
        .any(|(name, value)| {
            name.to_ascii_lowercase().contains(keyword)
                || value.to_ascii_lowercase().contains(keyword)
        })
}

const METHODS: &[&str] = &[
    "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "CONNECT",
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum StatusMatch {
    Exact(u16),
    Class(u16),
    Failed,
}

impl StatusMatch {
    fn parse(token: &str) -> Option<Self> {
        let lower = token.trim().to_ascii_lowercase();
        if lower == "fail" || lower == "失败" || lower == "err" || lower == "error" {
            return Some(Self::Failed);
        }
        if let Some(class) = lower.strip_suffix("xx")
            && class.len() == 1
            && let Ok(digit) = class.parse::<u16>()
            && (1..6).contains(&digit)
        {
            return Some(Self::Class(digit));
        }
        if lower.len() == 3
            && lower.bytes().all(|byte| byte.is_ascii_digit())
            && let Ok(code) = lower.parse::<u16>()
        {
            return Some(Self::Exact(code));
        }
        None
    }

    fn matches(&self, flow: &Flow) -> bool {
        match self {
            Self::Failed => flow.error.is_some() && flow.status.is_none(),
            Self::Exact(code) => flow.status == Some(*code),
            Self::Class(digit) => flow.status.is_some_and(|code| code / 100 == *digit),
        }
    }
}

fn split_or_terms(text: &str) -> Vec<String> {
    text.split(['|', ','])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| part.to_ascii_lowercase())
        .collect()
}

/// Joins `|` tokens separated by spaces (`a | b` -> `a|b`) so whitespace
/// splitting keeps an OR group together.
pub fn normalize_filter_pipes(filter: &str) -> String {
    let mut tokens: Vec<String> = Vec::new();
    let mut join_next = false;
    for raw in filter.split_whitespace() {
        if raw == "|" {
            join_next = true;
            continue;
        }
        if let Some(rest) = raw.strip_prefix('|') {
            if let Some(last) = tokens.last_mut() {
                last.push('|');
                last.push_str(rest);
            } else if !rest.is_empty() {
                tokens.push(rest.to_string());
            }
            join_next = raw.ends_with('|');
            continue;
        }
        if join_next {
            if let Some(last) = tokens.last_mut() {
                if !last.ends_with('|') {
                    last.push('|');
                }
                last.push_str(raw);
            } else {
                tokens.push(raw.to_string());
            }
            join_next = raw.ends_with('|');
            continue;
        }
        join_next = raw.ends_with('|');
        tokens.push(raw.to_string());
    }
    tokens.join(" ")
}

fn host_matches(host: &str, pattern: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        host == suffix || host.ends_with(&format!(".{suffix}"))
    } else {
        host.contains(pattern)
    }
}

fn path_prefix_matches(path: &str, prefix: &str) -> bool {
    let prefix = prefix.trim();
    if prefix.is_empty() {
        return true;
    }
    if prefix.starts_with('/') {
        path.starts_with(prefix)
    } else {
        path.starts_with(prefix)
            || path
                .strip_prefix('/')
                .is_some_and(|rest| rest.starts_with(prefix))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlowQuery {
    texts: Vec<Vec<String>>,
    hosts: Vec<String>,
    path_prefixes: Vec<String>,
    excludes: Vec<String>,
    methods: Vec<String>,
    statuses: Vec<StatusMatch>,
    content_types: Vec<String>,
    bodies: Vec<Vec<String>>,
    headers: Vec<Vec<String>>,
    pinned_only: bool,
    slow_only: bool,
}

impl FlowQuery {
    fn parse(filter: &str) -> Self {
        let normalized = normalize_filter_pipes(filter);
        let mut query = Self {
            texts: Vec::new(),
            hosts: Vec::new(),
            path_prefixes: Vec::new(),
            excludes: Vec::new(),
            methods: Vec::new(),
            statuses: Vec::new(),
            content_types: Vec::new(),
            bodies: Vec::new(),
            headers: Vec::new(),
            pinned_only: false,
            slow_only: false,
        };
        for token in normalized.split_whitespace() {
            let lower = token.to_ascii_lowercase();
            if lower == "pin:" {
                query.pinned_only = true;
                continue;
            }
            if lower == "slow:" {
                query.slow_only = true;
                continue;
            }
            if let Some(keyword) = lower.strip_prefix("mute:") {
                query.excludes.extend(split_or_terms(keyword));
                continue;
            }
            if let Some(keyword) = lower.strip_prefix('-') {
                if !keyword.is_empty() && !keyword.contains(':') {
                    query.excludes.extend(split_or_terms(keyword));
                }
                continue;
            }
            if let Some(rest) = lower
                .strip_prefix("host:")
                .or_else(|| lower.strip_prefix("domain:"))
            {
                query.hosts.extend(split_or_terms(rest));
                continue;
            }
            if let Some(rest) = lower.strip_prefix("path:") {
                query.path_prefixes.extend(split_or_terms(rest));
                continue;
            }
            if let Some(keyword) = lower.strip_prefix("body:") {
                let group = split_or_terms(keyword);
                if !group.is_empty() {
                    query.bodies.push(group);
                }
                continue;
            }
            if let Some(keyword) = lower.strip_prefix("header:") {
                let group = split_or_terms(keyword);
                if !group.is_empty() {
                    query.headers.push(group);
                }
                continue;
            }
            if let Some(kind) = lower
                .strip_prefix("type:")
                .or_else(|| lower.strip_prefix("ct:"))
            {
                query.content_types.extend(split_or_terms(kind));
                continue;
            }
            if let Some(rest) = lower.strip_prefix("method:") {
                for item in split_or_terms(rest) {
                    let upper = item.to_ascii_uppercase();
                    if !query.methods.contains(&upper) {
                        query.methods.push(upper);
                    }
                }
                continue;
            }
            if let Some(rest) = lower.strip_prefix("status:") {
                for item in split_or_terms(rest) {
                    if let Some(status) = StatusMatch::parse(&item)
                        && !query.statuses.contains(&status)
                    {
                        query.statuses.push(status);
                    }
                }
                continue;
            }
            let parts: Vec<&str> = token
                .split(['|', ','])
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .collect();
            if !parts.is_empty()
                && parts.iter().all(|part| {
                    METHODS
                        .iter()
                        .any(|method| method.eq_ignore_ascii_case(part))
                })
            {
                for part in parts {
                    let upper = part.to_ascii_uppercase();
                    if !query.methods.contains(&upper) {
                        query.methods.push(upper);
                    }
                }
                continue;
            }
            if !parts.is_empty() && parts.iter().all(|part| StatusMatch::parse(part).is_some()) {
                for part in parts {
                    if let Some(status) = StatusMatch::parse(part)
                        && !query.statuses.contains(&status)
                    {
                        query.statuses.push(status);
                    }
                }
                continue;
            }
            let group = split_or_terms(&lower);
            if !group.is_empty() {
                query.texts.push(group);
            }
        }
        query
    }

    fn matches(&self, flow: &Flow) -> bool {
        if self.pinned_only && !flow.pinned {
            return false;
        }
        if self.slow_only && !is_slow(flow.started, flow.finished_at, SystemTime::now()) {
            return false;
        }
        if !self.methods.is_empty()
            && !self
                .methods
                .iter()
                .any(|method| flow.method.eq_ignore_ascii_case(method))
        {
            return false;
        }
        if !self.statuses.is_empty() && !self.statuses.iter().any(|status| status.matches(flow)) {
            return false;
        }
        if !self.content_types.is_empty() {
            let header = flow
                .response_headers
                .iter()
                .chain(flow.request_headers.iter())
                .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                .map(|(_, value)| value.to_ascii_lowercase());
            if !header
                .is_some_and(|value| self.content_types.iter().any(|kind| value.contains(kind)))
            {
                return false;
            }
        }
        if !self
            .bodies
            .iter()
            .all(|group| group.iter().any(|keyword| body_contains(flow, keyword)))
        {
            return false;
        }
        if !self
            .headers
            .iter()
            .all(|group| group.iter().any(|keyword| header_contains(flow, keyword)))
        {
            return false;
        }
        let host = flow.host.to_ascii_lowercase();
        let path = flow.path.to_ascii_lowercase();
        if !self.hosts.is_empty()
            && !self
                .hosts
                .iter()
                .any(|pattern| host_matches(&host, pattern))
        {
            return false;
        }
        if !self.path_prefixes.is_empty()
            && !self
                .path_prefixes
                .iter()
                .any(|prefix| path_prefix_matches(&path, prefix))
        {
            return false;
        }
        if self
            .excludes
            .iter()
            .any(|text| host_matches(&host, text) || path.contains(text))
        {
            return false;
        }
        self.texts.iter().all(|group| {
            group
                .iter()
                .any(|text| host_matches(&host, text) || path.contains(text))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(host: &str) -> FlowDraft {
        FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: host.into(),
            path: "/".into(),
            url: format!("http://{host}/"),
            request_headers: vec![("Host".into(), host.into())],
        }
    }

    #[test]
    fn body_cap_marks_truncation() {
        let mut session = Session::default();
        let id = session.open(draft("a.test"));
        let chunk = vec![1u8; BODY_CAP];
        assert!(!session.append_body(id, Direction::Response, &chunk));
        assert!(session.append_body(id, Direction::Response, &[2]));
        let flow = session.get(id).unwrap();
        assert_eq!(flow.response_body.len(), BODY_CAP);
        assert!(flow.response_truncated);
        assert_eq!(flow.response_body[0], 1);
    }

    #[test]
    fn session_cap_drops_oldest() {
        let mut session = Session::default();
        let first = session.open(draft("old.test"));
        let _ = session.append_body(first, Direction::Response, &vec![7u8; BODY_CAP]);
        let mut opened = 1u64;
        while session.get(first).is_some() && opened < 400 {
            let id = session.open(draft("new.test"));
            session.append_body(id, Direction::Response, &vec![8u8; BODY_CAP]);
            opened += 1;
        }
        assert!(session.get(first).is_none());
        assert!(session.dropped() >= 1);
        assert!(session.memory_bytes() <= SESSION_CAP);
    }

    #[test]
    fn gap_skips_the_hidden_neighbor() {
        let mut session = Session::default();
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut open = |host: &str, path: &str| {
            session.open(FlowDraft {
                protocol: Protocol::Http11,
                method: "GET".into(),
                host: host.into(),
                path: path.into(),
                url: format!("http://{host}{path}"),
                request_headers: vec![],
            })
        };
        let first = open("alpha.test", "/item");
        let hidden = open("beta.test", "/secret");
        let third = open("gamma.test", "/item");
        session.set_started(first, base);
        session.set_started(hidden, base + Duration::from_millis(45));
        session.set_started(third, base + Duration::from_millis(45 + 1_240));
        let rows = session.matching_with_previous_start("item");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, first);
        assert_eq!(rows[0].1, None);
        assert_eq!(rows[1].0, third);
        assert_eq!(rows[1].1, Some(base));
    }

    #[test]
    fn host_filter_is_a_substring() {
        let mut session = Session::default();
        let a = session.open(draft("api.example.com"));
        let _ = session.open(draft("other.test"));
        assert_eq!(session.ids_matching("EXAMPLE"), vec![a]);
        assert_eq!(session.ids_matching("").len(), 2);
    }

    #[test]
    fn clear_drops_flows_and_the_dropped_count() {
        let mut session = Session::default();
        let id = session.open(draft("a.test"));
        session.append_body(id, Direction::Response, b"hi");
        session.clear();
        assert!(session.get(id).is_none());
        assert_eq!(session.memory_bytes(), 0);
        assert_eq!(session.dropped(), 0);
        assert!(session.ids_matching("").is_empty());
    }

    #[test]
    fn timing_and_transfer_bytes() {
        let mut session = Session::default();
        let id = session.open(draft("a.test"));
        assert!(session.ttfb(id).is_none());
        session.add_transfer(id, Direction::Request, 12);
        session.set_response(id, 200, Vec::new(), None);
        session.add_transfer(id, Direction::Response, 40);
        session.mark_finished(id);
        assert!(session.ttfb(id).is_some());
        assert!(session.duration(id).is_some());
        let flow = session.get(id).unwrap();
        assert_eq!(flow.request_bytes, 12);
        assert_eq!(flow.response_bytes, 40);
        session.mark_finished(id);
        let again = session.duration(id);
        assert_eq!(again, session.duration(id));
    }

    #[test]
    fn filter_matches_method_status_path_and_type() {
        let mut session = Session::default();
        let mut get = draft("api.example.com");
        get.path = "/v1/users?q=1".into();
        let get_id = session.open(get);
        session.set_response(
            get_id,
            200,
            vec![("Content-Type".into(), "application/json".into())],
            None,
        );
        let mut post = draft("other.test");
        post.method = "POST".into();
        post.path = "/v1/login".into();
        let post_id = session.open(post);
        session.set_response(post_id, 500, Vec::new(), None);
        let failed = session.open(draft("pin.test"));
        session.set_error(failed, "证书被拒绝");

        assert_eq!(session.ids_matching("GET"), vec![get_id, failed]);
        assert_eq!(session.ids_matching("POST /login"), vec![post_id]);
        assert_eq!(session.ids_matching("5xx"), vec![post_id]);
        assert_eq!(session.ids_matching("type:json"), vec![get_id]);
        assert_eq!(session.ids_matching("失败"), vec![failed]);
        assert_eq!(session.ids_matching("/v1"), vec![get_id, post_id]);
        assert_eq!(
            session
                .flows_matching("POST")
                .iter()
                .map(|flow| flow.id)
                .collect::<Vec<_>>(),
            vec![post_id]
        );
        assert_eq!(session.flows_matching("").len(), 3);
    }

    #[test]
    fn filter_matches_body_and_header_keywords() {
        let mut session = Session::default();
        let mut first = draft("api.example.com");
        first
            .request_headers
            .push(("X-Trace".into(), "alpha-1".into()));
        let id = session.open(first);
        session.append_body(id, Direction::Request, b"{\"code\":1001}");
        session.set_response(
            id,
            200,
            vec![("Content-Type".into(), "application/json".into())],
            None,
        );
        session.append_body(id, Direction::Response, b"{\"msg\":\"nope\"}");
        let other = session.open(draft("other.test"));
        session.append_body(other, Direction::Response, b"hello");

        assert_eq!(session.ids_matching("body:1001"), vec![id]);
        assert_eq!(session.ids_matching("BODY:Nope"), vec![id]);
        assert_eq!(session.ids_matching("header:x-trace"), vec![id]);
        assert_eq!(session.ids_matching("header:alpha"), vec![id]);
        assert_eq!(session.ids_matching("header:json"), vec![id]);
        assert!(session.ids_matching("body:1001 header:missing").is_empty());
        assert_eq!(session.ids_matching("body:hello"), vec![other]);
        assert_eq!(session.ids_matching("body:1001 example"), vec![id]);
    }

    #[test]
    fn mute_and_pin_filter_and_pinned_flows_skip_eviction() {
        let mut session = Session::default();
        let noisy = session.open(draft("ads.example"));
        let kept = session.open(draft("api.example"));
        assert!(session.set_pinned(kept, true));
        assert_eq!(session.ids_matching("-ads.example"), vec![kept]);
        assert_eq!(session.ids_matching("mute:ads.example"), vec![kept]);
        assert_eq!(session.ids_matching("pin:"), vec![kept]);
        session.append_body(noisy, Direction::Response, &vec![1u8; BODY_CAP]);
        session.append_body(kept, Direction::Response, &vec![2u8; BODY_CAP]);
        let mut opened = 2u64;
        while session.get(noisy).is_some() && opened < 400 {
            let id = session.open(draft("new.test"));
            session.append_body(id, Direction::Response, &vec![3u8; BODY_CAP]);
            opened += 1;
        }
        assert!(session.get(noisy).is_none());
        assert!(session.get(kept).is_some());
        assert!(session.get(kept).unwrap().pinned);
    }

    #[test]
    fn slow_filter_matches_the_one_second_warning() {
        let mut session = Session::default();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let slow = session.open(draft("slow.test"));
        session.set_started(slow, now - Duration::from_secs(2));
        session.restore_imported(slow, None, Some(now), None, None);
        let fast = session.open(draft("fast.test"));
        session.set_started(fast, now - Duration::from_millis(200));
        session.restore_imported(fast, None, Some(now), None, None);
        let _ = fast;
        let edge = session.open(draft("edge.test"));
        session.set_started(edge, now);
        session.restore_imported(edge, None, Some(now + SLOW_AFTER), None, None);
        let young = session.open(draft("young.test"));
        session.set_started(young, SystemTime::now() - Duration::from_millis(200));
        let _ = young;
        let pending = session.open(draft("pending.test"));
        session.set_started(pending, SystemTime::now() - Duration::from_secs(2));

        assert_eq!(session.ids_matching("slow:"), vec![slow, edge, pending]);
        assert_eq!(session.ids_matching("SLOW: slow.test"), vec![slow]);
        assert!(session.ids_matching("slow: fast.test").is_empty());
        assert!(!is_slow(
            now,
            Some(now + Duration::from_millis(999)),
            now + Duration::from_secs(5),
        ));
    }

    #[test]
    fn filter_supports_domain_or_and_path_prefix_or() {
        let mut session = Session::default();
        let mut a = draft("api.example.com");
        a.path = "/v1/users/list".into();
        let id_a = session.open(a);
        session.set_response(id_a, 200, Vec::new(), None);

        let mut b = draft("cdn.other.org");
        b.method = "POST".into();
        b.path = "/auth/token".into();
        let id_b = session.open(b);
        session.set_response(id_b, 401, Vec::new(), None);

        let mut c = draft("sub.third.net");
        c.method = "DELETE".into();
        c.path = "/legacy/v1/users".into();
        let id_c = session.open(c);
        session.set_response(id_c, 502, Vec::new(), None);

        assert_eq!(
            session.ids_matching("host:example.com|other.org"),
            vec![id_a, id_b]
        );
        assert_eq!(
            session.ids_matching("host:*.example.com | *.third.net"),
            vec![id_a, id_c]
        );
        assert_eq!(session.ids_matching("path:/v1|/auth"), vec![id_a, id_b]);
        assert_eq!(session.ids_matching("path:/v1 | /auth"), vec![id_a, id_b]);
        assert_eq!(session.ids_matching("GET|POST"), vec![id_a, id_b]);
        assert_eq!(session.ids_matching("2xx|5xx"), vec![id_a, id_c]);
        assert_eq!(session.ids_matching("mute:other.org|third.net"), vec![id_a]);
    }
}
