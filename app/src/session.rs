//! In-memory capture session.
//!
//! Bodies are already decompressed by the proxy. The 1 MB cap is per
//! direction. The 256 MB cap drops the oldest whole flow.

use std::collections::{HashMap, VecDeque};

pub const BODY_CAP: usize = 1024 * 1024;
pub const SESSION_CAP: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Http11,
    Http2,
    WebSocket,
    Tunnel,
}

impl Protocol {
    pub fn label(self) -> &'static str {
        match self {
            Self::Http11 => "HTTP/1.1",
            Self::Http2 => "HTTP/2",
            Self::WebSocket => "WebSocket",
            Self::Tunnel => "隧道",
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
    accounted: usize,
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
        }
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

    pub fn memory_bytes(&self) -> usize {
        self.bytes
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn ids_matching(&self, host_filter: &str) -> Vec<u64> {
        let needle = host_filter.trim().to_ascii_lowercase();
        self.order
            .iter()
            .copied()
            .filter(|id| {
                if needle.is_empty() {
                    return true;
                }
                self.flows
                    .get(id)
                    .map(|flow| flow.host.to_ascii_lowercase().contains(&needle))
                    .unwrap_or(false)
            })
            .collect()
    }

    fn evict(&mut self, keep: u64) {
        while self.bytes > SESSION_CAP && self.order.len() > 1 {
            let Some(old) = self.order.front().copied() else {
                break;
            };
            if old == keep {
                break;
            }
            self.order.pop_front();
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
    fn host_filter_is_a_substring() {
        let mut session = Session::default();
        let a = session.open(draft("api.example.com"));
        let _ = session.open(draft("other.test"));
        assert_eq!(session.ids_matching("EXAMPLE"), vec![a]);
        assert_eq!(session.ids_matching("").len(), 2);
    }
}
