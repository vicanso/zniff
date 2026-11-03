//! Map Local, Map Remote, and breakpoints.
//!
//! One rule per line:
//! `map-local example.com/api ./body.json`
//! `map-remote example.com/api https://other.test/v1`
//! `break example.com/pay`

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    MapLocal {
        host: String,
        path_prefix: String,
        file: PathBuf,
    },
    MapRemote {
        host: String,
        path_prefix: String,
        url: String,
    },
    Break {
        host: String,
        path_prefix: String,
    },
}

impl Rule {
    pub fn matches(&self, host: &str, path: &str) -> bool {
        let (rule_host, prefix) = match self {
            Self::MapLocal {
                host, path_prefix, ..
            }
            | Self::MapRemote {
                host, path_prefix, ..
            }
            | Self::Break {
                host, path_prefix, ..
            } => (host, path_prefix),
        };
        host_matches(rule_host, host) && path_matches(prefix, path)
    }
}

pub fn parse_rules(text: &str) -> Vec<Rule> {
    text.lines()
        .filter_map(|line| {
            let line = line.split('#').next().unwrap_or(line).trim();
            if line.is_empty() {
                return None;
            }
            parse_line(line)
        })
        .collect()
}

pub fn first_match<'a>(rules: &'a [Rule], host: &str, path: &str) -> Option<&'a Rule> {
    rules.iter().find(|rule| rule.matches(host, path))
}

fn parse_line(line: &str) -> Option<Rule> {
    let mut parts = line.split_whitespace();
    let kind = parts.next()?;
    let pattern = parts.next()?;
    let (host, path_prefix) = split_pattern(pattern);
    match kind {
        "map-local" => {
            let file = parts.next()?;
            Some(Rule::MapLocal {
                host,
                path_prefix,
                file: PathBuf::from(file),
            })
        }
        "map-remote" => {
            let url = parts.next()?.to_string();
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return None;
            }
            Some(Rule::MapRemote {
                host,
                path_prefix,
                url,
            })
        }
        "break" => Some(Rule::Break { host, path_prefix }),
        _ => None,
    }
}

fn split_pattern(pattern: &str) -> (String, String) {
    let pattern = pattern.trim();
    if let Some((host, path)) = pattern.split_once('/') {
        let path = if path.is_empty() {
            "/".to_string()
        } else {
            format!("/{path}")
        };
        (normalize_host(host), path)
    } else {
        (normalize_host(pattern), "/".to_string())
    }
}

fn host_matches(rule: &str, host: &str) -> bool {
    let host = normalize_host(host);
    if let Some(rest) = rule.strip_prefix("*.") {
        host.ends_with(&format!(".{rest}"))
    } else {
        rule == host
    }
}

fn path_matches(prefix: &str, path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    if prefix == "/" {
        return true;
    }
    path == prefix || path.starts_with(&format!("{prefix}/"))
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
    fn parses_and_matches_the_first_rule() {
        let rules = parse_rules(
            "\n# comment\nmap-local example.com/api ./a.json\nmap-remote *.other.test/v1 https://remote.test/x\nbreak pay.test/checkout\n",
        );
        assert!(matches!(rules[0], Rule::MapLocal { .. }));
        assert!(rules[0].matches("Example.com", "/api/v2"));
        assert!(!rules[0].matches("example.com", "/other"));
        assert!(rules[1].matches("a.other.test", "/v1"));
        assert!(!rules[1].matches("other.test", "/v1"));
        assert!(rules[2].matches("pay.test", "/checkout/1"));
        assert!(first_match(&rules, "example.com", "/api").is_some());
    }
}
