#[allow(unused_imports)]
use super::*;

pub(super) struct PooledH2 {
    pub(super) sender: H2Sender,
    pub(super) used: Instant,
    pub(super) generation: u64,
    pub(super) conn_info: crate::session::ConnInfo,
    pub(super) first_used: Arc<AtomicBool>,
}

pub(super) type PlainPoolMap = HashMap<(String, u16), Vec<(u64, TcpStream)>>;

pub struct Hub {
    pub(super) session: Mutex<Session>,
    pub(super) generation: Arc<AtomicU64>,
    pub(super) state_tx: watch::Sender<u64>,
    /// Milliseconds since [`body_clock`] of the last streaming UI bump.
    pub(super) body_touch_ms: AtomicU64,
    pub ca: Arc<Ca>,
    pub(super) roots: RootCertStore,
    pub(super) default_client_config: ClientConfig,
    pub(super) bypass: Mutex<BypassRules>,
    pub(super) bypass_text: Mutex<String>,
    pub(super) rules: Mutex<Vec<Rule>>,
    pub(super) attributed_rules: Mutex<Vec<rules::AttributedRule>>,
    pub(super) rules_text: Mutex<String>,
    pub(super) rule_groups: Mutex<Vec<crate::config::RuleGroup>>,
    pub(super) rules_rev: AtomicU64,
    /// Session-only. Off skips every rule without rewriting the text.
    pub(super) apply_rules: AtomicBool,
    /// Session-only. Off drops `break` and `break-res` and keeps the other rules.
    pub(super) apply_breaks: AtomicBool,
    /// Off by default. On records an origin TLS probe that fell back to a tunnel.
    pub(super) show_tunnel_failures: AtomicBool,
    pub(super) upstream: Mutex<Option<Upstream>>,
    pub(super) upstream_text: Mutex<String>,
    pub(super) proxy_auth: Mutex<String>,
    pub(super) allow_hosts: Mutex<AllowHostRules>,
    pub(super) allow_hosts_text: Mutex<String>,
    pub(super) allow_hosts_strict: AtomicBool,
    pub(super) allow_hosts_locked: AtomicBool,
    pub(super) allow_denied: Mutex<crate::allow_hosts::DeniedHostStats>,
    /// Idle plaintext origin sockets, used only on the proxy runtime.
    pub(super) pool: tokio::sync::Mutex<PlainPoolMap>,
    /// Origin HTTP/2 senders. Each browser connection keeps its own lane.
    pub(super) h2_pool: Mutex<HashMap<(String, u16), PooledH2>>,
    pub(super) h2_generation: AtomicU64,
    pub(super) origin_epoch: AtomicU64,
    /// Test-only. The next send on a live browser connection dials again.
    #[cfg(test)]
    pub(super) refresh_h2: AtomicBool,
    pub(super) break_queue: Mutex<std::collections::VecDeque<BreakSlot>>,
    /// One live HTML share. A new call replaces the id.
    pub(super) share: Mutex<Option<crate::share::LiveShare>>,
    /// Off until the user turns on 「接收中继」. Uploads stay a 404 until then.
    pub(super) relay_accept: AtomicBool,
    pub(super) relay_token: Mutex<String>,
    /// Documents pushed by other zniff processes, keyed by share id.
    pub(super) relay_in: Mutex<HashMap<String, InboundShare>>,
}

pub(super) struct InboundShare {
    pub(super) document: Vec<u8>,
    pub(super) bodies: HashMap<u64, Vec<u8>>,
    pub(super) expires: std::time::SystemTime,
}

/// One snapshot the capture process pushes to a relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShareExport {
    pub rev: u64,
    pub document: String,
    pub bodies: Vec<(u64, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SharePoll {
    Ended,
    Unchanged,
    Ready(ShareExport),
}

#[cfg(test)]
pub(crate) fn spilled_flow_ids(document: &str) -> Vec<u64> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(document) else {
        return Vec::new();
    };
    let Some(flows) = value.get("flows").and_then(|item| item.as_array()) else {
        return Vec::new();
    };
    flows
        .iter()
        .filter_map(|flow| {
            let request = flow
                .get("request_body_spilled")
                .and_then(|item| item.as_bool())
                .unwrap_or(false);
            let response = flow
                .get("response_body_spilled")
                .and_then(|item| item.as_bool())
                .unwrap_or(false);
            if request || response {
                flow.get("id").and_then(|item| item.as_u64())
            } else {
                None
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamKind {
    Http,
    Socks5 { remote_dns: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    pub kind: UpstreamKind,
    pub host: String,
    pub port: u16,
    pub auth_header: Option<String>,
    pub socks5_auth: Option<(String, String)>,
}

impl Upstream {
    pub fn is_http_proxy(&self) -> bool {
        matches!(self.kind, UpstreamKind::Http)
    }
}

pub(super) struct BreakSlot {
    pub(super) id: u64,
    pub(super) side: BreakSide,
    pub(super) touched: Instant,
    pub(super) reply: Option<tokio::sync::oneshot::Sender<BreakDecision>>,
}

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
    /// Replaces the response status. Ignored for a request break.
    pub status: Option<u16>,
}

/// Optional edits for one replay. Empty method or URL keeps the stored request.
#[derive(Debug, Default, Clone)]
pub struct ReplayOverride {
    pub method: Option<String>,
    pub url: Option<String>,
    pub headers: Option<Vec<(String, String)>>,
    pub body: Option<Vec<u8>>,
}

impl Hub {
    pub fn new(ca: Arc<Ca>) -> Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let default_client_config = ClientConfig::builder()
            .with_root_certificates(roots.clone())
            .with_no_client_auth();
        let (state_tx, _) = watch::channel(0_u64);
        Ok(Self {
            session: Mutex::new(Session::default()),
            generation: Arc::new(AtomicU64::new(1)),
            state_tx,
            body_touch_ms: AtomicU64::new(0),
            ca,
            roots,
            default_client_config,
            bypass: Mutex::new(BypassRules::default()),
            bypass_text: Mutex::new(String::new()),
            rules: Mutex::new(Vec::new()),
            attributed_rules: Mutex::new(Vec::new()),
            rules_text: Mutex::new(String::new()),
            rule_groups: Mutex::new(Vec::new()),
            rules_rev: AtomicU64::new(0),
            apply_rules: AtomicBool::new(true),
            apply_breaks: AtomicBool::new(true),
            show_tunnel_failures: AtomicBool::new(false),
            upstream: Mutex::new(None),
            upstream_text: Mutex::new(String::new()),
            proxy_auth: Mutex::new(String::new()),
            allow_hosts: Mutex::new(AllowHostRules::default()),
            allow_hosts_text: Mutex::new(String::new()),
            allow_hosts_strict: AtomicBool::new(false),
            allow_hosts_locked: AtomicBool::new(false),
            allow_denied: Mutex::new(crate::allow_hosts::DeniedHostStats::default()),
            pool: tokio::sync::Mutex::new(HashMap::new()),
            h2_pool: Mutex::new(HashMap::new()),
            h2_generation: AtomicU64::new(1),
            origin_epoch: AtomicU64::new(1),
            #[cfg(test)]
            refresh_h2: AtomicBool::new(false),
            break_queue: Mutex::new(std::collections::VecDeque::new()),
            share: Mutex::new(None),
            relay_accept: AtomicBool::new(false),
            relay_token: Mutex::new(String::new()),
            relay_in: Mutex::new(HashMap::new()),
        })
    }

    pub fn set_relay_ingress(&self, accept: bool, token: &str) {
        self.set_relay_token(token);
        self.relay_accept.store(accept, Ordering::Relaxed);
    }

    pub fn set_relay_token(&self, token: &str) {
        *self
            .relay_token
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = crate::relay::usable_token(token);
    }

    pub fn set_relay_accept(&self, accept: bool) {
        self.relay_accept.store(accept, Ordering::Relaxed);
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

    pub(super) fn relay_authorized(&self, authorization: Option<&str>) -> bool {
        let expected = self
            .relay_token
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        crate::relay::bearer_ok(&expected, authorization)
    }

    /// Replaces any previous link. The id is unguessable and dies at `ttl`.
    #[cfg(test)]
    pub fn begin_share(
        &self,
        ttl: std::time::Duration,
        filter: &str,
    ) -> (String, std::time::SystemTime) {
        self.begin_share_with_redact(ttl, filter, crate::share::RedactOptions::default())
    }

    pub fn begin_share_with_redact(
        &self,
        ttl: std::time::Duration,
        filter: &str,
        redact: crate::share::RedactOptions,
    ) -> (String, std::time::SystemTime) {
        let id = crate::share::new_id();
        let expires = std::time::SystemTime::now() + ttl;
        *self.share.lock().unwrap_or_else(|err| err.into_inner()) = Some(crate::share::LiveShare {
            id: id.clone(),
            expires,
            filter: filter.to_string(),
            redact,
        });
        (id, expires)
    }

    pub fn end_share(&self) {
        *self.share.lock().unwrap_or_else(|err| err.into_inner()) = None;
    }

    /// The current id while it is still before its deadline.
    pub fn share_id(&self) -> Option<String> {
        let mut slot = self.share.lock().unwrap_or_else(|err| err.into_inner());
        let share = slot.as_ref()?;
        if std::time::SystemTime::now() < share.expires {
            return Some(share.id.clone());
        }
        *slot = None;
        None
    }

    #[cfg(test)]
    pub(super) fn share_document(&self, id: &str) -> Option<String> {
        self.share_document_since(id, None)
    }

    pub(super) fn share_document_since(&self, id: &str, since: Option<u64>) -> Option<String> {
        let mut slot = self.share.lock().unwrap_or_else(|err| err.into_inner());
        let share = slot.as_ref()?;
        let now = std::time::SystemTime::now();
        if !crate::share::grant_ok(share, id, now) {
            if share.id == id {
                *slot = None;
            }
            return None;
        }
        let expires = share.expires;
        let filter = share.filter.clone();
        let redact = share.redact.clone();
        drop(slot);
        let (plan, rev) =
            self.with_session(|session| (session.plan_filter(&filter), session.rev()));
        let matched = plan.finish();
        let omitted = matched.len().saturating_sub(crate::share::FLOW_CAP);
        let window_ids = &matched[omitted..];
        let flows = self.with_session(|session| {
            window_ids
                .iter()
                .filter_map(|(id, _)| session.get(*id).cloned())
                .collect::<Vec<_>>()
        });
        Some(
            crate::share::document_window_with_redact(
                omitted, &flows, expires, &filter, rev, since, &redact,
            )
            .2,
        )
    }

    pub(super) fn share_flow_body(&self, id: &str, flow_id: u64) -> Option<String> {
        let mut slot = self.share.lock().unwrap_or_else(|err| err.into_inner());
        let share = slot.as_ref()?;
        let now = std::time::SystemTime::now();
        if !crate::share::grant_ok(share, id, now) {
            if share.id == id {
                *slot = None;
            }
            return None;
        }
        let filter = share.filter.clone();
        let redact = share.redact.clone();
        drop(slot);
        let flow = self.with_session(|session| session.hydrated_matching(flow_id, &filter))?;
        Some(crate::share::flow_body_document_with_redact(&flow, &redact))
    }

    /// Full share document plus any spilled bodies not already pushed.
    /// Returns [`SharePoll::Unchanged`] with zero allocations or disk I/O
    /// whenever `session.rev()` matches `synced_rev`.
    pub(crate) fn share_export_if_changed(
        &self,
        id: &str,
        synced_rev: Option<u64>,
        pushed_bodies: &HashSet<u64>,
    ) -> SharePoll {
        let (expires, filter, redact) = {
            let mut slot = self.share.lock().unwrap_or_else(|err| err.into_inner());
            let Some(share) = slot.as_ref() else {
                return SharePoll::Ended;
            };
            let now = std::time::SystemTime::now();
            if !crate::share::grant_ok(share, id, now) {
                if share.id == id {
                    *slot = None;
                }
                return SharePoll::Ended;
            }
            (share.expires, share.filter.clone(), share.redact.clone())
        };
        let current_rev = self.with_session(|session| session.rev());
        if synced_rev == Some(current_rev) {
            return SharePoll::Unchanged;
        }
        let (plan, session_rev) =
            self.with_session(|session| (session.plan_filter(&filter), session.rev()));
        let matched = plan.finish();
        let omitted = matched.len().saturating_sub(crate::share::FLOW_CAP);
        let window_ids = &matched[omitted..];
        let flows = self.with_session(|session| {
            window_ids
                .iter()
                .filter_map(|(id, _)| session.get(*id).cloned())
                .collect::<Vec<_>>()
        });
        let (rev, spilled_ids, document) = crate::share::document_window_with_redact(
            omitted,
            &flows,
            expires,
            &filter,
            session_rev,
            None,
            &redact,
        );
        let mut bodies = Vec::new();
        for flow_id in spilled_ids {
            if pushed_bodies.contains(&flow_id) {
                continue;
            }
            if let Some(body) = self.share_flow_body(id, flow_id) {
                bodies.push((flow_id, body));
            }
        }
        SharePoll::Ready(ShareExport {
            rev,
            document,
            bodies,
        })
    }

    /// Full share document plus any spilled bodies the page will ask for.
    /// `rev` is the document revision. A later call with the same `rev` has
    /// the same flow list.
    #[allow(dead_code)]
    pub(crate) fn share_export(&self, id: &str) -> Option<ShareExport> {
        match self.share_export_if_changed(id, None, &HashSet::new()) {
            SharePoll::Ready(export) => Some(export),
            SharePoll::Ended | SharePoll::Unchanged => None,
        }
    }

    pub(super) fn put_inbound_flows(
        &self,
        id: &str,
        body: &[u8],
    ) -> Result<(), crate::relay::RelayStatus> {
        let now = std::time::SystemTime::now();
        let (bytes, expires) = crate::relay::prepare_flows(body, now)?;
        let parsed =
            crate::share::parse_inbound_share(&bytes).ok_or(crate::relay::RelayStatus {
                status: 400,
                reason: "Bad Request",
                message: "flows.json must be a JSON object",
            })?;
        self.gc_inbound(now);
        {
            let mut slots = self.relay_in.lock().unwrap_or_else(|err| err.into_inner());
            if !slots.contains_key(id) && slots.len() >= crate::relay::SLOT_CAP {
                return Err(crate::relay::RelayStatus {
                    status: 429,
                    reason: "Too Many Requests",
                    message: "too many shares",
                });
            }
            match slots.get_mut(id) {
                Some(slot) => {
                    slot.document = bytes;
                    slot.expires = expires;
                }
                None => {
                    slots.insert(
                        id.to_string(),
                        InboundShare {
                            document: bytes,
                            bodies: HashMap::new(),
                            expires,
                        },
                    );
                }
            }
        }
        let drafts = relay_drafts(&parsed);
        self.with_session(|session| session.sync_relay_share(id, drafts));
        self.touch();
        Ok(())
    }

    pub(super) fn put_inbound_body(
        &self,
        id: &str,
        flow_id: u64,
        body: &[u8],
    ) -> Result<(), crate::relay::RelayStatus> {
        if body.is_empty() || body.len() > crate::relay::MAX_BODY {
            return Err(crate::relay::RelayStatus {
                status: 413,
                reason: "Content Too Large",
                message: "body is too large",
            });
        }
        if serde_json::from_slice::<serde_json::Value>(body).is_err() {
            return Err(crate::relay::RelayStatus {
                status: 400,
                reason: "Bad Request",
                message: "flow body must be JSON",
            });
        }
        let now = std::time::SystemTime::now();
        self.gc_inbound(now);
        {
            let mut slots = self.relay_in.lock().unwrap_or_else(|err| err.into_inner());
            let Some(slot) = slots.get_mut(id) else {
                return Err(crate::relay::RelayStatus {
                    status: 409,
                    reason: "Conflict",
                    message: "upload flows.json first",
                });
            };
            slot.bodies.insert(flow_id, body.to_vec());
        }
        if let Some(parsed) = crate::share::parse_inbound_body(body) {
            let messages = shared_messages(&parsed.messages);
            self.with_session(|session| {
                session.apply_relay_body(
                    id,
                    flow_id,
                    parsed.request_body.into_bytes(),
                    parsed.response_body.into_bytes(),
                    messages,
                );
            });
            self.touch();
        }
        Ok(())
    }

    pub(super) fn delete_inbound(&self, id: &str) {
        self.relay_in
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(id);
        self.with_session(|session| {
            session.drop_relay_source(id);
        });
        self.touch();
    }

    pub(super) fn gc_inbound(&self, now: std::time::SystemTime) {
        let expired: Vec<String> = {
            let mut slots = self.relay_in.lock().unwrap_or_else(|err| err.into_inner());
            let expired: Vec<String> = slots
                .iter()
                .filter(|(_, slot)| now >= slot.expires)
                .map(|(id, _)| id.clone())
                .collect();
            for id in &expired {
                slots.remove(id);
            }
            expired
        };
        if expired.is_empty() {
            return;
        }
        self.with_session(|session| {
            for id in &expired {
                session.drop_relay_source(id);
            }
        });
        self.touch();
    }

    pub(super) fn inbound_document(&self, id: &str) -> Option<String> {
        let now = std::time::SystemTime::now();
        self.gc_inbound(now);
        let slots = self.relay_in.lock().unwrap_or_else(|err| err.into_inner());
        let slot = slots.get(id)?;
        if now >= slot.expires {
            return None;
        }
        Some(String::from_utf8_lossy(&slot.document).into_owned())
    }

    pub(super) fn inbound_body(&self, id: &str, flow_id: u64) -> Option<Vec<u8>> {
        let now = std::time::SystemTime::now();
        self.gc_inbound(now);
        let slots = self.relay_in.lock().unwrap_or_else(|err| err.into_inner());
        let slot = slots.get(id)?;
        if now >= slot.expires {
            return None;
        }
        slot.bodies.get(&flow_id).cloned()
    }

    pub fn set_bypass(&self, text: &str) {
        *self
            .bypass_text
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = text.to_string();
        *self.bypass.lock().unwrap_or_else(|err| err.into_inner()) = BypassRules::parse(text);
    }

    pub fn bypass_text(&self) -> String {
        self.bypass_text
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub fn pac_script(&self, proxy_authority: &str) -> String {
        let allow = self
            .allow_hosts
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        self.bypass
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .pac_script_with_allow_hosts(proxy_authority, &allow)
    }

    fn rebuild_rules(&self) {
        let workspace = self
            .rules_text
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        let groups = self
            .rule_groups
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        let attributed = rules::parse_rules_attributed(&groups, &workspace);
        let next_rules: Vec<Rule> = attributed.iter().map(|entry| entry.rule.clone()).collect();
        let origin_rules_changed = {
            let mut guard = self.rules.lock().unwrap_or_else(|err| err.into_inner());
            let changed = origin_dial_rules(&guard) != origin_dial_rules(&next_rules);
            *guard = next_rules;
            changed
        };
        *self
            .attributed_rules
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = attributed;
        self.rules_rev.fetch_add(1, Ordering::Relaxed);
        if origin_rules_changed {
            self.invalidate_origin_pools();
        }
    }

    pub fn set_rules(&self, text: &str) {
        *self
            .rules_text
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = text.to_string();
        self.rebuild_rules();
    }

    pub fn set_rule_groups(&self, groups: &[crate::config::RuleGroup]) {
        *self
            .rule_groups
            .lock()
            .unwrap_or_else(|err| err.into_inner()) =
            crate::config::sanitize_rule_groups(groups.to_vec());
        self.rebuild_rules();
    }

    pub fn rule_groups(&self) -> Vec<crate::config::RuleGroup> {
        self.rule_groups
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub fn effective_rules_count(&self) -> usize {
        self.rules
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .len()
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
        let prev = self.apply_rules.swap(enabled, Ordering::Relaxed);
        if prev != enabled {
            let has_origin_dial_rules = self
                .rules
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .iter()
                .any(|r| {
                    matches!(
                        r,
                        rules::Rule::ClientCert { .. } | rules::Rule::Resolve { .. }
                    )
                });
            if has_origin_dial_rules {
                self.invalidate_origin_pools();
            }
        }
    }

    pub fn rules_enabled(&self) -> bool {
        self.apply_rules.load(Ordering::Relaxed)
    }

    pub fn set_breaks_enabled(&self, enabled: bool) {
        self.apply_breaks.store(enabled, Ordering::Relaxed);
    }

    pub fn breaks_enabled(&self) -> bool {
        self.apply_breaks.load(Ordering::Relaxed)
    }

    pub fn set_show_tunnel_failures(&self, show: bool) {
        self.show_tunnel_failures.store(show, Ordering::Relaxed);
    }

    pub fn tunnel_failures_shown(&self) -> bool {
        self.show_tunnel_failures.load(Ordering::Relaxed)
    }

    pub fn set_upstream(&self, text: &str) {
        *self
            .upstream_text
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = text.to_string();
        let next = parse_upstream(text);
        let changed = {
            let mut guard = self.upstream.lock().unwrap_or_else(|err| err.into_inner());
            let changed = *guard != next;
            *guard = next;
            changed
        };
        if changed {
            self.invalidate_origin_pools();
        }
    }

    pub(super) fn invalidate_origin_pools(&self) {
        self.origin_epoch.fetch_add(1, Ordering::AcqRel);
        self.h2_pool
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clear();
        if let Ok(mut pool) = self.pool.try_lock() {
            pool.clear();
        }
    }

    pub fn upstream_text(&self) -> String {
        self.upstream_text
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub(super) fn upstream(&self) -> Option<Upstream> {
        self.upstream
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub(super) fn resolve_for_host(&self, host: &str) -> Option<(String, Option<u16>)> {
        if !self.apply_rules.load(Ordering::Relaxed) {
            return None;
        }
        let rules = self.rules.lock().unwrap_or_else(|err| err.into_inner());
        rules::custom_resolve(&rules, host)
    }

    pub(super) fn upstream_for_host(&self, host: &str) -> Option<Upstream> {
        if self.resolve_for_host(host).is_some() {
            None
        } else {
            self.upstream()
        }
    }

    pub fn set_proxy_auth(&self, auth: &str) {
        *self
            .proxy_auth
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = auth.trim().to_string();
    }

    /// Configures host allow-list policy, strictness, and CLI/env lock state on startup.
    pub fn configure_allow_hosts(&self, text: &str, strict: bool, locked: bool) {
        let trimmed = text.trim().to_string();
        self.allow_hosts_strict.store(strict, Ordering::Relaxed);
        self.allow_hosts_locked.store(locked, Ordering::Relaxed);
        *self
            .allow_hosts_text
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = trimmed.clone();
        *self
            .allow_hosts
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = AllowHostRules::parse(&trimmed, strict);
    }

    /// Updates editable allow-list rules unless locked by startup flags or environment variables.
    pub fn set_allow_hosts(&self, text: &str) -> std::result::Result<(), String> {
        if self.allow_hosts_locked.load(Ordering::Relaxed) {
            return Err("允许域名已由启动参数或环境变量锁定".to_string());
        }
        let trimmed = text.trim().to_string();
        let strict = self.allow_hosts_strict.load(Ordering::Relaxed);
        *self
            .allow_hosts_text
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = trimmed.clone();
        *self
            .allow_hosts
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = AllowHostRules::parse(&trimmed, strict);
        Ok(())
    }

    /// Returns the raw configured allow-hosts text.
    pub fn allow_hosts_text(&self) -> String {
        self.allow_hosts_text
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Returns whether strict deny-by-default host mode is enabled.
    pub fn allow_hosts_strict(&self) -> bool {
        self.allow_hosts_strict.load(Ordering::Relaxed)
    }

    /// Returns whether allow-hosts editing is locked by startup CLI flags or environment variables.
    pub fn allow_hosts_locked(&self) -> bool {
        self.allow_hosts_locked.load(Ordering::Relaxed)
    }

    /// Returns whether the current policy denies every proxied host.
    pub fn allow_hosts_denies_all(&self) -> bool {
        self.allow_hosts
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .denies_all()
    }

    /// Checks whether `host` is permitted by the active allow-list policy.
    pub(super) fn host_allowed(&self, host: &str) -> bool {
        self.allow_hosts
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .allows(host)
    }

    /// Records a request rejected by `allow_hosts` and wakes UI/web state observers.
    pub fn record_allow_denied(&self, host: &str) {
        self.allow_denied
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .record(host);
        self.touch();
    }

    /// Returns a bounded summary of requests rejected by `allow_hosts`.
    pub fn allow_denied_summary(&self) -> crate::allow_hosts::AllowDeniedSummary {
        self.allow_denied
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .summary()
    }

    /// Resets the `allow_hosts` denial summary counters.
    pub fn reset_allow_denied(&self) {
        let changed = self
            .allow_denied
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .reset();
        if changed {
            self.touch();
        }
    }

    pub fn proxy_auth_text(&self) -> String {
        self.proxy_auth
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub(super) fn proxy_auth_ok(&self, headers: &[(String, String)]) -> bool {
        let expected_raw = self
            .proxy_auth
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        if expected_raw.is_empty() {
            return true;
        }
        let expected = if expected_raw.contains(':') {
            expected_raw
        } else {
            format!("{expected_raw}:")
        };
        headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("proxy-authorization"))
            .any(|(_, value)| verify_basic_auth(&expected, value))
    }

    pub(super) fn skips_tls(&self, host: &str) -> bool {
        self.bypass
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .skips(host)
    }

    pub(super) fn needs_request_body_buffer(&self, host: &str, path: &str) -> bool {
        if !self.apply_rules.load(Ordering::Relaxed) {
            return false;
        }
        let rules = self
            .rules
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        rules::has_op_rules(&rules, host, path)
            || crate::rule_vars::rules_need_request_body(&rules, host, path)
            || rules::has_request_hooks(&rules, host, path)
    }

    #[cfg(test)]
    pub(super) fn plan(&self, host: &str, path: &str) -> rules::Effect {
        self.plan_with_ops(host, path, &[])
    }

    pub(super) fn plan_with_ops(&self, host: &str, path: &str, ops: &[String]) -> rules::Effect {
        if !self.apply_rules.load(Ordering::Relaxed) {
            return rules::Effect::default();
        }
        let rules = self
            .rules
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        let effect = rules::effect_with_ops(&rules, host, path, ops);
        if self.apply_breaks.load(Ordering::Relaxed) {
            effect
        } else {
            rules::without_breaks(effect)
        }
    }

    pub(super) fn server_config_for_host(
        &self,
        host: &str,
        alpn: &[u8],
    ) -> Result<Arc<rustls::ServerConfig>> {
        let rule_cert = if self.apply_rules.load(Ordering::Relaxed) {
            let rules = self
                .rules
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone();
            rules::custom_cert(&rules, host)
        } else {
            None
        };
        let custom_ref = rule_cert
            .as_ref()
            .map(|(cert, key)| (cert.as_path(), key.as_deref()));
        self.ca.server_config_with_custom(host, alpn, custom_ref)
    }

    pub(super) fn has_client_cert_for_host(&self, host: &str) -> bool {
        if self.apply_rules.load(Ordering::Relaxed) {
            let rules = self
                .rules
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone();
            if rules::custom_client_cert(&rules, host).is_some() {
                return true;
            }
        }
        self.ca.has_client_cert_in_dir(host)
    }

    pub(super) fn client_config_for_host(
        &self,
        host: &str,
        alpn: &[Vec<u8>],
    ) -> Result<(ClientConfig, Option<String>)> {
        let rule_cert = if self.apply_rules.load(Ordering::Relaxed) {
            let rules = self
                .rules
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone();
            rules::custom_client_cert(&rules, host)
        } else {
            None
        };
        let custom_ref = rule_cert
            .as_ref()
            .map(|(cert, key)| (cert.as_path(), key.as_deref()));
        let identity = self
            .ca
            .client_identity_for_host(host, custom_ref)
            .with_context(|| format!("加载源站 mTLS 客户端证书 ({host})"))?;
        let mut presented_cert = None;
        let mut config = match identity {
            Some(id) => {
                let cert_display = id.cert_path.display().to_string();
                let key_display = id.key_path.as_ref().map(|p| p.display().to_string());
                let mut cfg = ClientConfig::builder()
                    .with_root_certificates(self.roots.clone())
                    .with_client_auth_cert(id.certs, id.key)
                    .with_context(|| match &key_display {
                        Some(kp) => format!(
                            "配置源站 mTLS 客户端证书 ({host}, cert={cert_display}, key={kp})"
                        ),
                        None => {
                            format!("配置源站 mTLS 客户端证书 ({host}, pem={cert_display})")
                        }
                    })?;
                cfg.resumption = self.default_client_config.resumption.clone();
                presented_cert = Some(cert_display);
                cfg
            }
            None => self.default_client_config.clone(),
        };
        config.alpn_protocols = alpn.to_vec();
        Ok((config, presented_cert))
    }

    #[cfg(test)]
    pub(super) fn note_rules(&self, id: u64, host: &str, path: &str) {
        self.note_rules_with_ops(id, host, path, &[]);
    }

    pub(super) fn note_rules_with_ops(&self, id: u64, host: &str, path: &str, ops: &[String]) {
        if !self.apply_rules.load(Ordering::Relaxed) {
            return;
        }
        let attributed = self
            .attributed_rules
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        let include_breaks = self.apply_breaks.load(Ordering::Relaxed);
        let (hit, details) =
            rules::hit_details_with_ops(&attributed, host, path, ops, include_breaks);
        if !hit.is_empty() || !details.is_empty() {
            self.with_session(|session| session.set_rule_hit_details(id, hit, details));
            self.touch();
        }
    }

    pub(super) fn note_resolve_hit(&self, id: u64, host: &str) {
        if !self.apply_rules.load(Ordering::Relaxed) {
            return;
        }
        let attributed = self
            .attributed_rules
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        if let Some(hit) = rules::matching_resolve_hit(&attributed, host) {
            self.with_session(|session| session.append_rule_hit(id, hit));
            self.touch();
        }
    }
}
