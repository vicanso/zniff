//! Live watch alert evaluation against the in-memory capture session (E6).

use std::collections::{HashMap, HashSet};

use crate::config::WatchRule;
use crate::session::{Flow, Session};

const RECENT_ID_CAP: usize = 5;

/// Live hit count and recent matched flow IDs for a single watch rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchRuleState {
    pub name: String,
    pub filter: String,
    pub enabled: bool,
    pub hit_count: usize,
    pub recent_ids: Vec<u64>,
}

/// Newly matched flow alert emitted when fresh traffic hits an active watch rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchAlert {
    pub rule_name: String,
    pub filter: String,
    pub flow_id: u64,
    pub target: String,
    pub new_count: usize,
}

/// Snapshot produced by [`WatchTracker::evaluate`] for UI rendering and toast dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WatchEvaluation {
    pub rules: Vec<WatchRuleState>,
    pub total_hits: usize,
    pub active_rules: usize,
    pub new_alerts: Vec<WatchAlert>,
}

/// Tracks seen flow IDs per watch rule so only newly matching flows trigger alerts.
#[derive(Debug, Default)]
pub struct WatchTracker {
    seen_hits: HashMap<String, HashSet<u64>>,
    initialized_rules: HashSet<String>,
    last_session_len: usize,
}

impl WatchTracker {
    /// Evaluates `rules` against `session`, returning live counts and any newly triggered alerts.
    ///
    /// The first time a rule is evaluated against an already-populated session, existing matches
    /// are counted and seeded without firing historical alerts. Any subsequent new flow matching
    /// an enabled rule emits a [`WatchAlert`].
    pub fn evaluate(&mut self, rules: &[WatchRule], session: &Session) -> WatchEvaluation {
        let session_len = session.len();
        if session_len == 0 {
            self.seen_hits.clear();
            self.last_session_len = 0;
        }
        let had_existing_session = session_len > 0;
        self.last_session_len = session_len;

        let active_keys: HashSet<String> = rules
            .iter()
            .filter_map(|r| {
                let f = r.filter.trim();
                (!f.is_empty()).then(|| f.to_ascii_lowercase())
            })
            .collect();
        self.seen_hits.retain(|k, _| active_keys.contains(k));
        self.initialized_rules.retain(|k| active_keys.contains(k));

        let mut out = WatchEvaluation::default();
        for rule in rules {
            let filter = rule.filter.trim();
            if filter.is_empty() {
                continue;
            }
            let key = filter.to_ascii_lowercase();
            let label = if rule.name.trim().is_empty() {
                filter.to_string()
            } else {
                rule.name.trim().to_string()
            };
            if !rule.enabled {
                self.seen_hits.remove(&key);
                self.initialized_rules.remove(&key);
                out.rules.push(WatchRuleState {
                    name: label,
                    filter: filter.to_string(),
                    enabled: false,
                    hit_count: 0,
                    recent_ids: Vec::new(),
                });
                continue;
            }
            out.active_rules += 1;
            let matched_ids: Vec<u64> = session
                .plan_filter(filter)
                .finish()
                .into_iter()
                .map(|(id, _)| id)
                .filter(|id| session.get(*id).is_some_and(|flow| !flow.is_marker()))
                .collect();

            let hit_count = matched_ids.len();
            out.total_hits = out.total_hits.saturating_add(hit_count);
            let recent_ids: Vec<u64> = matched_ids
                .iter()
                .rev()
                .take(RECENT_ID_CAP)
                .copied()
                .collect();

            let is_new_rule = self.initialized_rules.insert(key.clone());
            let seen = self.seen_hits.entry(key).or_default();
            // Prune IDs that were evicted or cleared from the session.
            let matched_set: HashSet<u64> = matched_ids.iter().copied().collect();
            seen.retain(|id| matched_set.contains(id));

            let mut fresh: Vec<u64> = Vec::new();
            for &id in &matched_ids {
                if seen.insert(id) {
                    fresh.push(id);
                }
            }

            // Seed without spamming alerts when a rule is first attached to an existing session.
            if !fresh.is_empty()
                && !(is_new_rule && had_existing_session && hit_count == fresh.len())
            {
                let latest_id = *fresh.last().unwrap();
                let target = session
                    .get(latest_id)
                    .map(format_alert_target)
                    .unwrap_or_else(|| format!("#{latest_id}"));
                out.new_alerts.push(WatchAlert {
                    rule_name: label.clone(),
                    filter: filter.to_string(),
                    flow_id: latest_id,
                    target,
                    new_count: fresh.len(),
                });
            }

            out.rules.push(WatchRuleState {
                name: label,
                filter: filter.to_string(),
                enabled: true,
                hit_count,
                recent_ids,
            });
        }
        out
    }
}

fn format_alert_target(flow: &Flow) -> String {
    let status = match flow.status {
        Some(code) => format!(" {code}"),
        None if flow.error.is_some() => " ERR".to_string(),
        None => String::new(),
    };
    let target = if flow.host.is_empty() {
        flow.path.clone()
    } else if flow.path.starts_with('/') {
        format!("{}{}", flow.host, flow.path)
    } else if flow.path.is_empty() {
        flow.host.clone()
    } else {
        format!("{}/{}", flow.host, flow.path)
    };
    format!("{} {target}{status}", flow.method)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{FlowDraft, Protocol};

    fn draft(host: &str, path: &str) -> FlowDraft {
        FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: host.into(),
            path: path.into(),
            url: format!("https://{host}{path}"),
            request_headers: vec![],
        }
    }

    #[test]
    fn watch_tracker_counts_hits_and_alerts_only_on_new_matches() {
        let mut session = Session::default();
        let f1 = session.open(draft("api.x.com", "/v1/old"));
        session.set_response(f1, 500, vec![], None);

        let rules = vec![WatchRule {
            name: "API 5xx".into(),
            filter: "status:5xx host:api.x.com".into(),
            enabled: true,
        }];
        let mut tracker = WatchTracker::default();

        // First evaluation on already-populated session seeds existing hits without spamming alerts.
        let eval1 = tracker.evaluate(&rules, &session);
        assert_eq!(eval1.total_hits, 1);
        assert_eq!(eval1.active_rules, 1);
        assert!(eval1.new_alerts.is_empty());
        assert_eq!(eval1.rules[0].recent_ids, vec![f1]);

        // New non-matching flow does not alert.
        let f2 = session.open(draft("api.x.com", "/v1/ok"));
        session.set_response(f2, 200, vec![], None);
        let eval2 = tracker.evaluate(&rules, &session);
        assert_eq!(eval2.total_hits, 1);
        assert!(eval2.new_alerts.is_empty());

        // New matching 502 flow triggers one alert.
        let f3 = session.open(draft("api.x.com", "/v1/checkout"));
        session.set_response(f3, 502, vec![], None);
        let eval3 = tracker.evaluate(&rules, &session);
        assert_eq!(eval3.total_hits, 2);
        assert_eq!(eval3.new_alerts.len(), 1);
        assert_eq!(eval3.new_alerts[0].rule_name, "API 5xx");
        assert_eq!(eval3.new_alerts[0].flow_id, f3);
        assert_eq!(eval3.new_alerts[0].new_count, 1);
        assert!(eval3.new_alerts[0].target.contains("api.x.com/v1/checkout"));
        assert!(eval3.new_alerts[0].target.contains("502"));

        // Re-evaluating without new flows does not duplicate alerts.
        let eval4 = tracker.evaluate(&rules, &session);
        assert_eq!(eval4.total_hits, 2);
        assert!(eval4.new_alerts.is_empty());

        // Disabled rules report 0 hits and emit no alerts.
        let disabled = vec![WatchRule {
            name: "API 5xx".into(),
            filter: "status:5xx host:api.x.com".into(),
            enabled: false,
        }];
        let eval5 = tracker.evaluate(&disabled, &session);
        assert_eq!(eval5.total_hits, 0);
        assert_eq!(eval5.active_rules, 0);
        assert!(eval5.new_alerts.is_empty());
    }

    #[test]
    fn watch_tracker_alerts_on_first_match_when_started_on_empty_session() {
        let mut session = Session::default();
        let rules = vec![WatchRule {
            name: "".into(),
            filter: "status:5xx".into(),
            enabled: true,
        }];
        let mut tracker = WatchTracker::default();

        let eval0 = tracker.evaluate(&rules, &session);
        assert_eq!(eval0.total_hits, 0);
        assert!(eval0.new_alerts.is_empty());

        let f1 = session.open(draft("api.x.com", "/boom"));
        session.set_response(f1, 503, vec![], None);
        let eval1 = tracker.evaluate(&rules, &session);
        assert_eq!(eval1.total_hits, 1);
        assert_eq!(eval1.new_alerts.len(), 1);
        assert_eq!(eval1.new_alerts[0].rule_name, "status:5xx");
    }
}
