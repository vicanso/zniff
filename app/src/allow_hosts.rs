//! Host allow-list policy enforced across the proxy capture port, map-remote, and replay.
//!
//! When `strict` is false (desktop GUI and local `zniff --web`), an empty allow-list permits all
//! non-excluded hosts. When `strict` is true (Docker default), an empty or exclusion-only list
//! denies all hosts until an explicit positive rule (`example.com`, `*.example.com`, or `*`) is set.

use std::collections::{HashMap, HashSet};

/// Maximum number of distinct denied hosts tracked in memory at once.
pub const DENIED_HOST_CAP: usize = 64;

/// Snapshot of requests blocked by `allow_hosts`, sorted by hit count descending then host ascending.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct AllowDeniedSummary {
    pub total: u64,
    pub hosts: Vec<(String, u64)>,
}

/// Bounded in-memory counter of requests rejected by `allow_hosts`.
#[derive(Debug, Clone, Default)]
pub struct DeniedHostStats {
    total: u64,
    hosts: HashMap<String, u64>,
}

impl DeniedHostStats {
    /// Records one denied request for `host`.
    ///
    /// Once [`DENIED_HOST_CAP`] distinct hosts are tracked, new hostnames still increment
    /// `total` without growing the map, while already-tracked hosts continue incrementing.
    pub fn record(&mut self, host: &str) {
        self.total = self.total.saturating_add(1);
        let key = normalize_host(host);
        if key.is_empty() {
            return;
        }
        if let Some(count) = self.hosts.get_mut(&key) {
            *count = count.saturating_add(1);
        } else if self.hosts.len() < DENIED_HOST_CAP {
            self.hosts.insert(key, 1);
        }
    }

    /// Returns the total blocked count and tracked hosts sorted by count desc, then host asc.
    pub fn summary(&self) -> AllowDeniedSummary {
        let mut hosts: Vec<(String, u64)> = self
            .hosts
            .iter()
            .map(|(host, count)| (host.clone(), *count))
            .collect();
        hosts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        AllowDeniedSummary {
            total: self.total,
            hosts,
        }
    }

    /// Clears all tracked counts and returns `true` if any state changed.
    pub fn reset(&mut self) -> bool {
        if self.total == 0 && self.hosts.is_empty() {
            return false;
        }
        self.total = 0;
        self.hosts.clear();
        true
    }
}

/// Appends `host` as an exact allow rule to `existing` unless already present as a positive token.
pub fn append_allow_host(existing: &str, host: &str) -> String {
    let target = normalize_host(host);
    let trimmed = existing.trim();
    if target.is_empty() {
        return trimmed.to_string();
    }
    let already_listed = trimmed.lines().any(|line| {
        let clean = line.split('#').next().unwrap_or("");
        clean
            .split(|ch: char| ch.is_whitespace() || ch == ',' || ch == ';' || ch == '|')
            .filter(|tok| !tok.is_empty() && !tok.starts_with('-'))
            .any(|tok| normalize_host(tok) == target)
    });
    if already_listed {
        return trimmed.to_string();
    }
    if trimmed.is_empty() {
        target
    } else {
        format!("{trimmed}\n{target}")
    }
}

/// Parsed host restriction rules for proxied and replayed requests.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AllowHostRules {
    strict: bool,
    allow_all: bool,
    allow_exact: HashSet<String>,
    /// Subdomain suffixes with leading dot (e.g. `".vicanso.dev"` for `*.vicanso.dev`).
    allow_suffixes: Vec<String>,
    deny_all: bool,
    deny_exact: HashSet<String>,
    deny_suffixes: Vec<String>,
}

impl AllowHostRules {
    /// Parses newline/comma/whitespace-separated host rules with optional `#` line comments.
    pub fn parse(text: &str, strict: bool) -> Self {
        let mut rules = Self {
            strict,
            ..Self::default()
        };
        for line in text.lines() {
            let clean = line.split('#').next().unwrap_or("");
            for raw in
                clean.split(|ch: char| ch.is_whitespace() || ch == ',' || ch == ';' || ch == '|')
            {
                let token = raw.trim();
                if token.is_empty() {
                    continue;
                }
                let (negated, rest) = match token.strip_prefix('-') {
                    Some(stripped) => (true, stripped.trim()),
                    None => (false, token),
                };
                if rest.is_empty() {
                    continue;
                }
                let host = normalize_host(rest);
                if host.is_empty() {
                    continue;
                }
                rules.insert_rule(negated, host);
            }
        }
        rules
    }

    fn insert_rule(&mut self, negated: bool, host: String) {
        let (all, exact, suffixes) = if negated {
            (
                &mut self.deny_all,
                &mut self.deny_exact,
                &mut self.deny_suffixes,
            )
        } else {
            (
                &mut self.allow_all,
                &mut self.allow_exact,
                &mut self.allow_suffixes,
            )
        };
        if host == "*" {
            *all = true;
        } else if let Some(rest) = host.strip_prefix("*.") {
            if !rest.is_empty() {
                let dot_suffix = format!(".{rest}");
                if !suffixes.contains(&dot_suffix) {
                    suffixes.push(dot_suffix);
                }
            }
        } else {
            exact.insert(host);
        }
    }

    fn has_positive(&self) -> bool {
        self.allow_all || !self.allow_exact.is_empty() || !self.allow_suffixes.is_empty()
    }

    /// Returns whether `host` is permitted by this policy.
    pub fn allows(&self, host: &str) -> bool {
        let host = normalize_host(host);
        if host.is_empty() {
            return false;
        }
        if matches_host_set(&host, self.deny_all, &self.deny_exact, &self.deny_suffixes) {
            return false;
        }
        if !self.has_positive() {
            return !self.strict;
        }
        matches_host_set(
            &host,
            self.allow_all,
            &self.allow_exact,
            &self.allow_suffixes,
        )
    }

    /// Returns `true` when every host is denied (strict empty/exclude-only, or explicit `-*`).
    pub fn denies_all(&self) -> bool {
        self.deny_all || (self.strict && !self.has_positive())
    }

    /// Returns `true` when any host restriction or exclusion is active.
    pub fn is_restricted(&self) -> bool {
        self.denies_all()
            || !self.deny_exact.is_empty()
            || !self.deny_suffixes.is_empty()
            || (!self.allow_all && self.has_positive())
    }

    /// Returns whether strict deny-by-default mode is active.
    #[allow(dead_code)]
    pub fn strict(&self) -> bool {
        self.strict
    }

    /// Builds a JS boolean expression over normalized PAC variable `h` when hosts are filtered.
    /// Returns `None` when completely unrestricted.
    pub fn pac_allow_expr(&self) -> Option<String> {
        if self.denies_all() {
            return Some("false".to_string());
        }
        if !self.is_restricted() {
            return None;
        }
        let pos = (self.has_positive() && !self.allow_all)
            .then(|| pac_match_expr(&self.allow_exact, &self.allow_suffixes));
        let deny = (!self.deny_exact.is_empty() || !self.deny_suffixes.is_empty())
            .then(|| pac_match_expr(&self.deny_exact, &self.deny_suffixes));
        match (pos, deny) {
            (Some(p), Some(d)) => Some(format!("({p}) && !({d})")),
            (Some(p), None) => Some(p),
            (None, Some(d)) => Some(format!("!({d})")),
            (None, None) => None,
        }
    }
}

fn matches_host_set(host: &str, all: bool, exact: &HashSet<String>, suffixes: &[String]) -> bool {
    all || exact.contains(host) || suffixes.iter().any(|suffix| host.ends_with(suffix))
}

fn pac_match_expr(exact: &HashSet<String>, suffixes: &[String]) -> String {
    let mut checks = Vec::new();
    let mut exact_list: Vec<&str> = exact.iter().map(String::as_str).collect();
    exact_list.sort_unstable();
    for host in exact_list {
        if let Ok(lit) = serde_json::to_string(host) {
            checks.push(format!("h === {lit}"));
        }
    }
    let mut suffix_list: Vec<&str> = suffixes.iter().map(String::as_str).collect();
    suffix_list.sort_unstable();
    for suffix in suffix_list {
        if let Ok(lit) = serde_json::to_string(suffix) {
            checks.push(format!("dnsDomainIs(h, {lit})"));
        }
    }
    if checks.is_empty() {
        "false".to_string()
    } else {
        checks.join(" || ")
    }
}

/// Normalizes a domain or IP literal consistently with `bypass.rs`.
pub(crate) fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_matches(|ch| ch == '[' || ch == ']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_strict_empty_allows_all_while_strict_empty_denies_all() {
        let open = AllowHostRules::parse("  # comment only\n", false);
        assert!(!open.denies_all());
        assert!(!open.is_restricted());
        assert!(open.allows("api.vicanso.dev"));
        assert!(!open.allows("   "));
        assert_eq!(open.pac_allow_expr(), None);

        let strict = AllowHostRules::parse("  # comment only\n", true);
        assert!(strict.strict());
        assert!(strict.denies_all());
        assert!(strict.is_restricted());
        assert!(!strict.allows("api.vicanso.dev"));
        assert!(!strict.allows("127.0.0.1"));
        assert_eq!(strict.pac_allow_expr().as_deref(), Some("false"));
    }

    #[test]
    fn exact_wildcard_star_and_comments_work_consistently() {
        let text = "# allow api and subdomains\napi.vicanso.dev, *.staging.vicanso.dev; [::1]\n127.0.0.1 # inline";
        let rules = AllowHostRules::parse(text, true);
        assert!(!rules.denies_all());
        assert!(rules.is_restricted());
        assert!(rules.allows("API.VICANSO.DEV."));
        assert!(rules.allows("web.staging.vicanso.dev"));
        assert!(!rules.allows("staging.vicanso.dev"));
        assert!(rules.allows("::1"));
        assert!(rules.allows("[::1]"));
        assert!(rules.allows("127.0.0.1"));
        assert!(!rules.allows("other.vicanso.dev"));

        let star = AllowHostRules::parse("*", true);
        assert!(!star.denies_all());
        assert!(!star.is_restricted());
        assert!(star.allows("any.vicanso.dev"));
        assert_eq!(star.pac_allow_expr(), None);
    }

    #[test]
    fn exclusions_always_win_and_respect_strictness_when_no_positive_rules() {
        let non_strict_exclude =
            AllowHostRules::parse("-admin.vicanso.dev -*.internal.vicanso.dev", false);
        assert!(!non_strict_exclude.denies_all());
        assert!(non_strict_exclude.is_restricted());
        assert!(!non_strict_exclude.allows("admin.vicanso.dev"));
        assert!(!non_strict_exclude.allows("db.internal.vicanso.dev"));
        assert!(non_strict_exclude.allows("api.vicanso.dev"));
        assert_eq!(
            non_strict_exclude.pac_allow_expr().as_deref(),
            Some("!(h === \"admin.vicanso.dev\" || dnsDomainIs(h, \".internal.vicanso.dev\"))")
        );

        let strict_exclude_only = AllowHostRules::parse("-admin.vicanso.dev", true);
        assert!(strict_exclude_only.denies_all());
        assert!(!strict_exclude_only.allows("api.vicanso.dev"));

        let strict_star_minus = AllowHostRules::parse("* -admin.vicanso.dev", true);
        assert!(!strict_star_minus.denies_all());
        assert!(strict_star_minus.is_restricted());
        assert!(!strict_star_minus.allows("admin.vicanso.dev"));
        assert!(strict_star_minus.allows("api.vicanso.dev"));
        assert_eq!(
            strict_star_minus.pac_allow_expr().as_deref(),
            Some("!(h === \"admin.vicanso.dev\")")
        );

        let allow_with_exclude = AllowHostRules::parse("*.vicanso.dev -secret.vicanso.dev", true);
        assert!(allow_with_exclude.allows("api.vicanso.dev"));
        assert!(!allow_with_exclude.allows("secret.vicanso.dev"));
        assert_eq!(
            allow_with_exclude.pac_allow_expr().as_deref(),
            Some("(dnsDomainIs(h, \".vicanso.dev\")) && !(h === \"secret.vicanso.dev\")")
        );

        let deny_all = AllowHostRules::parse("* -*", false);
        assert!(deny_all.denies_all());
        assert!(!deny_all.allows("api.vicanso.dev"));
        assert_eq!(deny_all.pac_allow_expr().as_deref(), Some("false"));
    }

    #[test]
    fn denied_host_stats_counts_sorts_bounds_and_resets() {
        let mut stats = DeniedHostStats::default();
        assert!(!stats.reset());
        assert_eq!(stats.summary(), AllowDeniedSummary::default());

        stats.record("B.Example.COM.");
        stats.record("a.example.com");
        stats.record("b.example.com");
        stats.record("c.example.com");
        stats.record("   ");

        let summary = stats.summary();
        assert_eq!(summary.total, 5);
        assert_eq!(
            summary.hosts,
            vec![
                ("b.example.com".to_string(), 2),
                ("a.example.com".to_string(), 1),
                ("c.example.com".to_string(), 1),
            ]
        );

        // Fill up to DENIED_HOST_CAP and overflow with extra distinct hosts.
        for i in 0..(DENIED_HOST_CAP + 20) {
            stats.record(&format!("h{i:03}.flood.test"));
        }
        // Existing tracked host continues incrementing even when at cap.
        stats.record("b.example.com");

        let capped = stats.summary();
        assert_eq!(capped.hosts.len(), DENIED_HOST_CAP);
        assert_eq!(capped.total, 5 + (DENIED_HOST_CAP as u64 + 20) + 1);
        assert_eq!(capped.hosts[0], ("b.example.com".to_string(), 3));

        assert!(stats.reset());
        assert!(!stats.reset());
        assert_eq!(stats.summary(), AllowDeniedSummary::default());
    }

    #[test]
    fn append_allow_host_normalizes_and_deduplicates() {
        assert_eq!(append_allow_host("", "API.Vicanso.Dev."), "api.vicanso.dev");
        assert_eq!(
            append_allow_host("api.vicanso.dev", "API.VICANSO.DEV"),
            "api.vicanso.dev"
        );
        assert_eq!(
            append_allow_host("api.vicanso.dev", "cdn.vicanso.dev"),
            "api.vicanso.dev\ncdn.vicanso.dev"
        );
        assert_eq!(
            append_allow_host("api.vicanso.dev", "   "),
            "api.vicanso.dev"
        );
    }
}
