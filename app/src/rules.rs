//! Map Local, Map Remote, breakpoints, and non-terminal tweaks.
//!
//! One rule per line. The first matching terminal rule wins. `delay`,
//! `header`, and `cors` do not consume the match: every matching one applies,
//! including lines after the terminal rule.
//!
//! ```text
//! map-local example.com/api ./body.json status=429 delay=2000ms
//! map-remote example.com/api https://other.test/v1
//! break example.com/pay
//! break-res example.com/pay
//! delay example.com/api 2000ms
//! header example.com/api res set X-Debug 1
//! header example.com/api req del Cookie
//! cors example.com
//! replace example.com/api res hello -> hi there
//! ```
//!
//! `replace` changes UTF-8 text in the body and leaves the rest of the bytes.
//! `from` is one token. `to` may contain spaces. It is not a terminal rule.
//!
//! `delay` is capped at 30s. A response break buffers at most 1 MB.

use std::path::PathBuf;
use std::time::Duration;

pub const DELAY_CAP: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    MapLocal {
        host: String,
        path_prefix: String,
        file: PathBuf,
        status: Option<u16>,
        delay: Duration,
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
    BreakResponse {
        host: String,
        path_prefix: String,
    },
    Delay {
        host: String,
        path_prefix: String,
        delay: Duration,
    },
    Header {
        host: String,
        path_prefix: String,
        response: bool,
        remove: bool,
        name: String,
        value: String,
    },
    Cors {
        host: String,
        path_prefix: String,
    },
    /// Substitute text in the request or response body. Not a terminal rule.
    Replace {
        host: String,
        path_prefix: String,
        response: bool,
        from: String,
        to: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderOp {
    Set { name: String, value: String },
    Remove { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    MapLocal { file: PathBuf, status: u16 },
    MapRemote { url: String },
    BreakRequest,
    BreakResponse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effect {
    pub delay: Duration,
    pub request_headers: Vec<HeaderOp>,
    pub response_headers: Vec<HeaderOp>,
    pub cors: bool,
    pub action: Option<Action>,
    /// `(from, to)` applied in order to a UTF-8 body that fits in one flow.
    pub request_replaces: Vec<(String, String)>,
    pub response_replaces: Vec<(String, String)>,
}

impl Default for Effect {
    fn default() -> Self {
        Self {
            delay: Duration::ZERO,
            request_headers: Vec::new(),
            response_headers: Vec::new(),
            cors: false,
            action: None,
            request_replaces: Vec::new(),
            response_replaces: Vec::new(),
        }
    }
}

impl Rule {
    pub fn matches(&self, host: &str, path: &str) -> bool {
        let (rule_host, prefix) = self.target();
        host_matches(rule_host, host) && path_matches(prefix, path)
    }

    fn target(&self) -> (&str, &str) {
        match self {
            Self::MapLocal {
                host, path_prefix, ..
            }
            | Self::MapRemote {
                host, path_prefix, ..
            }
            | Self::Break {
                host, path_prefix, ..
            }
            | Self::BreakResponse {
                host, path_prefix, ..
            }
            | Self::Delay {
                host, path_prefix, ..
            }
            | Self::Header {
                host, path_prefix, ..
            }
            | Self::Cors {
                host, path_prefix, ..
            }
            | Self::Replace {
                host, path_prefix, ..
            } => (host, path_prefix),
        }
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

#[cfg(test)]
pub fn first_match<'a>(rules: &'a [Rule], host: &str, path: &str) -> Option<&'a Rule> {
    rules.iter().find(|rule| rule.matches(host, path))
}

/// Modifiers accumulate. The first matching terminal rule is the action.
pub fn effect(rules: &[Rule], host: &str, path: &str) -> Effect {
    let mut effect = Effect::default();
    for rule in rules {
        if !rule.matches(host, path) {
            continue;
        }
        match rule {
            Rule::Delay { delay, .. } => {
                effect.delay = cap_delay(effect.delay.saturating_add(*delay));
            }
            Rule::Header {
                response,
                remove,
                name,
                value,
                ..
            } => {
                let op = if *remove {
                    HeaderOp::Remove { name: name.clone() }
                } else {
                    HeaderOp::Set {
                        name: name.clone(),
                        value: value.clone(),
                    }
                };
                if *response {
                    effect.response_headers.push(op);
                } else {
                    effect.request_headers.push(op);
                }
            }
            Rule::Cors { .. } => effect.cors = true,
            Rule::Replace {
                response,
                from,
                to,
                ..
            } => {
                let pair = (from.clone(), to.clone());
                if *response {
                    effect.response_replaces.push(pair);
                } else {
                    effect.request_replaces.push(pair);
                }
            }
            Rule::MapLocal {
                file,
                status,
                delay,
                ..
            } if effect.action.is_none() => {
                effect.delay = cap_delay(effect.delay.saturating_add(*delay));
                effect.action = Some(Action::MapLocal {
                    file: file.clone(),
                    status: status.unwrap_or(200),
                });
            }
            Rule::MapRemote { url, .. } if effect.action.is_none() => {
                effect.action = Some(Action::MapRemote { url: url.clone() });
            }
            Rule::Break { .. } if effect.action.is_none() => {
                effect.action = Some(Action::BreakRequest);
            }
            Rule::BreakResponse { .. } if effect.action.is_none() => {
                effect.action = Some(Action::BreakResponse);
            }
            _ => {}
        }
    }
    effect
}

pub fn apply_header_ops(headers: &mut Vec<(String, String)>, ops: &[HeaderOp]) {
    for op in ops {
        match op {
            HeaderOp::Remove { name } => {
                headers.retain(|(existing, _)| !existing.eq_ignore_ascii_case(name));
            }
            HeaderOp::Set { name, value } => {
                headers.retain(|(existing, _)| !existing.eq_ignore_ascii_case(name));
                headers.push((name.clone(), value.clone()));
            }
        }
    }
}

pub fn cors_pairs() -> Vec<(String, String)> {
    vec![
        ("Access-Control-Allow-Origin".into(), "*".into()),
        (
            "Access-Control-Allow-Methods".into(),
            "GET, POST, PUT, PATCH, DELETE, OPTIONS, HEAD".into(),
        ),
        ("Access-Control-Allow-Headers".into(), "*".into()),
        ("Access-Control-Max-Age".into(), "86400".into()),
    ]
}

pub fn ensure_cors(headers: &mut Vec<(String, String)>) {
    for (name, value) in cors_pairs() {
        if headers
            .iter()
            .any(|(existing, _)| existing.eq_ignore_ascii_case(&name))
        {
            continue;
        }
        headers.push((name, value));
    }
}

fn cap_delay(delay: Duration) -> Duration {
    if delay > DELAY_CAP { DELAY_CAP } else { delay }
}

fn parse_line(line: &str) -> Option<Rule> {
    let mut parts = line.split_whitespace();
    let kind = parts.next()?;
    let pattern = parts.next()?;
    let (host, path_prefix) = split_pattern(pattern);
    match kind {
        "map-local" => {
            let file = parts.next()?;
            let mut status = None;
            let mut delay = Duration::ZERO;
            for token in parts {
                if let Some(code) = token.strip_prefix("status=") {
                    let code: u16 = code.parse().ok()?;
                    if !(100..600).contains(&code) {
                        return None;
                    }
                    status = Some(code);
                } else if let Some(text) = token.strip_prefix("delay=") {
                    delay = parse_delay(text)?;
                } else {
                    return None;
                }
            }
            Some(Rule::MapLocal {
                host,
                path_prefix,
                file: PathBuf::from(file),
                status,
                delay,
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
        "break-res" | "break-response" => Some(Rule::BreakResponse { host, path_prefix }),
        "delay" => {
            let text = parts.next()?;
            Some(Rule::Delay {
                host,
                path_prefix,
                delay: parse_delay(text)?,
            })
        }
        "header" => {
            let side = parts.next()?;
            let op = parts.next()?;
            let name = parts.next()?.to_string();
            if name.is_empty() {
                return None;
            }
            let response = match side {
                "res" | "response" => true,
                "req" | "request" => false,
                _ => return None,
            };
            let remove = match op {
                "del" | "delete" | "rm" => true,
                "set" | "add" => false,
                _ => return None,
            };
            let value = parts.collect::<Vec<_>>().join(" ");
            Some(Rule::Header {
                host,
                path_prefix,
                response,
                remove,
                name,
                value,
            })
        }
        "cors" => Some(Rule::Cors { host, path_prefix }),
        "replace" => {
            let side = parts.next()?;
            let rest = parts.collect::<Vec<_>>().join(" ");
            let (from, to) = rest.split_once(" -> ")?;
            if from.is_empty() {
                return None;
            }
            let response = match side {
                "res" | "response" => true,
                "req" | "request" => false,
                _ => return None,
            };
            Some(Rule::Replace {
                host,
                path_prefix,
                response,
                from: from.to_string(),
                to: to.to_string(),
            })
        }
        _ => None,
    }
}

/// Replaces each pair in order. `None` when the body is not UTF-8, `from` is
/// empty, or nothing changed. Callers keep the original bytes in that case.
pub fn apply_replaces(body: &[u8], pairs: &[(String, String)]) -> Option<Vec<u8>> {
    if pairs.is_empty() {
        return None;
    }
    let mut text = std::str::from_utf8(body).ok()?.to_string();
    let mut changed = false;
    for (from, to) in pairs {
        if from.is_empty() || !text.contains(from.as_str()) {
            continue;
        }
        text = text.replace(from, to);
        changed = true;
    }
    changed.then(|| text.into_bytes())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QuickRule {
    Replace,
    Remote,
    Local,
    Delay,
    Break,
    BreakRes,
}

impl QuickRule {
    pub fn label(self) -> (&'static str, &'static str) {
        match self {
            Self::Replace => ("替换", "Replace"),
            Self::Remote => ("转发", "Forward"),
            Self::Local => ("文件", "File"),
            Self::Delay => ("延迟", "Delay"),
            Self::Break => ("断点", "Break"),
            Self::BreakRes => ("响应断点", "Break res"),
        }
    }
}

/// `host` plus the path, without the query. Both must be one token.
pub fn request_pattern(host: &str, path: &str) -> Option<String> {
    let host = host.trim();
    let path = path.split('?').next().unwrap_or(path).trim();
    if host.is_empty()
        || host.contains(char::is_whitespace)
        || path.contains(char::is_whitespace)
    {
        return None;
    }
    let path = if path.is_empty() || !path.starts_with('/') {
        return None;
    } else {
        path
    };
    Some(format!("{host}{path}"))
}

pub fn append_break_line(rules: &str, host: &str, path: &str) -> Option<String> {
    let pattern = request_pattern(host, path)?;
    push_line(rules, &format!("break {pattern}"))
}

pub fn append_quick_rule(
    rules: &str,
    kind: QuickRule,
    pattern: &str,
    response: bool,
    from: &str,
    to: &str,
) -> Option<String> {
    let pattern = pattern.trim();
    if pattern.is_empty() || pattern.contains(char::is_whitespace) {
        return None;
    }
    let from = from.trim();
    let to = to.trim();
    let line = match kind {
        QuickRule::Replace => {
            return append_replace_line(rules, pattern, response, from, to);
        }
        QuickRule::Remote => {
            if !(from.starts_with("http://") || from.starts_with("https://")) {
                return None;
            }
            format!("map-remote {pattern} {from}")
        }
        QuickRule::Local => {
            if from.is_empty() || from.contains(char::is_whitespace) {
                return None;
            }
            format!("map-local {pattern} {from}")
        }
        QuickRule::Delay => {
            if from.is_empty() || from.contains(char::is_whitespace) {
                return None;
            }
            format!("delay {pattern} {from}")
        }
        QuickRule::Break => format!("break {pattern}"),
        QuickRule::BreakRes => format!("break-res {pattern}"),
    };
    push_line(rules, &line)
}

fn push_line(rules: &str, line: &str) -> Option<String> {
    if line.contains('\n') {
        return None;
    }
    let mut next = rules.to_string();
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(line);
    next.push('\n');
    Some(next)
}

/// One line for the rules box. `from` cannot contain spaces or `->`.
pub fn append_replace_line(
    rules: &str,
    pattern: &str,
    response: bool,
    from: &str,
    to: &str,
) -> Option<String> {
    let pattern = pattern.trim();
    let from = from.trim();
    let to = to.trim();
    if pattern.is_empty()
        || from.is_empty()
        || pattern.contains(char::is_whitespace)
        || from.contains(char::is_whitespace)
        || from.contains("->")
        || pattern.contains('\n')
        || to.contains('\n')
    {
        return None;
    }
    let side = if response { "res" } else { "req" };
    let line = format!("replace {pattern} {side} {from} -> {to}");
    let mut next = rules.to_string();
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(&line);
    next.push('\n');
    Some(next)
}

fn parse_delay(text: &str) -> Option<Duration> {
    let text = text.trim();
    let text = text.strip_prefix("delay=").unwrap_or(text);
    let (num, unit) = if let Some(num) = text.strip_suffix("ms") {
        (num, 1u64)
    } else if let Some(num) = text.strip_suffix('s') {
        (num, 1000)
    } else {
        (text, 1)
    };
    if num.is_empty() || !num.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let ms = num.parse::<u64>().ok()?.saturating_mul(unit);
    Some(Duration::from_millis(ms.min(DELAY_CAP.as_millis() as u64)))
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

    #[test]
    fn modifiers_apply_around_the_first_terminal_rule() {
        let rules = parse_rules(
            "\
delay example.com/api 1500ms
header example.com req set X-From zniff
cors example.com
map-local example.com/api ./a.json status=429 delay=1s
header example.com res del Set-Cookie
break example.com/api
",
        );
        let applied = effect(&rules, "example.com", "/api/v1");
        assert_eq!(applied.delay, Duration::from_millis(2500));
        assert!(applied.cors);
        assert_eq!(
            applied.action,
            Some(Action::MapLocal {
                file: PathBuf::from("./a.json"),
                status: 429,
            })
        );
        assert_eq!(applied.request_headers.len(), 1);
        assert!(matches!(
            &applied.response_headers[0],
            HeaderOp::Remove { name } if name == "Set-Cookie"
        ));
        let capped = effect(
            &parse_rules("delay example.com 40s"),
            "example.com",
            "/",
        );
        assert_eq!(capped.delay, DELAY_CAP);
    }

    #[test]
    fn header_ops_replace_and_remove() {
        let mut headers = vec![
            ("X-From".into(), "old".into()),
            ("Cookie".into(), "a=b".into()),
        ];
        apply_header_ops(
            &mut headers,
            &[
                HeaderOp::Set {
                    name: "X-From".into(),
                    value: "zniff".into(),
                },
                HeaderOp::Remove {
                    name: "Cookie".into(),
                },
            ],
        );
        assert_eq!(headers, vec![("X-From".into(), "zniff".into())]);
    }

    #[test]
    fn replace_keeps_the_rest_of_a_utf8_body() {
        let rules = parse_rules(
            "replace example.com/api res hello -> hi there\nreplace example.com/api req token -> secret\n",
        );
        let applied = effect(&rules, "example.com", "/api");
        assert!(applied.action.is_none());
        assert_eq!(
            applied.response_replaces,
            vec![("hello".into(), "hi there".into())]
        );
        assert_eq!(
            apply_replaces(b"say hello", &applied.response_replaces).as_deref(),
            Some(&b"say hi there"[..])
        );
        assert!(apply_replaces(b"untouched", &applied.response_replaces).is_none());
        assert!(apply_replaces(&[0xff, 0xfe], &applied.response_replaces).is_none());
        let line = append_replace_line("", "example.com/api", true, "hello", "hi there").unwrap();
        assert_eq!(line, "replace example.com/api res hello -> hi there\n");
        assert!(append_replace_line("", "has space", true, "a", "b").is_none());
        assert!(append_replace_line("", "example.com", true, "a -> b", "c").is_none());
        let broken = append_quick_rule("", QuickRule::Remote, "example.com/api", true, "not-a-url", "");
        assert!(broken.is_none());
        let forwarded = append_quick_rule(
            "",
            QuickRule::Remote,
            "example.com/api",
            true,
            "https://other.test/v1",
            "",
        )
        .unwrap();
        assert_eq!(forwarded, "map-remote example.com/api https://other.test/v1\n");
        let stopped = append_break_line("", "example.com", "/pay?x=1").unwrap();
        assert_eq!(stopped, "break example.com/pay\n");
        assert!(append_break_line("", "has space", "/pay").is_none());
    }
}
