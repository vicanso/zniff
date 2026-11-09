//! In-memory capture session with a ring-segment temporary spill store for large bodies.
//!
//! Bodies are already decompressed by the proxy. The 1 MB cap is per
//! direction. Completed unpinned bodies larger than [`INLINE_BODY_CAP`] (16 KB)
//! spill to rotating temporary segment files so the 256 MB in-memory cap keeps
//! flow metadata and small payloads resident.

use std::collections::{HashMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

pub const BODY_CAP: usize = 1024 * 1024;
pub const SESSION_CAP: usize = 256 * 1024 * 1024;
/// Bodies up to 16 KB stay inline in RAM; larger finished bodies spill to temporary segment files.
pub const INLINE_BODY_CAP: usize = 16 * 1024;
/// Each temporary spill segment file grows up to 64 MB before rolling to a new file.
pub const SPILL_SEGMENT_CAP: u64 = 64 * 1024 * 1024;
/// At most 8 segment files (512 MB total) are kept on disk; older segments are unlinked.
pub const SPILL_MAX_SEGMENTS: usize = 8;
/// Idle spill segment files older than 30 minutes are automatically unlinked.
pub const SPILL_SEGMENT_TTL: Duration = Duration::from_secs(30 * 60);
/// Pinned flows skip eviction. The cap keeps a pinned set from growing without a bound.
pub const PIN_CAP: usize = 32;
/// Same line as the warning color on a row: elapsed time from start to finish.
pub const SLOW_AFTER: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpillRef {
    pub segment_id: u32,
    pub offset: u64,
    pub len: u32,
}

#[derive(Debug)]
struct SpillSegment {
    id: u32,
    path: PathBuf,
    file: File,
    len: u64,
    last_write: SystemTime,
}

#[derive(Debug)]
struct SpillStore {
    dir: PathBuf,
    dir_created: bool,
    segments: VecDeque<SpillSegment>,
    next_segment_id: u32,
    inline_cap: usize,
    segment_cap: u64,
    max_segments: usize,
    segment_ttl: Duration,
}

impl Default for SpillStore {
    fn default() -> Self {
        Self::with_limits(
            INLINE_BODY_CAP,
            SPILL_SEGMENT_CAP,
            SPILL_MAX_SEGMENTS,
            SPILL_SEGMENT_TTL,
        )
    }
}

fn spill_base_dir() -> PathBuf {
    std::env::temp_dir().join("zniff-spill")
}

/// Cleans up leftover spill directories from previous terminated processes.
fn sweep_orphan_spill_dirs(base: &Path, now: SystemTime, ttl: Duration) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    let my_prefix = format!("{}-", std::process::id());
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if name_str.starts_with(&my_prefix) {
            continue;
        }
        let path = entry.path();
        // Empty orphan directories can always be removed immediately.
        if std::fs::remove_dir(&path).is_ok() {
            continue;
        }
        let expired = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= ttl);
        if expired {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

impl SpillStore {
    fn with_limits(
        inline_cap: usize,
        segment_cap: u64,
        max_segments: usize,
        segment_ttl: Duration,
    ) -> Self {
        static NEXT_STORE: AtomicUsize = AtomicUsize::new(1);
        let seq = NEXT_STORE.fetch_add(1, Ordering::Relaxed);
        let dir = spill_base_dir().join(format!("{}-{seq}", std::process::id()));
        Self {
            dir,
            dir_created: false,
            segments: VecDeque::new(),
            next_segment_id: 0,
            inline_cap,
            segment_cap: segment_cap.max(1),
            max_segments: max_segments.max(1),
            segment_ttl,
        }
    }

    fn write(&mut self, data: &[u8], now: SystemTime) -> Option<SpillRef> {
        if data.is_empty() {
            return None;
        }
        let len = u32::try_from(data.len()).ok()?;
        self.gc(now);
        let need_roll = self.segments.back().is_none_or(|seg| {
            seg.len > 0 && seg.len.saturating_add(data.len() as u64) > self.segment_cap
        });
        if need_roll {
            static SWEEP_ONCE: Once = Once::new();
            SWEEP_ONCE.call_once(|| {
                sweep_orphan_spill_dirs(&spill_base_dir(), now, SPILL_SEGMENT_TTL);
            });
            if !self.dir_created {
                std::fs::create_dir_all(&self.dir).ok()?;
                self.dir_created = true;
            }
            self.next_segment_id = self.next_segment_id.wrapping_add(1).max(1);
            let id = self.next_segment_id;
            let path = self.dir.join(format!("seg-{id:06}.dat"));
            let file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .read(true)
                .write(true)
                .open(&path)
                .ok()?;
            // On Unix, unlink the segment file immediately while keeping its File
            // descriptor open. Positional pread/pwrite continue to work, and the OS
            // kernel automatically reclaims the disk blocks even on SIGKILL (`kill -9`).
            #[cfg(unix)]
            {
                let _ = std::fs::remove_file(&path);
                let _ = std::fs::remove_dir(&self.dir);
                self.dir_created = false;
            }
            self.segments.push_back(SpillSegment {
                id,
                path,
                file,
                len: 0,
                last_write: now,
            });
            while self.segments.len() > self.max_segments {
                if let Some(old) = self.segments.pop_front() {
                    let _ = std::fs::remove_file(&old.path);
                }
            }
        }
        let seg = self.segments.back_mut()?;
        let offset = seg.len;
        write_at(&seg.file, data, offset).ok()?;
        seg.len += data.len() as u64;
        seg.last_write = now;
        Some(SpillRef {
            segment_id: seg.id,
            offset,
            len,
        })
    }

    fn read(&self, sref: SpillRef) -> Option<Vec<u8>> {
        let seg = self.segments.iter().find(|seg| seg.id == sref.segment_id)?;
        let mut buf = vec![0u8; sref.len as usize];
        read_at(&seg.file, &mut buf, sref.offset).ok()?;
        Some(buf)
    }

    fn gc(&mut self, now: SystemTime) -> bool {
        let before = self.segments.len();
        self.segments.retain(|seg| {
            let expired = now
                .duration_since(seg.last_write)
                .is_ok_and(|age| age >= self.segment_ttl);
            if expired {
                let _ = std::fs::remove_file(&seg.path);
                false
            } else {
                true
            }
        });
        self.segments.len() != before
    }

    fn clear(&mut self) {
        for seg in self.segments.drain(..) {
            let _ = std::fs::remove_file(&seg.path);
        }
        if self.dir_created {
            let _ = std::fs::remove_dir_all(&self.dir);
            self.dir_created = false;
        }
    }
}

impl Drop for SpillStore {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buf, offset)
}

#[cfg(windows)]
fn read_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let n = file.seek_read(buf, offset)?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "short spill read",
            ));
        }
        buf = &mut buf[n..];
        offset += n as u64;
    }
    Ok(())
}

#[cfg(unix)]
fn write_at(file: &File, data: &[u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(data, offset)
}

#[cfg(windows)]
fn write_at(file: &File, mut data: &[u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !data.is_empty() {
        let n = file.seek_write(data, offset)?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "short spill write",
            ));
        }
        data = &data[n..];
        offset += n as u64;
    }
    Ok(())
}

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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConnInfo {
    pub dns_ms: Option<u64>,
    pub tcp_ms: Option<u64>,
    pub tls_ms: Option<u64>,
    pub reused: bool,
    pub server_addr: String,
    pub tls_version: String,
    /// True when DNS/TCP/TLS happened before `flow.started` (e.g. initial request on `mitm_connect` or HAR import).
    pub dialed_before_start: bool,
}

impl ConnInfo {
    pub fn as_reused(&self) -> Self {
        Self {
            dns_ms: None,
            tcp_ms: None,
            tls_ms: None,
            reused: true,
            server_addr: self.server_addr.clone(),
            tls_version: self.tls_version.clone(),
            dialed_before_start: false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.dns_ms.is_none()
            && self.tcp_ms.is_none()
            && self.tls_ms.is_none()
            && !self.reused
            && self.server_addr.is_empty()
            && self.tls_version.is_empty()
    }
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
    pub client_ip: String,
    pub conn_info: ConnInfo,
    pub status: Option<u16>,
    pub error: Option<String>,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
    pub request_body: Vec<u8>,
    pub response_body: Vec<u8>,
    pub request_spill: Option<SpillRef>,
    pub response_spill: Option<SpillRef>,
    pub request_expired: Option<usize>,
    pub response_expired: Option<usize>,
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
    /// User-supplied note/comment attached to this flow.
    pub comment: String,
    /// Monotonic revision bumped whenever this flow is created or modified.
    pub rev: u64,
    accounted: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FilteredStats {
    pub total_count: usize,
    pub s2xx: usize,
    pub s3xx: usize,
    pub s4xx: usize,
    pub s5xx: usize,
    pub failed: usize,
    pub pending: usize,
    pub total_req_bytes: usize,
    pub total_res_bytes: usize,
    pub total_bytes: usize,
    pub avg_elapsed_ms: Option<u64>,
    pub max_elapsed: Option<(u64, u64, String)>,
}

impl FilteredStats {
    pub fn from_flows<'a>(flows: impl IntoIterator<Item = &'a Flow>) -> Self {
        let mut stats = Self::default();
        let mut elapsed_sum_ms: u128 = 0;
        let mut elapsed_count: u64 = 0;
        for flow in flows {
            stats.total_count += 1;
            match flow.status {
                Some(200..=299) => stats.s2xx += 1,
                Some(300..=399) => stats.s3xx += 1,
                Some(400..=499) => stats.s4xx += 1,
                Some(500..=599) => stats.s5xx += 1,
                Some(_) => {
                    if flow.error.is_some() {
                        stats.failed += 1;
                    } else if flow.finished_at.is_none() {
                        stats.pending += 1;
                    }
                }
                None => {
                    if flow.error.is_some() {
                        stats.failed += 1;
                    } else {
                        stats.pending += 1;
                    }
                }
            }
            let req = flow.req_bytes();
            let res = flow.res_bytes();
            stats.total_req_bytes = stats.total_req_bytes.saturating_add(req);
            stats.total_res_bytes = stats.total_res_bytes.saturating_add(res);
            stats.total_bytes = stats.total_bytes.saturating_add(flow.transfer_bytes());
            if let Some(ms) = flow.elapsed_ms() {
                elapsed_sum_ms += u128::from(ms);
                elapsed_count += 1;
                let replace = stats
                    .max_elapsed
                    .as_ref()
                    .is_none_or(|(_, max_ms, _)| ms > *max_ms);
                if replace {
                    stats.max_elapsed = Some((flow.id, ms, flow.display_target()));
                }
            }
        }
        if elapsed_count > 0 {
            stats.avg_elapsed_ms = Some((elapsed_sum_ms / u128::from(elapsed_count)) as u64);
        }
        stats
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FacetEntry {
    pub label: String,
    pub token: String,
    pub count: usize,
    pub error_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionFacets {
    pub domains: Vec<FacetEntry>,
    pub apps: Vec<FacetEntry>,
}

pub fn extract_client_app_facet(client_ip: &str) -> Option<(String, String)> {
    let raw = client_ip.trim();
    if raw.is_empty() || raw == "—" {
        return None;
    }
    let label = if let Some(rest) = raw.strip_suffix(')')
        && let Some(idx) = rest.rfind('(')
    {
        let proc_name = rest[idx + 1..].trim();
        if proc_name.is_empty() { raw } else { proc_name }
    } else {
        raw
    };
    let token = label.split_whitespace().collect::<Vec<_>>().join("_");
    if token.is_empty() {
        None
    } else {
        Some((label.to_string(), token))
    }
}

impl Flow {
    pub fn req_bytes(&self) -> usize {
        if self.request_bytes > 0 {
            self.request_bytes as usize
        } else if let Some(sref) = self.request_spill {
            sref.len as usize
        } else if let Some(exp) = self.request_expired {
            exp
        } else {
            self.request_body.len()
        }
    }

    pub fn res_bytes(&self) -> usize {
        if self.response_bytes > 0 {
            self.response_bytes as usize
        } else if let Some(sref) = self.response_spill {
            sref.len as usize
        } else if let Some(exp) = self.response_expired {
            exp
        } else {
            self.response_body.len()
        }
    }

    pub fn transfer_bytes(&self) -> usize {
        self.req_bytes().saturating_add(self.res_bytes())
    }

    pub fn elapsed_ms(&self) -> Option<u64> {
        self.finished_at?
            .duration_since(self.started)
            .ok()
            .map(|d| d.as_millis() as u64)
    }

    pub fn display_target(&self) -> String {
        if !self.path.is_empty() && self.path != "/" {
            if self.host.is_empty() {
                self.path.clone()
            } else if self.path.starts_with('/') || self.path.starts_with(':') {
                format!("{}{}", self.host, self.path)
            } else {
                format!("{}/{}", self.host, self.path)
            }
        } else if !self.url.is_empty() {
            self.url.clone()
        } else if !self.host.is_empty() {
            self.host.clone()
        } else {
            self.path.clone()
        }
    }
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
    spill: SpillStore,
    bytes: usize,
    dropped: u64,
    next_id: u64,
    next_rev: u64,
}

impl Session {
    #[cfg(test)]
    pub fn with_spill_limits(
        inline_cap: usize,
        segment_cap: u64,
        max_segments: usize,
        segment_ttl: Duration,
    ) -> Self {
        Self {
            spill: SpillStore::with_limits(inline_cap, segment_cap, max_segments, segment_ttl),
            ..Self::default()
        }
    }

    pub fn rev(&self) -> u64 {
        self.next_rev
    }

    pub fn open(&mut self, draft: FlowDraft) -> u64 {
        let now = SystemTime::now();
        self.spill.gc(now);
        self.next_id += 1;
        self.next_rev += 1;
        let id = self.next_id;
        let rev = self.next_rev;
        let headers_size = headers_size(&draft.request_headers);
        let flow = Flow {
            id,
            protocol: draft.protocol,
            method: draft.method,
            host: draft.host,
            path: draft.path,
            url: draft.url,
            client_ip: String::new(),
            conn_info: ConnInfo::default(),
            status: None,
            error: None,
            request_headers: draft.request_headers,
            response_headers: Vec::new(),
            request_body: Vec::new(),
            response_body: Vec::new(),
            request_spill: None,
            response_spill: None,
            request_expired: None,
            response_expired: None,
            request_truncated: false,
            response_truncated: false,
            request_encoding_error: None,
            response_encoding_error: None,
            started: now,
            response_at: None,
            finished_at: None,
            request_bytes: 0,
            response_bytes: 0,
            messages: Vec::new(),
            messages_truncated: false,
            pinned: false,
            rule_hit: String::new(),
            comment: String::new(),
            rev,
            accounted: headers_size,
        };
        self.bytes += headers_size;
        self.order.push_back(id);
        self.flows.insert(id, flow);
        self.evict(id);
        id
    }

    pub fn set_client_ip(&mut self, id: u64, client_ip: impl Into<String>) {
        self.next_rev += 1;
        let rev = self.next_rev;
        if let Some(flow) = self.flows.get_mut(&id) {
            flow.client_ip = client_ip.into();
            flow.rev = rev;
        }
    }

    pub fn set_conn_info(&mut self, id: u64, conn_info: ConnInfo) {
        self.next_rev += 1;
        let rev = self.next_rev;
        if let Some(flow) = self.flows.get_mut(&id) {
            flow.conn_info = conn_info;
            flow.rev = rev;
        }
    }

    pub fn set_response(
        &mut self,
        id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        protocol: Option<Protocol>,
    ) {
        let extra = headers_size(&headers);
        self.next_rev += 1;
        let rev = self.next_rev;
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
        flow.rev = rev;
        flow.accounted += extra;
        self.bytes += extra;
        self.evict(id);
    }

    pub fn set_error(&mut self, id: u64, error: impl Into<String>) {
        self.next_rev += 1;
        let rev = self.next_rev;
        if let Some(flow) = self.flows.get_mut(&id) {
            flow.error = Some(error.into());
            if flow.finished_at.is_none() {
                flow.finished_at = Some(SystemTime::now());
            }
            flow.rev = rev;
        }
    }

    pub fn add_transfer(&mut self, id: u64, direction: Direction, bytes: u64) {
        self.next_rev += 1;
        let rev = self.next_rev;
        let Some(flow) = self.flows.get_mut(&id) else {
            return;
        };
        match direction {
            Direction::Request => flow.request_bytes = flow.request_bytes.saturating_add(bytes),
            Direction::Response => flow.response_bytes = flow.response_bytes.saturating_add(bytes),
        }
        flow.rev = rev;
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
        self.next_rev += 1;
        let rev = self.next_rev;
        let Some(flow) = self.flows.get_mut(&id) else {
            return;
        };
        flow.rev = rev;
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
        self.next_rev += 1;
        let rev = self.next_rev;
        if let Some(flow) = self.flows.get_mut(&id) {
            flow.host = host;
            flow.path = path;
            flow.url = url;
            flow.rev = rev;
        }
    }

    pub fn replace_request(&mut self, id: u64, headers: Vec<(String, String)>, body: Vec<u8>) {
        self.next_rev += 1;
        let rev = self.next_rev;
        let Some(flow) = self.flows.get_mut(&id) else {
            return;
        };
        let old = headers_size(&flow.request_headers) + flow.request_body.len();
        let new_size = headers_size(&headers) + body.len();
        flow.request_headers = headers;
        flow.request_body = body;
        flow.request_spill = None;
        flow.request_expired = None;
        flow.rev = rev;
        flow.accounted = flow.accounted.saturating_sub(old) + new_size;
        self.bytes = self.bytes.saturating_sub(old) + new_size;
        self.spill_flow_if_needed(id, SystemTime::now());
    }

    pub fn replace_response(
        &mut self,
        id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) {
        self.next_rev += 1;
        let rev = self.next_rev;
        let Some(flow) = self.flows.get_mut(&id) else {
            return;
        };
        let old = headers_size(&flow.response_headers) + flow.response_body.len();
        let new_size = headers_size(&headers) + body.len();
        flow.status = Some(status);
        flow.response_headers = headers;
        flow.response_body = body;
        flow.response_spill = None;
        flow.response_expired = None;
        flow.response_truncated = false;
        if flow.response_at.is_none() {
            flow.response_at = Some(SystemTime::now());
        }
        flow.rev = rev;
        flow.accounted = flow.accounted.saturating_sub(old) + new_size;
        self.bytes = self.bytes.saturating_sub(old) + new_size;
        self.spill_flow_if_needed(id, SystemTime::now());
        self.evict(id);
    }

    /// Pins until [`PIN_CAP`]. Returns false when the flow is missing or the cap is full.
    /// Pinned flows promote any spilled bodies back to inline memory so segment rotation
    /// cannot expire them.
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
            self.next_rev += 1;
            let rev = self.next_rev;
            let Some(flow) = self.flows.get_mut(&id) else {
                return false;
            };
            flow.pinned = true;
            flow.rev = rev;
            let req_spill = flow.request_spill.take();
            let res_spill = flow.response_spill.take();
            let mut added = 0usize;
            if let Some(sref) = req_spill {
                match self.spill.read(sref) {
                    Some(bytes) => {
                        added += bytes.len();
                        if let Some(flow) = self.flows.get_mut(&id) {
                            flow.request_body = bytes;
                        }
                    }
                    None => {
                        if let Some(flow) = self.flows.get_mut(&id) {
                            flow.request_expired = Some(sref.len as usize);
                        }
                    }
                }
            }
            if let Some(sref) = res_spill {
                match self.spill.read(sref) {
                    Some(bytes) => {
                        added += bytes.len();
                        if let Some(flow) = self.flows.get_mut(&id) {
                            flow.response_body = bytes;
                        }
                    }
                    None => {
                        if let Some(flow) = self.flows.get_mut(&id) {
                            flow.response_expired = Some(sref.len as usize);
                        }
                    }
                }
            }
            if added > 0
                && let Some(flow) = self.flows.get_mut(&id)
            {
                flow.accounted += added;
                self.bytes += added;
            }
            true
        } else if let Some(flow) = self.flows.get_mut(&id) {
            self.next_rev += 1;
            flow.pinned = false;
            flow.rev = self.next_rev;
            self.spill_flow_if_needed(id, SystemTime::now());
            true
        } else {
            false
        }
    }

    pub fn mark_finished(&mut self, id: u64) {
        self.mark_finished_at(id, SystemTime::now());
    }

    pub fn mark_finished_at(&mut self, id: u64, now: SystemTime) {
        self.spill.gc(now);
        self.next_rev += 1;
        let rev = self.next_rev;
        if let Some(flow) = self.flows.get_mut(&id) {
            if flow.finished_at.is_none() {
                flow.finished_at = Some(now);
            }
            flow.rev = rev;
        }
        self.spill_flow_if_needed(id, now);
    }

    /// Removes expired spill segments whose last write is older than [`SPILL_SEGMENT_TTL`].
    /// Returns `true` if any segment file was unlinked.
    pub fn gc_spill(&mut self, now: SystemTime) -> bool {
        self.spill.gc(now)
    }

    fn spill_flow_if_needed(&mut self, id: u64, now: SystemTime) {
        let inline_cap = self.spill.inline_cap;
        let (req_to_spill, res_to_spill) = {
            let Some(flow) = self.flows.get(&id) else {
                return;
            };
            if flow.pinned || flow.finished_at.is_none() {
                return;
            }
            (
                flow.request_spill.is_none() && flow.request_body.len() > inline_cap,
                flow.response_spill.is_none() && flow.response_body.len() > inline_cap,
            )
        };
        if req_to_spill {
            let body_opt = self.flows.get(&id).map(|flow| flow.request_body.clone());
            if let Some(body) = body_opt
                && let Some(sref) = self.spill.write(&body, now)
                && let Some(flow) = self.flows.get_mut(&id)
            {
                let freed = flow.request_body.len();
                flow.request_body = Vec::new();
                flow.request_spill = Some(sref);
                flow.accounted = flow.accounted.saturating_sub(freed);
                self.bytes = self.bytes.saturating_sub(freed);
            }
        }
        if res_to_spill {
            let body_opt = self.flows.get(&id).map(|flow| flow.response_body.clone());
            if let Some(body) = body_opt
                && let Some(sref) = self.spill.write(&body, now)
                && let Some(flow) = self.flows.get_mut(&id)
            {
                let freed = flow.response_body.len();
                flow.response_body = Vec::new();
                flow.response_spill = Some(sref);
                flow.accounted = flow.accounted.saturating_sub(freed);
                self.bytes = self.bytes.saturating_sub(freed);
            }
        }
    }

    pub fn set_started(&mut self, id: u64, started: SystemTime) {
        self.next_rev += 1;
        let rev = self.next_rev;
        if let Some(flow) = self.flows.get_mut(&id) {
            flow.started = started;
            flow.rev = rev;
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
        self.next_rev += 1;
        let rev = self.next_rev;
        let Some(flow) = self.flows.get_mut(&id) else {
            return;
        };
        if let Some(at) = response_at {
            flow.response_at = Some(at);
        }
        if let Some(at) = finished_at {
            flow.finished_at = Some(at);
        }
        let req_len = flow
            .request_spill
            .map(|sref| sref.len as u64)
            .unwrap_or(flow.request_body.len() as u64);
        let res_len = flow
            .response_spill
            .map(|sref| sref.len as u64)
            .unwrap_or(flow.response_body.len() as u64);
        flow.request_bytes = request_bytes.unwrap_or(req_len);
        flow.response_bytes = response_bytes.unwrap_or(res_len);
        flow.rev = rev;
    }

    #[cfg(test)]
    pub fn flows(&self) -> Vec<Flow> {
        self.flows_matching("")
    }

    /// Flows that match `filter`, oldest first, with spilled bodies hydrated from disk.
    /// An empty filter is the whole session.
    pub fn flows_matching(&self, filter: &str) -> Vec<Flow> {
        self.matching_with_previous_start(filter)
            .into_iter()
            .filter_map(|(id, _)| self.hydrated(id))
            .collect()
    }

    /// Flows that match `filter`, oldest first, without reading spilled bodies from disk.
    /// Spilled bodies remain referenced by `request_spill` / `response_spill` so live
    /// share list polling can load them on demand when a row is clicked.
    pub fn flows_matching_unhydrated(&self, filter: &str) -> Vec<Flow> {
        self.matching_with_previous_start(filter)
            .into_iter()
            .filter_map(|(id, _)| self.flows.get(&id).cloned())
            .collect()
    }

    /// Hydrates a single flow only if it matches `filter`.
    pub fn hydrated_matching(&self, id: u64, filter: &str) -> Option<Flow> {
        let flow = self.flows.get(&id)?;
        let query = FlowQuery::parse(filter);
        if !query.matches(flow, &self.spill) {
            return None;
        }
        self.hydrated(id)
    }

    pub fn clear(&mut self) {
        self.next_rev += 1;
        self.order.clear();
        self.flows.clear();
        self.spill.clear();
        self.bytes = 0;
        self.dropped = 0;
    }

    /// Removes all unpinned flows while keeping pinned flows intact.
    /// Returns the number of flows removed.
    pub fn clear_unpinned(&mut self) -> usize {
        let before = self.flows.len();
        self.flows.retain(|_, flow| flow.pinned);
        let removed = before.saturating_sub(self.flows.len());
        self.order.retain(|id| self.flows.contains_key(id));
        if self.flows.is_empty() {
            self.spill.clear();
        }
        self.bytes = self.flows.values().map(|flow| flow.accounted).sum();
        self.dropped = 0;
        self.next_rev += 1;
        removed
    }

    pub fn set_comment(&mut self, id: u64, comment: impl Into<String>) -> bool {
        let Some(flow) = self.flows.get_mut(&id) else {
            return false;
        };
        self.next_rev += 1;
        flow.comment = comment.into().trim().to_string();
        flow.rev = self.next_rev;
        true
    }

    pub fn set_rule_hit(&mut self, id: u64, hit: impl Into<String>) {
        self.next_rev += 1;
        let rev = self.next_rev;
        if let Some(flow) = self.flows.get_mut(&id) {
            flow.rule_hit = hit.into();
            flow.rev = rev;
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
        self.next_rev += 1;
        let rev = self.next_rev;
        if let Some(flow) = self.flows.get_mut(&id) {
            flow.protocol = protocol;
            flow.rev = rev;
        }
    }

    pub fn set_encoding_error(&mut self, id: u64, direction: Direction, error: impl Into<String>) {
        self.next_rev += 1;
        let rev = self.next_rev;
        let Some(flow) = self.flows.get_mut(&id) else {
            return;
        };
        flow.rev = rev;
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
        self.next_rev += 1;
        let rev = self.next_rev;
        let Some(flow) = self.flows.get_mut(&id) else {
            return false;
        };
        flow.rev = rev;
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

    /// Clones flow `id` and loads any spilled request/response bodies from the
    /// temporary segment store. If a segment was rotated or expired, sets
    /// `request_expired` / `response_expired` to the original byte length.
    pub fn hydrated(&self, id: u64) -> Option<Flow> {
        let mut flow = self.flows.get(&id)?.clone();
        if let Some(sref) = flow.request_spill {
            match self.spill.read(sref) {
                Some(bytes) => flow.request_body = bytes,
                None => flow.request_expired = Some(sref.len as usize),
            }
        }
        if let Some(sref) = flow.response_spill {
            match self.spill.read(sref) {
                Some(bytes) => flow.response_body = bytes,
                None => flow.response_expired = Some(sref.len as usize),
            }
        }
        Some(flow)
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
            if !query.matches(flow, &self.spill) {
                continue;
            }
            out.push((*id, previous));
            previous = Some(flow.started);
        }
        out
    }

    #[cfg(test)]
    pub fn facets(&self) -> SessionFacets {
        self.facets_matching("")
    }

    pub fn facets_matching(&self, filter: &str) -> SessionFacets {
        const FACET_CAP: usize = 64;
        let query = FlowQuery::parse(filter);
        let mut domain_map: HashMap<String, FacetEntry> = HashMap::new();
        let mut app_map: HashMap<String, FacetEntry> = HashMap::new();
        for id in &self.order {
            let Some(flow) = self.flows.get(id) else {
                continue;
            };
            if !query.matches(flow, &self.spill) {
                continue;
            }
            let is_err = flow.error.is_some() || flow.status.is_some_and(|code| code >= 400);
            let host = flow.host.trim();
            if !host.is_empty() {
                let key = host.to_ascii_lowercase();
                let entry = domain_map.entry(key.clone()).or_insert_with(|| FacetEntry {
                    label: host.to_string(),
                    token: key,
                    count: 0,
                    error_count: 0,
                });
                entry.count += 1;
                if is_err {
                    entry.error_count += 1;
                }
            }
            if let Some((label, token)) = extract_client_app_facet(&flow.client_ip) {
                let key = token.to_ascii_lowercase();
                let entry = app_map.entry(key).or_insert_with(|| FacetEntry {
                    label,
                    token,
                    count: 0,
                    error_count: 0,
                });
                entry.count += 1;
                if is_err {
                    entry.error_count += 1;
                }
            }
        }
        let sort_facets = |mut items: Vec<FacetEntry>| -> Vec<FacetEntry> {
            items.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.token.cmp(&b.token)));
            items.truncate(FACET_CAP);
            items
        };
        SessionFacets {
            domains: sort_facets(domain_map.into_values().collect()),
            apps: sort_facets(app_map.into_values().collect()),
        }
    }

    #[cfg(test)]
    pub fn filtered_stats(&self, filter: &str) -> FilteredStats {
        let query = FlowQuery::parse(filter);
        let matching = self.order.iter().filter_map(|id| {
            let flow = self.flows.get(id)?;
            query.matches(flow, &self.spill).then_some(flow)
        });
        FilteredStats::from_flows(matching)
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

fn body_groups_match(flow: &Flow, spill: &SpillStore, groups: &[Vec<String>]) -> bool {
    if groups.is_empty() {
        return true;
    }
    let req_spilled = flow.request_spill.and_then(|sref| spill.read(sref));
    let req_bytes = req_spilled.as_deref().unwrap_or(&flow.request_body);
    let request = String::from_utf8_lossy(req_bytes).to_ascii_lowercase();

    let res_spilled = flow.response_spill.and_then(|sref| spill.read(sref));
    let res_bytes = res_spilled.as_deref().unwrap_or(&flow.response_body);
    let response = String::from_utf8_lossy(res_bytes).to_ascii_lowercase();

    groups.iter().all(|group| {
        group.iter().any(|keyword| {
            request.contains(keyword)
                || response.contains(keyword)
                || flow.messages.iter().any(|msg| {
                    msg.text.to_ascii_lowercase().contains(keyword)
                        || msg.kind.to_ascii_lowercase().contains(keyword)
                })
        })
    })
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

/// Combines a base setup filter and a secondary list filter into a two-stage
/// filter string (`\n`-separated) so the secondary filter narrows down the
/// results of the base filter.
pub fn combine_filters(base: &str, secondary: &str) -> String {
    let b = base.trim();
    let s = secondary.trim();
    match (b.is_empty(), s.is_empty()) {
        (true, true) => String::new(),
        (false, true) => b.to_string(),
        (true, false) => s.to_string(),
        (false, false) => format!("{b}\n{s}"),
    }
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

fn comment_has_repeat_id(comment_lower: &str, id_str: &str) -> bool {
    let tag = format!("repeat:{id_str}");
    let mut rest = comment_lower;
    while let Some(pos) = rest.find(&tag) {
        let after = &rest[pos + tag.len()..];
        if after.is_empty() || !after.as_bytes()[0].is_ascii_digit() {
            return true;
        }
        rest = after;
    }
    false
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlowQuery {
    stages: Vec<FlowStage>,
}

impl FlowQuery {
    fn parse(filter: &str) -> Self {
        let stages = filter
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(FlowStage::parse)
            .collect();
        Self { stages }
    }

    fn matches(&self, flow: &Flow, spill: &SpillStore) -> bool {
        self.stages.iter().all(|stage| stage.matches(flow, spill))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlowStage {
    texts: Vec<Vec<String>>,
    hosts: Vec<String>,
    client_ips: Vec<String>,
    path_prefixes: Vec<String>,
    excludes: Vec<String>,
    methods: Vec<String>,
    statuses: Vec<StatusMatch>,
    content_types: Vec<String>,
    bodies: Vec<Vec<String>>,
    headers: Vec<Vec<String>>,
    comments: Vec<Vec<String>>,
    repeats: Vec<Vec<String>>,
    ops: Vec<Vec<String>>,
    pinned_only: bool,
    slow_only: bool,
}

impl FlowStage {
    fn parse(filter: &str) -> Self {
        let normalized = normalize_filter_pipes(filter);
        let mut query = Self {
            texts: Vec::new(),
            hosts: Vec::new(),
            client_ips: Vec::new(),
            path_prefixes: Vec::new(),
            excludes: Vec::new(),
            methods: Vec::new(),
            statuses: Vec::new(),
            content_types: Vec::new(),
            bodies: Vec::new(),
            headers: Vec::new(),
            comments: Vec::new(),
            repeats: Vec::new(),
            ops: Vec::new(),
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
            if let Some(rest) = lower
                .strip_prefix("ip:")
                .or_else(|| lower.strip_prefix("client:"))
                .or_else(|| lower.strip_prefix("app:"))
                .or_else(|| lower.strip_prefix("proc:"))
            {
                query.client_ips.extend(split_or_terms(rest));
                continue;
            }
            if let Some(rest) = lower
                .strip_prefix("note:")
                .or_else(|| lower.strip_prefix("comment:"))
            {
                let group = split_or_terms(rest);
                query.comments.push(group);
                continue;
            }
            if let Some(rest) = lower.strip_prefix("repeat:") {
                let group: Vec<String> = split_or_terms(rest)
                    .into_iter()
                    .map(|term| term.trim_start_matches('#').to_string())
                    .filter(|term| !term.is_empty())
                    .collect();
                query.repeats.push(group);
                continue;
            }
            if let Some(rest) = lower
                .strip_prefix("op:")
                .or_else(|| lower.strip_prefix("gql:"))
                .or_else(|| lower.strip_prefix("operation:"))
            {
                query.ops.push(split_or_terms(rest));
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

    fn matches(&self, flow: &Flow, spill: &SpillStore) -> bool {
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
            .headers
            .iter()
            .all(|group| group.iter().any(|keyword| header_contains(flow, keyword)))
        {
            return false;
        }
        let host = flow.host.to_ascii_lowercase();
        let path = flow.path.to_ascii_lowercase();
        let client_ip = flow.client_ip.to_ascii_lowercase();
        let comment = flow.comment.to_ascii_lowercase();
        if !self.comments.is_empty()
            && !self.comments.iter().all(|group| {
                if group.is_empty() {
                    !comment.is_empty()
                } else {
                    !comment.is_empty() && group.iter().any(|kw| comment.contains(kw))
                }
            })
        {
            return false;
        }
        if !self.repeats.is_empty()
            && !self.repeats.iter().all(|group| {
                if group.is_empty() {
                    comment.contains("repeat:")
                } else {
                    group.iter().any(|target| {
                        flow.id.to_string() == *target || comment_has_repeat_id(&comment, target)
                    })
                }
            })
        {
            return false;
        }
        if !self.hosts.is_empty()
            && !self
                .hosts
                .iter()
                .any(|pattern| host_matches(&host, pattern))
        {
            return false;
        }
        if !self.client_ips.is_empty()
            && !self.client_ips.iter().any(|ip| {
                !client_ip.is_empty()
                    && (client_ip.contains(ip)
                        || (ip.contains('_') && client_ip.contains(&ip.replace('_', " "))))
            })
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
        if !self.texts.iter().all(|group| {
            group.iter().any(|text| {
                host_matches(&host, text)
                    || path.contains(text)
                    || (!client_ip.is_empty() && client_ip.contains(text))
                    || (!comment.is_empty() && comment.contains(text))
            })
        }) {
            return false;
        }
        if !self.ops.is_empty() {
            let req_spilled = flow.request_spill.and_then(|sref| spill.read(sref));
            let req_bytes = req_spilled.as_deref().unwrap_or(&flow.request_body);
            let ops = crate::present::graphql_operations(&flow.path, req_bytes);
            if ops.is_empty() {
                return false;
            }
            if !self.ops.iter().all(|group| {
                if group.is_empty() {
                    !ops.is_empty()
                } else {
                    group.iter().any(|pat| crate::rules::op_matches(pat, &ops))
                }
            }) {
                return false;
            }
        }
        body_groups_match(flow, spill, &self.bodies)
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

    #[test]
    fn large_bodies_spill_to_ring_segments_and_hydrate_or_expire() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        // inline_cap = 1024 B, segment_cap = 4096 B, max_segments = 2, ttl = 60s
        let mut session = Session::with_spill_limits(1024, 4096, 2, Duration::from_secs(60));

        // 1. Small body (<= 1024 B) stays inline in RAM.
        let small = session.open(draft("small.test"));
        session.append_body(small, Direction::Response, b"small-inline-json");
        session.mark_finished_at(small, now);
        assert!(session.get(small).unwrap().response_spill.is_none());
        assert_eq!(
            session.get(small).unwrap().response_body,
            b"small-inline-json"
        );

        // 2. Large body (> 1024 B) spills to segment #1 on mark_finished, freeing RAM.
        let first_large = session.open(draft("bundle1.test"));
        let mut payload1 = vec![b'a'; 3000];
        payload1.extend_from_slice(b"-needle-one");
        session.append_body(first_large, Direction::Response, &payload1);
        let ram_before = session.memory_bytes();
        assert!(ram_before >= payload1.len());
        session.mark_finished_at(first_large, now);
        assert!(session.memory_bytes() < ram_before - 2000);
        assert!(session.get(first_large).unwrap().response_body.is_empty());
        assert!(session.get(first_large).unwrap().response_spill.is_some());

        // Hydrated flow restores the full spilled payload and `body:` search still matches it.
        let hydrated1 = session.hydrated(first_large).unwrap();
        assert_eq!(hydrated1.response_body, payload1);
        assert_eq!(hydrated1.response_expired, None);
        assert_eq!(session.ids_matching("body:needle-one"), vec![first_large]);

        // 3. Pinning promotes a spilled body back to inline memory so it survives rotation.
        let pinned_large = session.open(draft("pinned.test"));
        let payload_pin = vec![b'p'; 3000];
        session.append_body(pinned_large, Direction::Response, &payload_pin);
        session.mark_finished_at(pinned_large, now);
        assert!(session.get(pinned_large).unwrap().response_spill.is_some());
        assert!(session.set_pinned(pinned_large, true));
        assert!(session.get(pinned_large).unwrap().response_spill.is_none());
        assert_eq!(session.get(pinned_large).unwrap().response_body.len(), 3000);

        // 4. Write two more large bodies to force segment #1 out of the 2-segment ring.
        let second_large = session.open(draft("bundle2.test"));
        session.append_body(second_large, Direction::Response, &vec![b'b'; 3000]);
        session.mark_finished_at(second_large, now + Duration::from_secs(1));

        let third_large = session.open(draft("bundle3.test"));
        session.append_body(third_large, Direction::Response, &vec![b'c'; 3000]);
        session.mark_finished_at(third_large, now + Duration::from_secs(2));

        // `first_large` row and metadata remain in the session, while its spilled body is marked expired.
        let expired1 = session.hydrated(first_large).unwrap();
        assert_eq!(expired1.host, "bundle1.test");
        assert!(expired1.response_body.is_empty());
        assert_eq!(expired1.response_expired, Some(payload1.len()));
        // Pinned flow and newer segments remain intact.
        assert_eq!(
            session.hydrated(pinned_large).unwrap().response_body,
            payload_pin
        );
        assert_eq!(
            session.hydrated(third_large).unwrap().response_body.len(),
            3000
        );

        // 5. Periodic TTL GC expires remaining segments after 60s.
        assert!(session.gc_spill(now + Duration::from_secs(120)));
        let expired3 = session.hydrated(third_large).unwrap();
        assert!(expired3.response_body.is_empty());
        assert_eq!(expired3.response_expired, Some(3000));
        // Pinned flow is still in memory.
        assert_eq!(
            session.hydrated(pinned_large).unwrap().response_body,
            payload_pin
        );
    }

    #[test]
    fn orphan_spill_dirs_sweep_and_unix_segments_unlink_on_open() {
        assert_eq!(INLINE_BODY_CAP, 16 * 1024);
        let base = std::env::temp_dir().join(format!("zniff-spill-test-{}", std::process::id()));
        let empty_orphan = base.join("99999999-1");
        let stale_orphan = base.join("99999999-2");
        let own_dir = base.join(format!("{}-1", std::process::id()));
        std::fs::create_dir_all(&empty_orphan).unwrap();
        std::fs::create_dir_all(&stale_orphan).unwrap();
        std::fs::write(stale_orphan.join("seg-000001.dat"), b"stale").unwrap();
        std::fs::create_dir_all(&own_dir).unwrap();
        std::fs::write(own_dir.join("seg-000001.dat"), b"active").unwrap();

        let future = SystemTime::now() + Duration::from_secs(3600);
        sweep_orphan_spill_dirs(&base, future, Duration::from_secs(60));
        assert!(!empty_orphan.exists());
        assert!(!stale_orphan.exists());
        assert!(own_dir.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn comment_app_filter_and_clear_unpinned_work() {
        let mut session = Session::default();
        let a = session.open(draft("api.test"));
        let b = session.open(draft("cdn.test"));
        session.set_client_ip(a, "127.0.0.1 (curl)");
        session.set_client_ip(b, "127.0.0.1 (Safari)");
        session.set_comment(a, "login regression ticket-42");
        session.set_pinned(a, true);

        assert_eq!(session.ids_matching("app:curl"), vec![a]);
        assert_eq!(session.ids_matching("proc:safari"), vec![b]);
        assert_eq!(session.ids_matching("note:"), vec![a]);
        assert_eq!(session.ids_matching("note:ticket-42"), vec![a]);
        assert_eq!(session.ids_matching("comment:regression"), vec![a]);
        assert_eq!(session.ids_matching("ticket-42"), vec![a]);

        assert_eq!(session.clear_unpinned(), 1);
        assert_eq!(session.len(), 1);
        assert_eq!(session.ids_matching(""), vec![a]);
    }

    #[test]
    fn filtered_stats_aggregates_status_bytes_and_latency() {
        let mut session = Session::default();
        let f1 = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "api.test".into(),
            path: "/users".into(),
            url: "https://api.test/users".into(),
            request_headers: Vec::new(),
        });
        session.set_response(f1, 200, Vec::new(), None);
        session.append_body(f1, Direction::Request, b"1234");
        session.append_body(f1, Direction::Response, b"123456");
        session.add_transfer(f1, Direction::Request, 100);
        session.add_transfer(f1, Direction::Response, 300);
        session.mark_finished(f1);
        if let Some(flow) = session.flows.get_mut(&f1) {
            flow.finished_at = Some(flow.started + Duration::from_millis(120));
        }

        let f2 = session.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "POST".into(),
            host: "api.test".into(),
            path: "/slow-checkout".into(),
            url: "https://api.test/slow-checkout".into(),
            request_headers: Vec::new(),
        });
        session.set_response(f2, 502, Vec::new(), None);
        session.add_transfer(f2, Direction::Request, 200);
        session.add_transfer(f2, Direction::Response, 400);
        session.mark_finished(f2);
        if let Some(flow) = session.flows.get_mut(&f2) {
            flow.finished_at = Some(flow.started + Duration::from_millis(480));
        }

        let f3 = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "cdn.test".into(),
            path: "/asset.js".into(),
            url: "https://cdn.test/asset.js".into(),
            request_headers: Vec::new(),
        });
        session.set_error(f3, "connection reset");
        if let Some(flow) = session.flows.get_mut(&f3) {
            flow.finished_at = Some(flow.started + Duration::from_millis(300));
        }

        let f4 = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "api.test".into(),
            path: "/pending".into(),
            url: "https://api.test/pending".into(),
            request_headers: Vec::new(),
        });
        let _ = f4;

        let all = session.filtered_stats("");
        assert_eq!(all.total_count, 4);
        assert_eq!(all.s2xx, 1);
        assert_eq!(all.s3xx, 0);
        assert_eq!(all.s4xx, 0);
        assert_eq!(all.s5xx, 1);
        assert_eq!(all.failed, 1);
        assert_eq!(all.pending, 1);
        assert_eq!(all.total_req_bytes, 300);
        assert_eq!(all.total_res_bytes, 700);
        assert_eq!(all.total_bytes, 1000);
        assert_eq!(all.avg_elapsed_ms, Some(300));
        assert_eq!(
            all.max_elapsed,
            Some((f2, 480, "api.test/slow-checkout".to_string()))
        );

        let filtered = session.filtered_stats("host:api.test 2xx");
        assert_eq!(filtered.total_count, 1);
        assert_eq!(filtered.s2xx, 1);
        assert_eq!(filtered.total_bytes, 400);
        assert_eq!(filtered.avg_elapsed_ms, Some(120));
        assert_eq!(
            filtered.max_elapsed,
            Some((f1, 120, "api.test/users".to_string()))
        );
    }

    #[test]
    fn filter_matches_ws_messages_and_repeat_tags() {
        let mut session = Session::default();
        let f1 = session.open(draft("ws.test"));
        session.add_message(f1, Direction::Response, "文本", "order_created:9981");
        let f2 = session.open(draft("api.test"));
        session.set_comment(f2, "repeat:1 (1/3)");
        let f3 = session.open(draft("api.test"));
        session.set_comment(f3, "repeat:10 (1/2)");

        assert_eq!(session.ids_matching("body:order_created"), vec![f1]);
        assert_eq!(session.ids_matching("repeat:"), vec![f2, f3]);
        assert_eq!(session.ids_matching("repeat:1"), vec![f1, f2]);
        assert_eq!(session.ids_matching("repeat:#10"), vec![f3]);
    }

    #[test]
    fn filter_matches_graphql_operations() {
        let mut session = Session::default();
        let f1 = session.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "POST".into(),
            host: "api.test".into(),
            path: "/graphql".into(),
            url: "https://api.test/graphql".into(),
            request_headers: Vec::new(),
        });
        session.append_body(
            f1,
            Direction::Request,
            br#"{"operationName":"GetCart","query":"query GetCart { cart { id } }"}"#,
        );
        let f2 = session.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "POST".into(),
            host: "api.test".into(),
            path: "/graphql".into(),
            url: "https://api.test/graphql".into(),
            request_headers: Vec::new(),
        });
        session.append_body(
            f2,
            Direction::Request,
            br#"{"query":"mutation CheckoutOrder { checkout { id } }"}"#,
        );
        let f3 = session.open(draft("api.test"));

        assert_eq!(session.ids_matching("op:"), vec![f1, f2]);
        assert_eq!(session.ids_matching("op:GetCart"), vec![f1]);
        assert_eq!(session.ids_matching("gql:Checkout*"), vec![f2]);
        assert_eq!(
            session.ids_matching("op:GetCart|CheckoutOrder"),
            vec![f1, f2]
        );
        let _ = f3;
    }

    #[test]
    fn facets_aggregate_domains_and_apps_sorted_by_count() {
        let mut session = Session::default();
        let f1 = session.open(draft("api.example.com"));
        session.set_client_ip(f1, "127.0.0.1 (curl)");
        session.replace_response(f1, 200, Vec::new(), Vec::new());

        let f2 = session.open(draft("api.example.com"));
        session.set_client_ip(f2, "127.0.0.1 (Google Chrome)");
        session.replace_response(f2, 502, Vec::new(), Vec::new());

        let f3 = session.open(draft("cdn.example.com"));
        session.set_client_ip(f3, "127.0.0.1 (Google Chrome)");
        session.replace_response(f3, 200, Vec::new(), Vec::new());

        let facets = session.facets();
        assert_eq!(
            facets.domains,
            vec![
                FacetEntry {
                    label: "api.example.com".into(),
                    token: "api.example.com".into(),
                    count: 2,
                    error_count: 1,
                },
                FacetEntry {
                    label: "cdn.example.com".into(),
                    token: "cdn.example.com".into(),
                    count: 1,
                    error_count: 0,
                },
            ]
        );
        assert_eq!(
            facets.apps,
            vec![
                FacetEntry {
                    label: "Google Chrome".into(),
                    token: "Google_Chrome".into(),
                    count: 2,
                    error_count: 1,
                },
                FacetEntry {
                    label: "curl".into(),
                    token: "curl".into(),
                    count: 1,
                    error_count: 0,
                },
            ]
        );
        assert_eq!(session.ids_matching("app:Google_Chrome"), vec![f2, f3]);
        assert_eq!(session.ids_matching("app:curl"), vec![f1]);

        let filtered_facets = session.facets_matching("host:api.example.com");
        assert_eq!(filtered_facets.domains.len(), 1);
        assert_eq!(filtered_facets.domains[0].token, "api.example.com");
        assert_eq!(filtered_facets.domains[0].count, 2);
    }

    #[test]
    fn combine_filters_applies_secondary_filter_strictly_within_base_results() {
        let mut session = Session::default();
        let f1 = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "a.example.com".into(),
            path: "/api/orders".into(),
            url: "https://a.example.com/api/orders".into(),
            request_headers: Vec::new(),
        });
        let f2 = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "POST".into(),
            host: "b.example.com".into(),
            path: "/api/users".into(),
            url: "https://b.example.com/api/users".into(),
            request_headers: Vec::new(),
        });
        let f3 = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "c.example.com".into(),
            path: "/api/orders".into(),
            url: "https://c.example.com/api/orders".into(),
            request_headers: Vec::new(),
        });

        let base = "host:a.example.com|b.example.com path:/api";
        assert_eq!(session.ids_matching(base), vec![f1, f2]);

        // Secondary host filter narrows to `a.example.com`, does NOT union with `b.example.com`.
        let combined_a = combine_filters(base, "host:a.example.com");
        assert_eq!(session.ids_matching(&combined_a), vec![f1]);

        // Secondary host filter for `c.example.com` (excluded by base) returns empty.
        let combined_c = combine_filters(base, "host:c.example.com");
        assert!(session.ids_matching(&combined_c).is_empty());

        // Secondary path prefix filter narrows within base `path:/api`.
        let combined_orders = combine_filters(base, "path:/api/orders");
        assert_eq!(session.ids_matching(&combined_orders), vec![f1]);
        let _ = f3;
    }
}
