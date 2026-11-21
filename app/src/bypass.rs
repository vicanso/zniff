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

    /// Builds a dynamic `proxy.pac` script reflecting host allow-list, [`Self::skips`], and loopback bypasses.
    pub fn pac_script_with_allow_hosts(
        &self,
        proxy_authority: &str,
        allow: &crate::allow_hosts::AllowHostRules,
    ) -> String {
        let authority = sanitize_pac_authority(proxy_authority);
        let proxy_literal = serde_json::to_string(&format!("PROXY {authority}; DIRECT"))
            .unwrap_or_else(|_| "\"PROXY 127.0.0.1:8877; DIRECT\"".to_string());

        let decision_body = if allow.denies_all() {
            "  return \"DIRECT\";".to_string()
        } else {
            let allow_guard = allow
                .pac_allow_expr()
                .map(|expr| format!("  if (!({expr})) {{\n    return \"DIRECT\";\n  }}\n"))
                .unwrap_or_default();
            let bypass_body = if self.all {
                let result = if self.only {
                    proxy_literal.as_str()
                } else {
                    "\"DIRECT\""
                };
                format!("  return {result};")
            } else {
                let match_expr = self.pac_match_expr();
                if self.only {
                    format!(
                        "  if ({match_expr}) {{\n    return {proxy_literal};\n  }}\n  return \"DIRECT\";"
                    )
                } else {
                    format!(
                        "  if ({match_expr}) {{\n    return \"DIRECT\";\n  }}\n  return {proxy_literal};"
                    )
                }
            };
            format!("{allow_guard}{bypass_body}")
        };

        format!(
            "function FindProxyForURL(url, host) {{\n  var h = String(host || \"\").toLowerCase().replace(/^\\[|\\]$/g, \"\").replace(/\\.+$/, \"\");\n  if (!h || isPlainHostName(h) || h === \"localhost\" || h === \"127.0.0.1\" || h === \"::1\") {{\n    return \"DIRECT\";\n  }}\n{decision_body}\n}}\n"
        )
    }

    fn pac_match_expr(&self) -> String {
        let mut checks = Vec::new();
        let mut exact: Vec<&str> = self.exact.iter().map(String::as_str).collect();
        exact.sort_unstable();
        for host in exact {
            if let Ok(lit) = serde_json::to_string(host) {
                checks.push(format!("h === {lit}"));
            }
        }
        let mut suffixes: Vec<&str> = self.suffixes.iter().map(String::as_str).collect();
        suffixes.sort_unstable();
        for suffix in suffixes {
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
}

fn sanitize_pac_authority(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|ch| {
            !ch.is_control() && !ch.is_whitespace() && !matches!(ch, '"' | '\'' | '\\' | ';' | '/')
        })
        .collect();
    if cleaned.is_empty() {
        "127.0.0.1:8877".to_string()
    } else {
        cleaned
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

    #[test]
    fn pac_script_reflects_bypass_and_only_modes_and_sanitizes_authority() {
        let unrestricted = crate::allow_hosts::AllowHostRules::parse("", false);
        let normal = BypassRules::parse("exact.vicanso.dev, *.corp.vicanso.dev");
        let pac = normal.pac_script_with_allow_hosts("127.0.0.1:8877\"; alert(1)//", &unrestricted);
        assert!(pac.contains("function FindProxyForURL(url, host)"), "{pac}");
        assert!(pac.contains("isPlainHostName(h)"), "{pac}");
        assert!(
            pac.contains("PROXY 127.0.0.1:8877alert(1); DIRECT"),
            "{pac}"
        );
        assert!(pac.contains("h === \"exact.vicanso.dev\""), "{pac}");
        assert!(
            pac.contains("dnsDomainIs(h, \".corp.vicanso.dev\")"),
            "{pac}"
        );
        assert!(!pac.contains("\"; alert"), "{pac}");

        let only = BypassRules::parse("only: api.vicanso.dev *.staging.vicanso.dev");
        let only_pac = only.pac_script_with_allow_hosts("10.0.0.2:8877", &unrestricted);
        assert!(only_pac.contains("h === \"api.vicanso.dev\""), "{only_pac}");
        assert!(
            only_pac.contains("dnsDomainIs(h, \".staging.vicanso.dev\")\n  ) {\n")
                || only_pac
                    .contains("return \"PROXY 10.0.0.2:8877; DIRECT\";\n  }\n  return \"DIRECT\";"),
            "{only_pac}"
        );

        let deny_all = crate::allow_hosts::AllowHostRules::parse("", true);
        let strict_pac = normal.pac_script_with_allow_hosts("10.0.0.2:8877", &deny_all);
        assert!(strict_pac.contains("return \"DIRECT\";\n}"), "{strict_pac}");

        let allowed = crate::allow_hosts::AllowHostRules::parse("*.vicanso.dev", true);
        let allowed_pac = normal.pac_script_with_allow_hosts("10.0.0.2:8877", &allowed);
        assert!(
            allowed_pac.contains(
                "if (!(dnsDomainIs(h, \".vicanso.dev\"))) {\n    return \"DIRECT\";\n  }"
            ),
            "{allowed_pac}"
        );
    }
}
