//! Hosts that skip TLS interception and stay a TCP tunnel.
//!
//! An empty list intercepts every host. Listed hosts are tunneled.
//! `only:` flips the list into an allow-list: every other host is tunneled.

use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BypassRules {
    only: bool,
    all: bool,
    exact: HashSet<String>,
    /// Suffixes including the leading dot, for `*.example.com`.
    suffixes: Vec<String>,
}

impl Default for BypassRules {
    fn default() -> Self {
        Self::parse("")
    }
}

impl BypassRules {
    pub fn parse(text: &str) -> Self {
        let mut only = false;
        let mut all = false;
        let mut exact = HashSet::new();
        let mut suffixes = Vec::new();
        for raw in text.split(|ch: char| ch.is_whitespace() || ch == ',' || ch == ';' || ch == '|')
        {
            let token = raw.trim();
            if token.is_empty() {
                continue;
            }
            let token = token
                .strip_prefix("only:")
                .map(|rest| {
                    only = true;
                    rest.trim()
                })
                .unwrap_or(token);
            if token.is_empty() || token.eq_ignore_ascii_case("only") {
                only = true;
                continue;
            }
            if token == "*" {
                all = true;
                continue;
            }
            let host = normalize_host(token);
            if let Some(rest) = host.strip_prefix("*.") {
                if !rest.is_empty() {
                    suffixes.push(format!(".{rest}"));
                }
            } else if !host.is_empty() {
                exact.insert(host);
            }
        }
        Self {
            only,
            all,
            exact,
            suffixes,
        }
    }

    /// Whether this host should be tunneled instead of decrypted.
    pub fn skips(&self, host: &str) -> bool {
        let host = normalize_host(host);
        if host.is_empty() {
            return false;
        }
        let listed = self.all
            || self.exact.contains(&host)
            || self.suffixes.iter().any(|suffix| host.ends_with(suffix));
        if self.only { !listed } else { listed }
    }
}

fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_matches(|ch| ch == '[' || ch == ']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_list_intercepts() {
        let rules = BypassRules::parse(" \n");
        assert!(!rules.skips("example.com"));
    }

    #[test]
    fn exact_and_wildcard_tunnel() {
        let rules = BypassRules::parse("Pin.Example.com, *.apple.com");
        assert!(rules.skips("pin.example.com"));
        assert!(rules.skips("api.apple.com"));
        assert!(!rules.skips("apple.com"));
        assert!(!rules.skips("example.com"));
    }

    #[test]
    fn only_tunnels_everyone_else() {
        let rules = BypassRules::parse("only: api.example.com *.internal.test");
        assert!(!rules.skips("api.example.com"));
        assert!(!rules.skips("a.internal.test"));
        assert!(rules.skips("other.example.com"));
    }

    #[test]
    fn star_tunnels_all_unless_only() {
        assert!(BypassRules::parse("*").skips("anywhere.test"));
        assert!(!BypassRules::parse("only *").skips("anywhere.test"));
    }
}
