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
//! `replace` changes UTF-8 text. `res` changes the response body. `req`
//! changes the request target (`/path?query`) and the request body.
//! `from` is one token. `to` may contain spaces. It is not a terminal rule.
//! A request-target replacement that does not still start with `/` is ignored.
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
        file: Option<PathBuf>,
        status: Option<u16>,
        delay: Duration,
        op: Option<String>,
        fallback: bool,
    },
    MapRemote {
        host: String,
        path_prefix: String,
        url: String,
        op: Option<String>,
    },
    Break {
        host: String,
        path_prefix: String,
        op: Option<String>,
    },
    BreakResponse {
        host: String,
        path_prefix: String,
        op: Option<String>,
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
    Query {
        host: String,
        path_prefix: String,
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
    /// Set or remove a JSON property/element by JSON Pointer (`/a/b/0`) or dot/bracket path (`a.b[0]`).
    Json {
        host: String,
        path_prefix: String,
        response: bool,
        remove: bool,
        json_path: String,
        value: serde_json::Value,
    },
    /// Bandwidth limit in bytes per second for download (`down_bps`) and/or upload (`up_bps`).
    Throttle {
        host: String,
        path_prefix: String,
        down_bps: Option<u64>,
        up_bps: Option<u64>,
    },
    /// Custom TLS certificate chain and private key for MITM handshakes on `host`.
    /// When `key_path` is `None`, `cert_path` is a combined PEM containing both
    /// certificate(s) and the private key.
    Cert {
        host: String,
        cert_path: PathBuf,
        key_path: Option<PathBuf>,
    },
    /// Client certificate chain and private key for upstream mutual TLS (mTLS) on `host`.
    ClientCert {
        host: String,
        cert_path: PathBuf,
        key_path: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderOp {
    Set { name: String, value: String },
    Remove { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryOp {
    Set { name: String, value: String },
    Remove { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonOp {
    Set {
        path: String,
        value: serde_json::Value,
    },
    Remove {
        path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    MapLocal {
        path_prefix: String,
        file: Option<PathBuf>,
        status: u16,
        fallback: bool,
    },
    MapRemote {
        url: String,
    },
    BreakRequest,
    BreakResponse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effect {
    pub delay: Duration,
    pub throttle_down_bps: Option<u64>,
    pub throttle_up_bps: Option<u64>,
    pub request_headers: Vec<HeaderOp>,
    pub response_headers: Vec<HeaderOp>,
    pub query_ops: Vec<QueryOp>,
    pub cors: bool,
    pub action: Option<Action>,
    /// `(from, to)` applied in order to a UTF-8 body that fits in one flow.
    pub request_replaces: Vec<(String, String)>,
    pub response_replaces: Vec<(String, String)>,
    /// Structured JSON path set/remove operations applied to JSON request/response bodies.
    pub request_json_ops: Vec<JsonOp>,
    pub response_json_ops: Vec<JsonOp>,
}

impl Default for Effect {
    fn default() -> Self {
        Self {
            delay: Duration::ZERO,
            throttle_down_bps: None,
            throttle_up_bps: None,
            request_headers: Vec::new(),
            response_headers: Vec::new(),
            query_ops: Vec::new(),
            cors: false,
            action: None,
            request_replaces: Vec::new(),
            response_replaces: Vec::new(),
            request_json_ops: Vec::new(),
            response_json_ops: Vec::new(),
        }
    }
}

impl Rule {
    #[cfg(test)]
    pub fn matches(&self, host: &str, path: &str) -> bool {
        self.matches_with_ops(host, path, &[])
    }

    pub fn matches_with_ops(&self, host: &str, path: &str, ops: &[String]) -> bool {
        let (rule_host, prefix) = self.target();
        if !host_matches(rule_host, host) || !path_matches(prefix, path) {
            return false;
        }
        match self.op() {
            Some(op_pattern) => op_matches(op_pattern, ops),
            None => true,
        }
    }

    pub fn op(&self) -> Option<&str> {
        match self {
            Self::MapLocal { op, .. }
            | Self::MapRemote { op, .. }
            | Self::Break { op, .. }
            | Self::BreakResponse { op, .. } => op.as_deref(),
            _ => None,
        }
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
            | Self::Query {
                host, path_prefix, ..
            }
            | Self::Cors {
                host, path_prefix, ..
            }
            | Self::Replace {
                host, path_prefix, ..
            }
            | Self::Json {
                host, path_prefix, ..
            }
            | Self::Throttle {
                host, path_prefix, ..
            } => (host, path_prefix),
            Self::Cert { host, .. } | Self::ClientCert { host, .. } => (host, "/"),
        }
    }
}

/// First matching `cert` rule for `host`, returning `(cert_path, key_path)`.
pub fn custom_cert(rules: &[Rule], host: &str) -> Option<(PathBuf, Option<PathBuf>)> {
    rules.iter().find_map(|rule| match rule {
        Rule::Cert {
            host: rule_host,
            cert_path,
            key_path,
        } if host_matches(rule_host, host) => Some((cert_path.clone(), key_path.clone())),
        _ => None,
    })
}

/// First matching `client-cert` rule for `host`, returning `(cert_path, key_path)`.
pub fn custom_client_cert(rules: &[Rule], host: &str) -> Option<(PathBuf, Option<PathBuf>)> {
    rules.iter().find_map(|rule| match rule {
        Rule::ClientCert {
            host: rule_host,
            cert_path,
            key_path,
        } if host_matches(rule_host, host) => Some((cert_path.clone(), key_path.clone())),
        _ => None,
    })
}

pub fn parse_rules(text: &str) -> Vec<Rule> {
    text.lines()
        .filter_map(|line| {
            let line = strip_rule_comment(line).trim();
            if line.is_empty() {
                return None;
            }
            parse_line(line)
        })
        .collect()
}

/// Returns true when at least one rule matching `(host, path)` by host and path
/// also carries a GraphQL `op=<name>` condition.
pub fn has_op_rules(rules: &[Rule], host: &str, path: &str) -> bool {
    rules.iter().any(|rule| {
        if rule.op().is_none() {
            return false;
        }
        let (rule_host, prefix) = rule.target();
        host_matches(rule_host, host) && path_matches(prefix, path)
    })
}

/// Labels of the rules that actually apply, in text order. The first terminal
/// rule stops later terminal rules. Modifiers still count after it.
#[cfg(test)]
pub fn hit_summary(rules: &[Rule], host: &str, path: &str) -> String {
    hit_summary_with_ops(rules, host, path, &[])
}

pub fn hit_summary_with_ops(rules: &[Rule], host: &str, path: &str, ops: &[String]) -> String {
    let mut labels = Vec::new();
    let mut took_terminal = false;
    for rule in rules {
        if !rule.matches_with_ops(host, path, ops) {
            continue;
        }
        let (label, terminal) = match rule {
            Rule::MapLocal { .. } => ("map-local", true),
            Rule::MapRemote { .. } => ("map-remote", true),
            Rule::Break { .. } => ("break", true),
            Rule::BreakResponse { .. } => ("break-res", true),
            Rule::Delay { .. } => ("delay", false),
            Rule::Throttle { .. } => ("throttle", false),
            Rule::Cert { .. } => ("cert", false),
            Rule::ClientCert { .. } => ("client-cert", false),
            Rule::Header { .. } => ("header", false),
            Rule::Query { .. } => ("query", false),
            Rule::Cors { .. } => ("cors", false),
            Rule::Replace { .. } => ("replace", false),
            Rule::Json { .. } => ("json", false),
        };
        if terminal {
            if took_terminal {
                continue;
            }
            took_terminal = true;
        }
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    labels.join(" ")
}

/// Drops a request or response pause. Delay, headers, CORS, replace, and map stay.
pub fn without_breaks(mut effect: Effect) -> Effect {
    if matches!(
        effect.action,
        Some(Action::BreakRequest | Action::BreakResponse)
    ) {
        effect.action = None;
    }
    effect
}

/// `hit_summary` with the pause labels removed.
pub fn without_break_labels(summary: &str) -> String {
    summary
        .split_whitespace()
        .filter(|label| *label != "break" && *label != "break-res")
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
pub fn first_match<'a>(rules: &'a [Rule], host: &str, path: &str) -> Option<&'a Rule> {
    rules.iter().find(|rule| rule.matches(host, path))
}

/// Modifiers accumulate. The first matching terminal rule is the action.
#[cfg(test)]
pub fn effect(rules: &[Rule], host: &str, path: &str) -> Effect {
    effect_with_ops(rules, host, path, &[])
}

pub fn effect_with_ops(rules: &[Rule], host: &str, path: &str, ops: &[String]) -> Effect {
    let mut effect = Effect::default();
    for rule in rules {
        if !rule.matches_with_ops(host, path, ops) {
            continue;
        }
        match rule {
            Rule::Delay { delay, .. } => {
                effect.delay = cap_delay(effect.delay.saturating_add(*delay));
            }
            Rule::Throttle {
                down_bps, up_bps, ..
            } => {
                if let Some(down) = down_bps {
                    effect.throttle_down_bps =
                        Some(effect.throttle_down_bps.map_or(*down, |cur| cur.min(*down)));
                }
                if let Some(up) = up_bps {
                    effect.throttle_up_bps =
                        Some(effect.throttle_up_bps.map_or(*up, |cur| cur.min(*up)));
                }
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
            Rule::Query {
                remove,
                name,
                value,
                ..
            } => {
                let op = if *remove {
                    QueryOp::Remove { name: name.clone() }
                } else {
                    QueryOp::Set {
                        name: name.clone(),
                        value: value.clone(),
                    }
                };
                effect.query_ops.push(op);
            }
            Rule::Cors { .. } => effect.cors = true,
            Rule::Replace {
                response, from, to, ..
            } => {
                let pair = (from.clone(), to.clone());
                if *response {
                    effect.response_replaces.push(pair);
                } else {
                    effect.request_replaces.push(pair);
                }
            }
            Rule::Json {
                response,
                remove,
                json_path,
                value,
                ..
            } => {
                let op = if *remove {
                    JsonOp::Remove {
                        path: json_path.clone(),
                    }
                } else {
                    JsonOp::Set {
                        path: json_path.clone(),
                        value: value.clone(),
                    }
                };
                if *response {
                    effect.response_json_ops.push(op);
                } else {
                    effect.request_json_ops.push(op);
                }
            }
            Rule::MapLocal {
                path_prefix,
                file,
                status,
                delay,
                fallback,
                ..
            } if effect.action.is_none() => {
                effect.delay = cap_delay(effect.delay.saturating_add(*delay));
                effect.action = Some(Action::MapLocal {
                    path_prefix: path_prefix.clone(),
                    file: file.clone(),
                    status: status.unwrap_or(200),
                    fallback: *fallback,
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

/// Strips a trailing `# ...` comment when `#` is outside `"..."` quotes.
fn strip_rule_comment(line: &str) -> &str {
    let mut in_quotes = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_quotes {
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_quotes = false;
            }
            continue;
        }
        if ch == '"' {
            in_quotes = true;
        } else if ch == '#' {
            return &line[..idx];
        }
    }
    line
}

/// Quotes a rule token with `"..."` when it contains whitespace, quotes, or backslashes.
pub fn quote_rule_token(text: &str) -> String {
    if text.is_empty()
        || text.chars().any(char::is_whitespace)
        || text.contains('"')
        || text.contains('\\')
    {
        let mut out = String::with_capacity(text.len() + 2);
        out.push('"');
        for ch in text.chars() {
            match ch {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                _ => out.push(ch),
            }
        }
        out.push('"');
        out
    } else {
        text.to_string()
    }
}

/// Unquotes an optional `"..."` wrapper, or returns the trimmed text when unquoted.
/// Returns `None` if the text starts with `"` but has an unclosed quote or trailing characters.
fn unquote_optional(text: &str) -> Option<String> {
    let text = text.trim();
    if !text.starts_with('"') {
        return Some(text.to_string());
    }
    let (token, rest) = next_rule_token(text)?;
    if !rest.trim().is_empty() {
        return None;
    }
    Some(token)
}

fn next_rule_token(input: &str) -> Option<(String, &str)> {
    let trimmed = input.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(after_open) = trimmed.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = after_open.char_indices();
        while let Some((idx, ch)) = chars.next() {
            match ch {
                '"' => {
                    let rest = &after_open[idx + 1..];
                    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
                        return None;
                    }
                    return Some((out, rest));
                }
                '\\' => {
                    let (_, esc) = chars.next()?;
                    match esc {
                        '"' | '\\' => out.push(esc),
                        other => {
                            out.push('\\');
                            out.push(other);
                        }
                    }
                }
                other => out.push(other),
            }
        }
        None
    } else {
        let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
        Some((trimmed[..end].to_string(), &trimmed[end..]))
    }
}

fn rule_tokens(line: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut rest = line.trim();
    while !rest.is_empty() {
        let (token, next) = next_rule_token(rest)?;
        tokens.push(token);
        rest = next.trim_start();
    }
    Some(tokens)
}

fn parse_status_opt(text: &str) -> Option<u16> {
    let raw = text.trim().strip_prefix("status=").unwrap_or(text.trim());
    let code: u16 = raw.parse().ok()?;
    (100..600).contains(&code).then_some(code)
}

fn parse_op_opt(text: &str) -> Option<String> {
    let raw = text
        .trim()
        .strip_prefix("op=")
        .or_else(|| text.trim().strip_prefix("operation="))
        .unwrap_or(text.trim())
        .trim();
    if raw.is_empty() || raw.contains(char::is_whitespace) {
        return None;
    }
    Some(raw.to_string())
}

fn parse_line(line: &str) -> Option<Rule> {
    let (kind, rest) = next_rule_token(line)?;
    let (pattern, rest) = next_rule_token(rest)?;
    let (host, path_prefix) = split_pattern(&pattern);
    match kind.as_str() {
        "map-local" => {
            let tokens = rule_tokens(rest)?;
            let mut file: Option<PathBuf> = None;
            let mut status = None;
            let mut delay = Duration::ZERO;
            let mut op = None;
            let mut fallback = false;
            for token in tokens {
                if let Some(code_str) = token.strip_prefix("status=") {
                    status = Some(parse_status_opt(code_str)?);
                } else if let Some(delay_str) = token.strip_prefix("delay=") {
                    delay = parse_delay(delay_str)?;
                } else if let Some(op_str) = token
                    .strip_prefix("op=")
                    .or_else(|| token.strip_prefix("operation="))
                {
                    if op.is_some() {
                        return None;
                    }
                    op = Some(parse_op_opt(op_str)?);
                } else if token.eq_ignore_ascii_case("fallback")
                    || token.eq_ignore_ascii_case("fallback=true")
                    || token.eq_ignore_ascii_case("fallback=1")
                {
                    fallback = true;
                } else if token.eq_ignore_ascii_case("fallback=false")
                    || token.eq_ignore_ascii_case("fallback=0")
                {
                    fallback = false;
                } else if file.is_none() && !token.is_empty() {
                    file = Some(PathBuf::from(token));
                } else {
                    return None;
                }
            }
            if (file.is_none() && status.is_none()) || (fallback && file.is_none()) {
                return None;
            }
            Some(Rule::MapLocal {
                host,
                path_prefix,
                file,
                status,
                delay,
                op,
                fallback,
            })
        }
        "map-remote" => {
            let tokens = rule_tokens(rest)?;
            if tokens.is_empty() || tokens.len() > 2 {
                return None;
            }
            let mut url = None;
            let mut op = None;
            for token in tokens {
                if let Some(op_str) = token
                    .strip_prefix("op=")
                    .or_else(|| token.strip_prefix("operation="))
                {
                    if op.is_some() {
                        return None;
                    }
                    op = Some(parse_op_opt(op_str)?);
                } else if url.is_none()
                    && (token.starts_with("http://") || token.starts_with("https://"))
                {
                    url = Some(token);
                } else {
                    return None;
                }
            }
            Some(Rule::MapRemote {
                host,
                path_prefix,
                url: url?,
                op,
            })
        }
        "break" => {
            let tokens = rule_tokens(rest)?;
            let op = match tokens.as_slice() {
                [] => None,
                [token] => {
                    let op_str = token
                        .strip_prefix("op=")
                        .or_else(|| token.strip_prefix("operation="))?;
                    Some(parse_op_opt(op_str)?)
                }
                _ => return None,
            };
            Some(Rule::Break {
                host,
                path_prefix,
                op,
            })
        }
        "break-res" | "break-response" => {
            let tokens = rule_tokens(rest)?;
            let op = match tokens.as_slice() {
                [] => None,
                [token] => {
                    let op_str = token
                        .strip_prefix("op=")
                        .or_else(|| token.strip_prefix("operation="))?;
                    Some(parse_op_opt(op_str)?)
                }
                _ => return None,
            };
            Some(Rule::BreakResponse {
                host,
                path_prefix,
                op,
            })
        }
        "delay" => {
            let tokens = rule_tokens(rest)?;
            if tokens.len() != 1 {
                return None;
            }
            Some(Rule::Delay {
                host,
                path_prefix,
                delay: parse_delay(&tokens[0])?,
            })
        }
        "throttle" => {
            let tokens = rule_tokens(rest)?;
            let (down_bps, up_bps) = parse_throttle_tokens(&tokens)?;
            Some(Rule::Throttle {
                host,
                path_prefix,
                down_bps,
                up_bps,
            })
        }
        "cert" => {
            let tokens = rule_tokens(rest)?;
            if tokens.is_empty() || tokens.len() > 2 || tokens[0].is_empty() {
                return None;
            }
            let cert_path = PathBuf::from(&tokens[0]);
            let key_path = tokens.get(1).filter(|s| !s.is_empty()).map(PathBuf::from);
            Some(Rule::Cert {
                host,
                cert_path,
                key_path,
            })
        }
        "client-cert" | "mtls" => {
            let tokens = rule_tokens(rest)?;
            if tokens.is_empty() || tokens.len() > 2 || tokens[0].is_empty() {
                return None;
            }
            let cert_path = PathBuf::from(&tokens[0]);
            let key_path = tokens.get(1).filter(|s| !s.is_empty()).map(PathBuf::from);
            Some(Rule::ClientCert {
                host,
                cert_path,
                key_path,
            })
        }
        "header" => {
            let (side, rest) = next_rule_token(rest)?;
            let (op, rest) = next_rule_token(rest)?;
            let (name, rest) = next_rule_token(rest)?;
            let name = name.trim_end_matches(':').to_string();
            if name.is_empty() {
                return None;
            }
            let response = match side.as_str() {
                "res" | "response" => true,
                "req" | "request" => false,
                _ => return None,
            };
            let remove = match op.as_str() {
                "del" | "delete" | "rm" => true,
                "set" | "add" => false,
                _ => return None,
            };
            let value = unquote_optional(rest)?;
            if remove && !value.is_empty() {
                return None;
            }
            Some(Rule::Header {
                host,
                path_prefix,
                response,
                remove,
                name,
                value,
            })
        }
        "query" => {
            let (op, rest) = next_rule_token(rest)?;
            let remove = match op.as_str() {
                "del" | "delete" | "rm" => true,
                "set" | "add" => false,
                _ => return None,
            };
            let (first, rest) = next_rule_token(rest)?;
            let (name, value) = if remove {
                if !rest.trim().is_empty() || first.is_empty() {
                    return None;
                }
                (first, String::new())
            } else if let Some((k, v)) = first.split_once('=') {
                if k.is_empty() || !rest.trim().is_empty() {
                    return None;
                }
                (k.to_string(), v.to_string())
            } else {
                let value = unquote_optional(rest)?;
                (first, value)
            };
            if name.is_empty() {
                return None;
            }
            Some(Rule::Query {
                host,
                path_prefix,
                remove,
                name,
                value,
            })
        }
        "cors" => {
            if !rest.trim().is_empty() {
                return None;
            }
            Some(Rule::Cors { host, path_prefix })
        }
        "replace" => {
            let (side, rest) = next_rule_token(rest)?;
            let rest = rest.trim();
            // `from ->` with nothing after the arrow deletes that text.
            // Whitespace around the arrow is not part of either side unless quoted.
            let (raw_from, raw_to) = if let Some((from, to)) = rest.split_once(" -> ") {
                (from, to)
            } else {
                let from = rest.strip_suffix(" ->")?;
                (from, "")
            };
            let from = unquote_optional(raw_from)?;
            let to = unquote_optional(raw_to)?;
            if from.is_empty() {
                return None;
            }
            let response = match side.as_str() {
                "res" | "response" => true,
                "req" | "request" => false,
                _ => return None,
            };
            Some(Rule::Replace {
                host,
                path_prefix,
                response,
                from,
                to,
            })
        }
        "json" => {
            let (side, rest) = next_rule_token(rest)?;
            let response = match side.as_str() {
                "res" | "response" => true,
                "req" | "request" => false,
                _ => return None,
            };
            let (op, rest) = next_rule_token(rest)?;
            let remove = match op.as_str() {
                "del" | "delete" | "rm" => true,
                "set" | "add" => false,
                _ => return None,
            };
            let (json_path, rest) = next_rule_token(rest)?;
            let json_path = json_path.trim().to_string();
            if json_path.is_empty() || parse_json_path(&json_path).is_empty() {
                return None;
            }
            if remove {
                if !rest.trim().is_empty() {
                    return None;
                }
                Some(Rule::Json {
                    host,
                    path_prefix,
                    response,
                    remove: true,
                    json_path,
                    value: serde_json::Value::Null,
                })
            } else {
                let value = parse_json_rule_value(rest)?;
                Some(Rule::Json {
                    host,
                    path_prefix,
                    response,
                    remove: false,
                    json_path,
                    value,
                })
            }
        }
        _ => None,
    }
}

/// Applies `query` rules to a request target (`/path` or `/path?a=1&b=2`).
/// Returns `None` when `ops` is empty, `path` does not start with `/`, or nothing changes.
pub fn apply_query_ops(path: &str, ops: &[QueryOp]) -> Option<String> {
    if ops.is_empty() || !path.starts_with('/') {
        return None;
    }
    let (base_path, query_str) = match path.split_once('?') {
        Some((base, q)) => (base, q),
        None => (path, ""),
    };
    let mut pairs: Vec<(String, Option<String>)> = query_str
        .split('&')
        .filter(|seg| !seg.is_empty())
        .map(|seg| match seg.split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (seg.to_string(), None),
        })
        .collect();

    for op in ops {
        match op {
            QueryOp::Remove { name } => {
                pairs.retain(|(k, _)| k != name && decode_query_component(k) != *name);
            }
            QueryOp::Set { name, value } => {
                let encoded_name = encode_query_component(name);
                let encoded_val = encode_query_component(value);
                let mut replaced = false;
                let mut next_pairs = Vec::with_capacity(pairs.len() + 1);
                for (k, v) in pairs {
                    if k == *name || decode_query_component(&k) == *name {
                        if !replaced {
                            next_pairs.push((encoded_name.clone(), Some(encoded_val.clone())));
                            replaced = true;
                        }
                    } else {
                        next_pairs.push((k, v));
                    }
                }
                if !replaced {
                    next_pairs.push((encoded_name, Some(encoded_val)));
                }
                pairs = next_pairs;
            }
        }
    }

    let next = if pairs.is_empty() {
        base_path.to_string()
    } else {
        let joined = pairs
            .into_iter()
            .map(|(k, v)| match v {
                Some(v) => format!("{k}={v}"),
                None => k,
            })
            .collect::<Vec<_>>()
            .join("&");
        format!("{base_path}?{joined}")
    };
    (next != path).then_some(next)
}

fn decode_query_component(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0;
    while idx < bytes.len() {
        if bytes[idx] == b'+' {
            out.push(b' ');
            idx += 1;
        } else if bytes[idx] == b'%'
            && idx + 2 < bytes.len()
            && let Ok(val) = u8::from_str_radix(&input[idx + 1..idx + 3], 16)
        {
            out.push(val);
            idx += 3;
        } else {
            out.push(bytes[idx]);
            idx += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn encode_query_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b':'
            | b'/'
            | b'%' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Replaces each pair in order. `None` when the text is not UTF-8, `from` is
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

/// Applies `req` pairs to the request target, path and query together.
/// `None` when nothing changes or the result would not still start with `/`.
pub fn replace_target(path: &str, pairs: &[(String, String)]) -> Option<String> {
    let next = apply_replaces(path.as_bytes(), pairs)?;
    let next = String::from_utf8(next).ok()?;
    next.starts_with('/').then_some(next)
}

fn parse_json_rule_value(raw: &str) -> Option<serde_json::Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Some(val);
    }
    let unquoted = unquote_optional(trimmed)?;
    Some(serde_json::Value::String(unquoted))
}

/// Parses either RFC 6901 JSON Pointer (`/a/b/0`) or dot/bracket notation (`$.a.b[0]` / `a.b.0`).
fn parse_json_path(path: &str) -> Vec<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if let Some(rest) = trimmed.strip_prefix('/') {
        return rest
            .split('/')
            .map(|seg| seg.replace("~1", "/").replace("~0", "~"))
            .collect();
    }
    let body = trimmed
        .strip_prefix("$.")
        .or_else(|| trimmed.strip_prefix('$'))
        .unwrap_or(trimmed);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '.' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            '[' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                let mut bracket = String::new();
                for bch in chars.by_ref() {
                    if bch == ']' {
                        break;
                    }
                    bracket.push(bch);
                }
                let btrimmed = bracket.trim().trim_matches('"').trim_matches('\'');
                if !btrimmed.is_empty() {
                    out.push(btrimmed.to_string());
                }
            }
            _ => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Applies JSON-aware `set`/`del` operations in order.
/// - Returns `Ok(Some(bytes))` when valid JSON was modified.
/// - Returns `Ok(None)` when `ops` is empty or the operations did not change the JSON document.
/// - Returns `Err(reason)` when `ops` is non-empty and `body` is not valid UTF-8 JSON.
pub fn apply_json_ops(body: &[u8], ops: &[JsonOp]) -> Result<Option<Vec<u8>>, &'static str> {
    if ops.is_empty() {
        return Ok(None);
    }
    let text = std::str::from_utf8(body).map_err(|_| "non-UTF-8 body")?;
    let mut root: serde_json::Value =
        serde_json::from_str(text.trim()).map_err(|_| "non-JSON body")?;
    let original = root.clone();
    for op in ops {
        match op {
            JsonOp::Set { path, value } => {
                let segs = parse_json_path(path);
                if !segs.is_empty() {
                    json_set_at(&mut root, &segs, value.clone());
                }
            }
            JsonOp::Remove { path } => {
                let segs = parse_json_path(path);
                if !segs.is_empty() {
                    json_remove_at(&mut root, &segs);
                }
            }
        }
    }
    if root == original {
        return Ok(None);
    }
    serde_json::to_vec(&root)
        .map(Some)
        .map_err(|_| "JSON serialization failed")
}

fn json_set_at(node: &mut serde_json::Value, segs: &[String], val: serde_json::Value) {
    let Some((first, rest)) = segs.split_first() else {
        return;
    };
    if rest.is_empty() {
        match node {
            serde_json::Value::Object(map) => {
                map.insert(first.clone(), val);
            }
            serde_json::Value::Array(arr) => {
                if first == "-" {
                    arr.push(val);
                } else if let Ok(idx) = first.parse::<usize>() {
                    if idx < arr.len() {
                        arr[idx] = val;
                    } else if idx == arr.len() {
                        arr.push(val);
                    }
                }
            }
            _ => {}
        }
        return;
    }
    match node {
        serde_json::Value::Object(map) => {
            let entry = map
                .entry(first.clone())
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
            if entry.is_null() {
                *entry = serde_json::Value::Object(serde_json::Map::new());
            }
            json_set_at(entry, rest, val);
        }
        serde_json::Value::Array(arr) => {
            if let Ok(idx) = first.parse::<usize>()
                && let Some(child) = arr.get_mut(idx)
            {
                json_set_at(child, rest, val);
            }
        }
        _ => {}
    }
}

fn json_remove_at(node: &mut serde_json::Value, segs: &[String]) {
    let Some((first, rest)) = segs.split_first() else {
        return;
    };
    if rest.is_empty() {
        match node {
            serde_json::Value::Object(map) => {
                map.remove(first);
            }
            serde_json::Value::Array(arr) => {
                if let Ok(idx) = first.parse::<usize>()
                    && idx < arr.len()
                {
                    arr.remove(idx);
                }
            }
            _ => {}
        }
        return;
    }
    match node {
        serde_json::Value::Object(map) => {
            if let Some(child) = map.get_mut(first) {
                json_remove_at(child, rest);
            }
        }
        serde_json::Value::Array(arr) => {
            if let Ok(idx) = first.parse::<usize>()
                && let Some(child) = arr.get_mut(idx)
            {
                json_remove_at(child, rest);
            }
        }
        _ => {}
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QuickRule {
    Replace,
    Json,
    Remote,
    Local,
    Delay,
    Throttle,
    Break,
    BreakRes,
    Header,
    Query,
    Cors,
    Cert,
}

impl QuickRule {
    pub fn label(self) -> (&'static str, &'static str) {
        match self {
            Self::Replace => ("替换", "Replace"),
            Self::Json => ("JSON", "JSON"),
            Self::Remote => ("转发", "Forward"),
            Self::Local => ("文件", "File"),
            Self::Delay => ("延迟", "Delay"),
            Self::Throttle => ("限速", "Throttle"),
            Self::Break => ("断点", "Break"),
            Self::BreakRes => ("响应断点", "Break res"),
            Self::Header => ("改头", "Header"),
            Self::Query => ("改参", "Query"),
            Self::Cors => ("跨域", "CORS"),
            Self::Cert => ("证书", "Cert"),
        }
    }
}

/// Counts 1-based line numbers whose rule syntax could not be parsed.
pub fn invalid_rule_lines(text: &str) -> Vec<usize> {
    let mut bad = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let stripped = strip_rule_comment(line).trim();
        if stripped.is_empty() {
            continue;
        }
        if parse_line(stripped).is_none() {
            bad.push(index + 1);
        }
    }
    bad
}

/// `host` plus the path, without the query. Both must be one token.
pub fn request_pattern(host: &str, path: &str) -> Option<String> {
    let host = host.trim();
    let path = path.split('?').next().unwrap_or(path).trim();
    if host.is_empty() || host.contains(char::is_whitespace) || path.contains(char::is_whitespace) {
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

/// Appends (or updates an existing `map-local <pattern>` line) for a captured flow.
pub fn append_map_local_line(
    rules: &str,
    host: &str,
    path: &str,
    file: Option<&str>,
    status: Option<u16>,
) -> Option<String> {
    let pattern = request_pattern(host, path)?;
    let file = file.map(str::trim).filter(|s| !s.is_empty());
    let status = status.filter(|code| (100..600).contains(code));
    if file.is_none() && status.is_none() {
        return None;
    }
    let mut line = format!("map-local {pattern}");
    if let Some(file) = file {
        if file.contains('\n') || file.contains('\r') {
            return None;
        }
        line.push(' ');
        line.push_str(&quote_rule_token(file));
    }
    if let Some(code) = status
        && (file.is_none() || code != 200)
    {
        line.push_str(&format!(" status={code}"));
    }
    push_line(rules, &line)
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
        QuickRule::Json => {
            if from.is_empty()
                || from.contains(char::is_whitespace)
                || parse_json_path(from).is_empty()
                || to.contains('\n')
            {
                return None;
            }
            let side = if response { "res" } else { "req" };
            if to.is_empty() {
                format!("json {pattern} {side} del {from}")
            } else {
                format!("json {pattern} {side} set {from} {to}")
            }
        }
        QuickRule::Remote => {
            if !(from.starts_with("http://") || from.starts_with("https://"))
                || from.contains(char::is_whitespace)
            {
                return None;
            }
            if to.is_empty() {
                format!("map-remote {pattern} {from}")
            } else {
                let op = parse_op_opt(to)?;
                format!("map-remote {pattern} {from} op={op}")
            }
        }
        QuickRule::Local => {
            if from.contains('\n') || to.contains('\n') {
                return None;
            }
            let status_from_to = if to.is_empty() {
                None
            } else {
                Some(parse_status_opt(to)?)
            };
            if from.is_empty() {
                let code = status_from_to?;
                format!("map-local {pattern} status={code}")
            } else if status_from_to.is_none()
                && let Some(code) = from.strip_prefix("status=").and_then(parse_status_opt)
            {
                format!("map-local {pattern} status={code}")
            } else {
                let quoted = quote_rule_token(from);
                let fb = if response { " fallback" } else { "" };
                match status_from_to {
                    Some(code) => format!("map-local {pattern} {quoted} status={code}{fb}"),
                    None => format!("map-local {pattern} {quoted}{fb}"),
                }
            }
        }
        QuickRule::Delay => {
            if from.is_empty() || from.contains(char::is_whitespace) || parse_delay(from).is_none()
            {
                return None;
            }
            format!("delay {pattern} {from}")
        }
        QuickRule::Throttle => {
            let spec = if to.is_empty() {
                from.to_string()
            } else if from.is_empty() {
                format!("up={to}")
            } else if from.contains('=') || to.contains('=') {
                format!("{from} {to}")
            } else {
                format!("down={from} up={to}")
            };
            let tokens = rule_tokens(&spec)?;
            let _ = parse_throttle_tokens(&tokens)?;
            format!("throttle {pattern} {}", tokens.join(" "))
        }
        QuickRule::Break => {
            if from.is_empty() {
                format!("break {pattern}")
            } else {
                let op = parse_op_opt(from)?;
                format!("break {pattern} op={op}")
            }
        }
        QuickRule::BreakRes => {
            if from.is_empty() {
                format!("break-res {pattern}")
            } else {
                let op = parse_op_opt(from)?;
                format!("break-res {pattern} op={op}")
            }
        }
        QuickRule::Header => {
            let from = from.trim_end_matches(':').trim();
            if from.is_empty() || from.contains(char::is_whitespace) || to.contains('\n') {
                return None;
            }
            let side = if response { "res" } else { "req" };
            if to.is_empty() {
                format!("header {pattern} {side} del {from}")
            } else {
                format!("header {pattern} {side} set {from} {to}")
            }
        }
        QuickRule::Query => {
            if from.is_empty()
                || from.contains(char::is_whitespace)
                || from.contains('=')
                || to.contains('\n')
            {
                return None;
            }
            if to.is_empty() {
                format!("query {pattern} del {from}")
            } else {
                let quoted_to = quote_rule_token(to);
                format!("query {pattern} set {from} {quoted_to}")
            }
        }
        QuickRule::Cors => format!("cors {pattern}"),
        QuickRule::Cert => {
            if from.is_empty() || from.contains('\n') || to.contains('\n') {
                return None;
            }
            let (host_only, _) = split_pattern(pattern);
            let cmd = if response { "client-cert" } else { "cert" };
            let quoted_cert = quote_rule_token(from);
            if to.is_empty() {
                format!("{cmd} {host_only} {quoted_cert}")
            } else {
                let quoted_key = quote_rule_token(to);
                format!("{cmd} {host_only} {quoted_cert} {quoted_key}")
            }
        }
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

/// One line for the rules box. `from` is quoted when it contains spaces; `->` is rejected.
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
        || from.contains("->")
        || pattern.contains('\n')
        || from.contains('\n')
        || to.contains('\n')
    {
        return None;
    }
    let side = if response { "res" } else { "req" };
    let quoted_from = quote_rule_token(from);
    let line = format!("replace {pattern} {side} {quoted_from} -> {to}");
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

pub const THROTTLE_MIN_BPS: u64 = 512;
pub const THROTTLE_MAX_BPS: u64 = 100 * 1024 * 1024;

/// Parses `throttle` rule tokens into `(down_bps, up_bps)` in bytes per second.
/// Supports presets (`2g` / `slow-3g`, `3g`, `4g` / `slow-4g`), directional
/// `down=<rate>` / `up=<rate>`, or a single bare `<rate>` token (applied to both
/// download and upload).
pub fn parse_throttle_tokens(tokens: &[String]) -> Option<(Option<u64>, Option<u64>)> {
    if tokens.is_empty() {
        return None;
    }
    if tokens.len() == 1 {
        let single = tokens[0].trim();
        let lower = single.to_ascii_lowercase();
        match lower.as_str() {
            "2g" | "slow-3g" | "slow3g" => return Some((Some(50_000), Some(20_000))),
            "3g" | "regular-3g" | "fast-3g" => return Some((Some(96_000), Some(32_000))),
            "4g" | "slow-4g" | "slow4g" => return Some((Some(512_000), Some(128_000))),
            _ => {}
        }
        if !lower.contains('=') {
            let bps = parse_bandwidth_bps(single)?;
            return Some((Some(bps), Some(bps)));
        }
    }
    let mut down_bps = None;
    let mut up_bps = None;
    for token in tokens {
        let lower = token.trim().to_ascii_lowercase();
        if let Some(rest) = lower
            .strip_prefix("down=")
            .or_else(|| lower.strip_prefix("dl="))
        {
            if down_bps.is_some() {
                return None;
            }
            down_bps = Some(parse_bandwidth_bps(rest)?);
        } else {
            let rest = lower
                .strip_prefix("up=")
                .or_else(|| lower.strip_prefix("ul="))?;
            if up_bps.is_some() {
                return None;
            }
            up_bps = Some(parse_bandwidth_bps(rest)?);
        }
    }
    if down_bps.is_none() && up_bps.is_none() {
        return None;
    }
    Some((down_bps, up_bps))
}

/// Parses a bandwidth rate token into bytes per second (`B/s`), clamped to
/// `[THROTTLE_MIN_BPS, THROTTLE_MAX_BPS]`.
///
/// Supported units (case-insensitive):
/// - `mb/s`, `mbyte/s`: `n * 1024 * 1024` B/s
/// - `mbps`, `mbit/s`, `mb`: `n * 125_000` B/s (megabits/s)
/// - `kb/s`, `kbyte/s`: `n * 1024` B/s
/// - `kbps`, `kbit/s`, `kb`, or bare integer: `n * 125` B/s (kilobits/s; `8 kbps = 1000 B/s`)
/// - `b/s`, `bps`: `n` B/s
pub fn parse_bandwidth_bps(text: &str) -> Option<u64> {
    let lower = text.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    let (num_str, multiplier, divisor) = if let Some(n) = lower
        .strip_suffix("mb/s")
        .or_else(|| lower.strip_suffix("mbyte/s"))
    {
        (n, 1024 * 1024, 1)
    } else if let Some(n) = lower
        .strip_suffix("mbps")
        .or_else(|| lower.strip_suffix("mbit/s"))
        .or_else(|| lower.strip_suffix("mb"))
    {
        (n, 1_000_000, 8)
    } else if let Some(n) = lower
        .strip_suffix("kb/s")
        .or_else(|| lower.strip_suffix("kbyte/s"))
    {
        (n, 1024, 1)
    } else if let Some(n) = lower
        .strip_suffix("kbps")
        .or_else(|| lower.strip_suffix("kbit/s"))
        .or_else(|| lower.strip_suffix("kb"))
    {
        (n, 1000, 8)
    } else if let Some(n) = lower
        .strip_suffix("b/s")
        .or_else(|| lower.strip_suffix("bps"))
    {
        (n, 1, 1)
    } else {
        (lower.as_str(), 1000, 8)
    };
    let num_str = num_str.trim();
    if num_str.is_empty() || !num_str.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let val = num_str.parse::<u64>().ok()?;
    if val == 0 {
        return None;
    }
    let bps = val.saturating_mul(multiplier) / divisor;
    Some(bps.clamp(THROTTLE_MIN_BPS, THROTTLE_MAX_BPS))
}

/// Builds a single-token rule pattern from a domain input (which may already
/// include a path, or multiple domains separated by `|` / `,` / spaces) and an
/// optional path-prefix input (multiple prefixes separated by `|` / `,` / spaces).
pub fn combine_rule_pattern(host_input: &str, path_input: &str) -> String {
    let host_trimmed = host_input.trim();
    let path_trimmed = path_input.trim();
    if path_trimmed.is_empty()
        && host_trimmed.contains('/')
        && !host_trimmed.contains(char::is_whitespace)
    {
        return host_trimmed.to_string();
    }
    let hosts: Vec<String> = host_trimmed
        .split(|ch: char| ch == '|' || ch == ',' || ch.is_whitespace())
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| part.trim_end_matches('/').to_string())
        .collect();
    let paths: Vec<String> = path_trimmed
        .split(|ch: char| ch == '|' || ch == ',' || ch.is_whitespace())
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            if part.starts_with('/') {
                part.to_string()
            } else {
                format!("/{part}")
            }
        })
        .collect();
    // An empty scope matches every host and every path. `*` is all hosts;
    // a missing path is `/`, which already matches every path.
    if hosts.is_empty() && paths.is_empty() {
        return "*".to_string();
    }
    let host_part = if hosts.is_empty() {
        "*".to_string()
    } else {
        hosts.join("|")
    };
    if paths.is_empty() {
        host_part
    } else {
        format!("{host_part}{}", paths.join("|"))
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
        (normalize_host_list(host), path)
    } else {
        (normalize_host_list(pattern), "/".to_string())
    }
}

fn normalize_host_list(rule: &str) -> String {
    rule.split(['|', ','])
        .map(normalize_host)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("|")
}

fn host_matches(rule: &str, host: &str) -> bool {
    let host = normalize_host(host);
    rule.split(['|', ','])
        .map(normalize_host)
        .filter(|part| !part.is_empty())
        .any(|part| {
            if part == "*" {
                true
            } else if let Some(rest) = part.strip_prefix("*.") {
                host.ends_with(&format!(".{rest}"))
            } else {
                part == host
            }
        })
}

fn path_matches(prefix: &str, path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    prefix
        .split(['|', ','])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .any(|part| {
            let p = if part.starts_with('/') {
                part.to_string()
            } else {
                format!("/{part}")
            };
            if p == "/" {
                return true;
            }
            let trimmed = p.trim_end_matches('/');
            path == p
                || path == trimmed
                || path.starts_with(&format!("{trimmed}/"))
                || path.starts_with(&p)
        })
}

fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_matches(|ch| ch == '[' || ch == ']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

pub fn op_matches(pattern: &str, ops: &[String]) -> bool {
    if ops.is_empty() {
        return false;
    }
    pattern
        .split(['|', ','])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .any(|part| {
            let pat_lower = part.to_ascii_lowercase();
            ops.iter()
                .any(|op| wildcard_matches(&pat_lower, &op.to_ascii_lowercase()))
        })
}

fn wildcard_matches(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == text;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut pos = 0usize;
    for (idx, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if idx == 0 {
            if !text.starts_with(part) {
                return false;
            }
            pos = part.len();
        } else if idx + 1 == parts.len() {
            if !text[pos..].ends_with(part) {
                return false;
            }
            pos = text.len();
        } else if let Some(found) = text[pos..].find(part) {
            pos += found + part.len();
        } else {
            return false;
        }
    }
    true
}

#[derive(Debug)]
pub enum MapLocalError {
    NotFound(std::io::Error),
    Io(std::io::Error),
    Traversal,
}

impl MapLocalError {
    pub fn message(&self) -> String {
        match self {
            Self::NotFound(err) | Self::Io(err) => format!("map-local 读不到文件: {err}"),
            Self::Traversal => "map-local 拒绝越界路径: 请求路径超出映射目录".to_string(),
        }
    }
}

/// Resolves a `map-local` file or directory target against the matched `path_prefix`
/// and incoming `req_path` (which may include a `?query` string).
///
/// - When `file` is `None`, returns `Ok(None)` (status-only response with empty body).
/// - When `file` is a regular file, returns `Ok(Some(file))`.
/// - When `file` is a directory, strips the matched `path_prefix` and `?query`,
///   percent-decodes relative segments, defaults `/` or a subdirectory to `index.html`,
///   and verifies via `canonicalize` that the resolved path stays inside the directory root.
pub fn resolve_map_local_target(
    file: Option<&std::path::Path>,
    path_prefix: &str,
    req_path: &str,
) -> Result<Option<PathBuf>, MapLocalError> {
    let Some(base) = file else {
        return Ok(None);
    };
    let meta = match std::fs::metadata(base) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if std::fs::symlink_metadata(base).is_ok() {
                return Err(MapLocalError::Io(err));
            }
            return Err(MapLocalError::NotFound(err));
        }
        Err(err) => return Err(MapLocalError::Io(err)),
    };
    if !meta.is_dir() {
        return Ok(Some(base.to_path_buf()));
    }
    let canonical_root = std::fs::canonicalize(base).map_err(MapLocalError::Io)?;
    let clean_path = req_path.split(['?', '#']).next().unwrap_or(req_path);
    let suffix = matched_route_suffix(path_prefix, clean_path);
    if suffix.contains('\\') || suffix.contains('\0') {
        return Err(MapLocalError::Traversal);
    }
    let mut cur = canonical_root.clone();
    for raw_seg in suffix.split('/') {
        if raw_seg.is_empty() || raw_seg == "." {
            continue;
        }
        if raw_seg == ".." {
            return Err(MapLocalError::Traversal);
        }
        let seg = percent_decode_segment(raw_seg).ok_or(MapLocalError::Traversal)?;
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." || seg.contains('/') || seg.contains('\\') || seg.contains('\0') {
            return Err(MapLocalError::Traversal);
        }
        let mut comps = std::path::Path::new(&seg).components();
        if !matches!(comps.next(), Some(std::path::Component::Normal(_))) || comps.next().is_some()
        {
            return Err(MapLocalError::Traversal);
        }
        let next = cur.join(&seg);
        if std::fs::symlink_metadata(&next).is_ok() {
            let canon = std::fs::canonicalize(&next).map_err(|_| MapLocalError::Traversal)?;
            if !canon.starts_with(&canonical_root) {
                return Err(MapLocalError::Traversal);
            }
        }
        cur = next;
    }
    if cur == canonical_root || suffix.ends_with('/') || cur.is_dir() {
        cur = cur.join("index.html");
    }
    let meta = match std::fs::symlink_metadata(&cur) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(MapLocalError::NotFound(err));
        }
        Err(err) => return Err(MapLocalError::Io(err)),
    };
    let canon_file = std::fs::canonicalize(&cur).map_err(|err| {
        if meta.file_type().is_symlink() {
            MapLocalError::Traversal
        } else {
            MapLocalError::Io(err)
        }
    })?;
    if !canon_file.starts_with(&canonical_root) {
        return Err(MapLocalError::Traversal);
    }
    if !canon_file.is_file() {
        return Err(MapLocalError::NotFound(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "resolved path is not a regular file",
        )));
    }
    Ok(Some(canon_file))
}

fn matched_route_suffix<'a>(prefix: &str, clean_path: &'a str) -> &'a str {
    let mut best_len: Option<usize> = None;
    for part in prefix
        .split(['|', ','])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let p = if part.starts_with('/') {
            part.to_string()
        } else {
            format!("/{part}")
        };
        if p == "/" {
            best_len = Some(best_len.unwrap_or(0));
            continue;
        }
        let trimmed = p.trim_end_matches('/');
        if clean_path == trimmed || clean_path.starts_with(&format!("{trimmed}/")) {
            let len = trimmed.len();
            if best_len.is_none_or(|cur| len > cur) {
                best_len = Some(len);
            }
        } else if clean_path.starts_with(&p) {
            let len = p.len();
            if best_len.is_none_or(|cur| len > cur) {
                best_len = Some(len);
            }
        }
    }
    let cut = best_len.unwrap_or(0).min(clean_path.len());
    &clean_path[cut..]
}

fn percent_decode_segment(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0;
    while idx < bytes.len() {
        if bytes[idx] == b'%' {
            if idx + 2 >= bytes.len() {
                return None;
            }
            let hi = (bytes[idx + 1] as char).to_digit(16)? as u8;
            let lo = (bytes[idx + 2] as char).to_digit(16)? as u8;
            out.push((hi << 4) | lo);
            idx += 3;
        } else {
            out.push(bytes[idx]);
            idx += 1;
        }
    }
    String::from_utf8(out).ok()
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
                path_prefix: "/api".to_string(),
                file: Some(PathBuf::from("./a.json")),
                status: 429,
                fallback: false,
            })
        );
        assert_eq!(applied.request_headers.len(), 1);
        assert!(matches!(
            &applied.response_headers[0],
            HeaderOp::Remove { name } if name == "Set-Cookie"
        ));
        let capped = effect(&parse_rules("delay example.com 40s"), "example.com", "/");
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
        let broken = append_quick_rule(
            "",
            QuickRule::Remote,
            "example.com/api",
            true,
            "not-a-url",
            "",
        );
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
        assert_eq!(
            forwarded,
            "map-remote example.com/api https://other.test/v1\n"
        );
        let stopped = append_break_line("", "example.com", "/pay?x=1").unwrap();
        assert_eq!(
            hit_summary(
                &parse_rules(
                    "delay example.com 1s\nreplace example.com res a -> b\nbreak example.com\nbreak-res example.com"
                ),
                "example.com",
                "/",
            ),
            "delay replace break"
        );
        assert_eq!(without_break_labels("delay replace break"), "delay replace");
        assert_eq!(without_break_labels("break-res"), "");
        let paused = effect(
            &parse_rules(
                "delay example.com 1s\nheader example.com res set X-Test 1\nbreak example.com",
            ),
            "example.com",
            "/",
        );
        assert!(matches!(paused.action, Some(Action::BreakRequest)));
        let kept = without_breaks(paused);
        assert!(kept.action.is_none());
        assert!(!kept.delay.is_zero());
        assert_eq!(kept.response_headers.len(), 1);
        assert_eq!(stopped, "break example.com/pay\n");
        assert!(append_break_line("", "has space", "/pay").is_none());
    }

    #[test]
    fn req_replace_rewrites_the_path_and_query() {
        let rules = parse_rules("replace example.com/api req ?draft=1 -> ?draft=0\n");
        let path = "/api?draft=1";
        assert!(rules[0].matches("example.com", path));
        let applied = effect(&rules, "example.com", path);
        assert_eq!(
            replace_target(path, &applied.request_replaces).as_deref(),
            Some("/api?draft=0")
        );
        assert!(
            replace_target(path, &applied.request_replaces)
                .unwrap()
                .starts_with('/')
        );
        let dropped = effect(
            &parse_rules("replace example.com/api req ?draft=1 -> \n"),
            "example.com",
            path,
        );
        assert_eq!(
            replace_target(path, &dropped.request_replaces).as_deref(),
            Some("/api")
        );
        assert!(replace_target("/keep", &[("/keep".into(), "away".into())]).is_none());
        assert!(
            !parse_rules("replace /api req ?draft=1 -> ?draft=0")[0].matches("example.com", path)
        );
    }

    #[test]
    fn rules_support_domain_or_and_path_prefix_or() {
        let pattern = combine_rule_pattern("a.test | b.test", "/api | /v2");
        assert_eq!(pattern, "a.test|b.test/api|/v2");
        let rules = parse_rules(&format!("break {pattern}\n"));
        assert!(rules[0].matches("a.test", "/api/v1"));
        assert!(rules[0].matches("b.test", "/v2/items"));
        assert!(!rules[0].matches("c.test", "/api/v1"));
        assert!(!rules[0].matches("a.test", "/other"));
        assert_eq!(combine_rule_pattern("", ""), "*");
        assert_eq!(combine_rule_pattern("example.com", ""), "example.com");
        assert_eq!(combine_rule_pattern("", "/api | /v2"), "*/api|/v2");
        assert_eq!(
            append_quick_rule("", QuickRule::Break, "*", false, "", "").as_deref(),
            Some("break *\n")
        );
        let every = parse_rules("break *\n");
        assert!(every[0].matches("z.test", "/anywhere"));
        let paths_only = parse_rules(&format!("break {}\n", combine_rule_pattern("", "/api")));
        assert!(paths_only[0].matches("z.test", "/api/v1"));
        assert!(!paths_only[0].matches("z.test", "/other"));

        let hdr_set = append_quick_rule(
            "",
            QuickRule::Header,
            "example.com/api",
            true,
            "X-Debug",
            "1",
        )
        .unwrap();
        assert_eq!(hdr_set, "header example.com/api res set X-Debug 1\n");
        let hdr_del = append_quick_rule(
            "",
            QuickRule::Header,
            "example.com/api",
            false,
            "Cookie",
            "",
        )
        .unwrap();
        assert_eq!(hdr_del, "header example.com/api req del Cookie\n");
        let cors = append_quick_rule("", QuickRule::Cors, "example.com", true, "", "").unwrap();
        assert_eq!(cors, "cors example.com\n");
        assert_eq!(
            invalid_rule_lines(
                "break example.com\nbad-rule foo\n# comment\nmap-remote a.com not_url"
            ),
            vec![2, 4]
        );
    }

    #[test]
    fn query_rules_and_quoted_map_local_work() {
        let rules = parse_rules(
            "\
query example.com/api set env staging
query example.com/api set msg \"hello world\"
query example.com/api del debug
map-local example.com/api/mock \"/Users/tree/My Mocks/data.json\" status=201
map-local example.com/api/empty status=503 delay=200ms
replace example.com/api res \"old text\" -> \"new text\"
",
        );
        assert_eq!(rules.len(), 6);
        let eff = effect(&rules, "example.com", "/api/items?debug=1&page=2&env=prod");
        assert_eq!(
            apply_query_ops("/api/items?debug=1&page=2&env=prod", &eff.query_ops).as_deref(),
            Some("/api/items?page=2&env=staging&msg=hello%20world")
        );
        assert_eq!(
            hit_summary(&rules, "example.com", "/api/mock"),
            "query map-local replace"
        );
        let mock_eff = effect(&rules, "example.com", "/api/mock");
        assert_eq!(
            mock_eff.action,
            Some(Action::MapLocal {
                path_prefix: "/api/mock".to_string(),
                file: Some(PathBuf::from("/Users/tree/My Mocks/data.json")),
                status: 201,
                fallback: false,
            })
        );
        let empty_eff = effect(&rules, "example.com", "/api/empty");
        assert_eq!(
            empty_eff.action,
            Some(Action::MapLocal {
                path_prefix: "/api/empty".to_string(),
                file: None,
                status: 503,
                fallback: false,
            })
        );
        assert_eq!(empty_eff.delay, Duration::from_millis(200));
        assert_eq!(
            empty_eff.response_replaces,
            vec![("old text".into(), "new text".into())]
        );

        let mapped = append_map_local_line(
            "",
            "example.com",
            "/v1/user?id=1",
            Some("/tmp/my mock/user.json"),
            Some(200),
        )
        .unwrap();
        assert_eq!(
            mapped,
            "map-local example.com/v1/user \"/tmp/my mock/user.json\"\n"
        );
        let mapped_status =
            append_map_local_line("", "example.com", "/v1/err", None, Some(503)).unwrap();
        assert_eq!(mapped_status, "map-local example.com/v1/err status=503\n");
        let q_set = append_quick_rule(
            "",
            QuickRule::Query,
            "example.com/api",
            false,
            "env",
            "staging",
        )
        .unwrap();
        assert_eq!(q_set, "query example.com/api set env staging\n");
        let q_del =
            append_quick_rule("", QuickRule::Query, "example.com/api", false, "token", "").unwrap();
        assert_eq!(q_del, "query example.com/api del token\n");
    }

    #[test]
    fn throttle_rules_parse_presets_and_directional_rates() {
        let rules = parse_rules(
            "\
throttle example.com/api down=256kbps up=64kbps
throttle example.com/slow 3g
throttle example.com/fast 64kb/s
",
        );
        assert_eq!(rules.len(), 3);
        let eff_api = effect(&rules, "example.com", "/api/v1");
        assert_eq!(eff_api.throttle_down_bps, Some(32_000));
        assert_eq!(eff_api.throttle_up_bps, Some(8_000));
        assert_eq!(hit_summary(&rules, "example.com", "/api/v1"), "throttle");

        let eff_3g = effect(&rules, "example.com", "/slow/item");
        assert_eq!(eff_3g.throttle_down_bps, Some(96_000));
        assert_eq!(eff_3g.throttle_up_bps, Some(32_000));

        let eff_kb = effect(&rules, "example.com", "/fast");
        assert_eq!(eff_kb.throttle_down_bps, Some(64 * 1024));
        assert_eq!(eff_kb.throttle_up_bps, Some(64 * 1024));

        assert_eq!(
            append_quick_rule(
                "",
                QuickRule::Throttle,
                "example.com/api",
                false,
                "256kbps",
                "64kbps"
            )
            .as_deref(),
            Some("throttle example.com/api down=256kbps up=64kbps\n")
        );
        assert_eq!(
            append_quick_rule("", QuickRule::Throttle, "example.com", false, "3g", "").as_deref(),
            Some("throttle example.com 3g\n")
        );
        assert!(
            append_quick_rule("", QuickRule::Throttle, "example.com", false, "invalid", "")
                .is_none()
        );
    }

    #[test]
    fn cert_rules_parse_and_match_hosts() {
        let rules = parse_rules(
            "\
cert *.example.com \"/tmp/my certs/fullchain.pem\" \"/tmp/my certs/privkey.key\"
cert single.test /tmp/bundle.pem
",
        );
        assert_eq!(rules.len(), 2);
        assert_eq!(
            custom_cert(&rules, "api.example.com"),
            Some((
                PathBuf::from("/tmp/my certs/fullchain.pem"),
                Some(PathBuf::from("/tmp/my certs/privkey.key"))
            ))
        );
        assert_eq!(
            custom_cert(&rules, "single.test"),
            Some((PathBuf::from("/tmp/bundle.pem"), None))
        );
        assert_eq!(custom_cert(&rules, "other.test"), None);
        assert_eq!(hit_summary(&rules, "api.example.com", "/v1/orders"), "cert");

        assert_eq!(
            append_quick_rule(
                "",
                QuickRule::Cert,
                "*.example.com/v1",
                false,
                "/tmp/my certs/fullchain.pem",
                "/tmp/my certs/privkey.key",
            )
            .as_deref(),
            Some(
                "cert *.example.com \"/tmp/my certs/fullchain.pem\" \"/tmp/my certs/privkey.key\"\n"
            )
        );
        assert_eq!(
            append_quick_rule(
                "",
                QuickRule::Cert,
                "single.test",
                false,
                "/tmp/bundle.pem",
                "",
            )
            .as_deref(),
            Some("cert single.test /tmp/bundle.pem\n")
        );
    }

    #[test]
    fn json_rules_set_and_remove_by_pointer_or_dot_path() {
        let rules = parse_rules(
            "\
json example.com/api res set /data/enabled true
json example.com/api res set items[0].price 19.5
json example.com/api res set $.meta.tier \"pro\"
json example.com/api res del /data/secret
json example.com/api req set user.roles [\"admin\",\"ops\"]
",
        );
        assert_eq!(rules.len(), 5);
        assert_eq!(hit_summary(&rules, "example.com", "/api/v1"), "json");
        let eff = effect(&rules, "example.com", "/api/v1");
        assert_eq!(eff.response_json_ops.len(), 4);
        assert_eq!(eff.request_json_ops.len(), 1);

        let res_body =
            br#"{"data":{"enabled":false,"secret":"tok"},"items":[{"price":10},{"price":20}]}"#;
        let rewritten = apply_json_ops(res_body, &eff.response_json_ops)
            .unwrap()
            .unwrap();
        let val: serde_json::Value = serde_json::from_slice(&rewritten).unwrap();
        assert_eq!(val["data"]["enabled"], serde_json::json!(true));
        assert!(val["data"].get("secret").is_none());
        assert_eq!(val["items"][0]["price"], serde_json::json!(19.5));
        assert_eq!(val["meta"]["tier"], serde_json::json!("pro"));

        let req_body = br#"{}"#;
        let req_rewritten = apply_json_ops(req_body, &eff.request_json_ops)
            .unwrap()
            .unwrap();
        let req_val: serde_json::Value = serde_json::from_slice(&req_rewritten).unwrap();
        assert_eq!(
            req_val["user"]["roles"],
            serde_json::json!(["admin", "ops"])
        );

        // Non-JSON body returns Err with a clear reason
        assert_eq!(
            apply_json_ops(b"<html>not json</html>", &eff.response_json_ops),
            Err("non-JSON body")
        );

        assert_eq!(
            append_quick_rule(
                "",
                QuickRule::Json,
                "example.com/api",
                true,
                "/data/enabled",
                "true",
            )
            .as_deref(),
            Some("json example.com/api res set /data/enabled true\n")
        );
        assert_eq!(
            append_quick_rule(
                "",
                QuickRule::Json,
                "example.com/api",
                true,
                "/data/secret",
                "",
            )
            .as_deref(),
            Some("json example.com/api res del /data/secret\n")
        );
    }

    #[test]
    fn graphql_op_rules_narrow_terminal_matching() {
        let rules = parse_rules(
            "\
map-local example.com/graphql ./cart.json op=GetCart|UpdateCart
map-remote example.com/graphql https://staging.test/graphql op=Checkout*
break example.com/graphql op=PlaceOrder
break-res example.com/graphql op=anonymous
",
        );
        assert_eq!(rules.len(), 4);
        assert!(has_op_rules(&rules, "example.com", "/graphql"));
        assert!(!has_op_rules(&rules, "other.com", "/graphql"));
        // Without ops, op-scoped rules do not match
        assert!(effect(&rules, "example.com", "/graphql").action.is_none());

        let cart_eff = effect_with_ops(
            &rules,
            "example.com",
            "/graphql",
            &["UpdateCart".to_string()],
        );
        assert_eq!(
            cart_eff.action,
            Some(Action::MapLocal {
                path_prefix: "/graphql".to_string(),
                file: Some(PathBuf::from("./cart.json")),
                status: 200,
                fallback: false,
            })
        );
        assert_eq!(
            hit_summary_with_ops(&rules, "example.com", "/graphql", &["GetCart".to_string()]),
            "map-local"
        );

        let remote_eff = effect_with_ops(
            &rules,
            "example.com",
            "/graphql",
            &["CheckoutSession".to_string()],
        );
        assert_eq!(
            remote_eff.action,
            Some(Action::MapRemote {
                url: "https://staging.test/graphql".to_string()
            })
        );

        let break_eff = effect_with_ops(
            &rules,
            "example.com",
            "/graphql",
            &["PlaceOrder".to_string()],
        );
        assert_eq!(break_eff.action, Some(Action::BreakRequest));

        let break_res_eff = effect_with_ops(
            &rules,
            "example.com",
            "/graphql",
            &["anonymous".to_string()],
        );
        assert_eq!(break_res_eff.action, Some(Action::BreakResponse));
    }

    #[test]
    fn parses_client_cert_rules_and_matches_hosts() {
        let rules = parse_rules(
            "client-cert *.corp.internal /tmp/client.crt /tmp/client.key\n\
             mtls api.partner.test \"/tmp/my bundle/partner.pem\"\n",
        );
        assert_eq!(rules.len(), 2);
        assert_eq!(
            custom_client_cert(&rules, "svc.corp.internal"),
            Some((
                PathBuf::from("/tmp/client.crt"),
                Some(PathBuf::from("/tmp/client.key")),
            ))
        );
        assert_eq!(
            custom_client_cert(&rules, "api.partner.test"),
            Some((PathBuf::from("/tmp/my bundle/partner.pem"), None))
        );
        assert_eq!(custom_client_cert(&rules, "other.example.com"), None);
        assert_eq!(
            hit_summary_with_ops(&rules, "svc.corp.internal", "/v1/health", &[]),
            "client-cert"
        );

        let appended = append_quick_rule(
            "",
            QuickRule::Cert,
            "mtls.example.com/v1",
            true,
            "/tmp/client.pem",
            "/tmp/client.key",
        )
        .unwrap();
        assert_eq!(
            appended,
            "client-cert mtls.example.com /tmp/client.pem /tmp/client.key\n"
        );
    }

    #[test]
    fn map_local_directory_resolves_files_index_and_blocks_traversal() {
        let dir = std::env::temp_dir().join(format!("zniff-mapdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dist = dir.join("dist");
        std::fs::create_dir_all(dist.join("js")).unwrap();
        std::fs::write(dist.join("index.html"), b"<h1>root</h1>").unwrap();
        std::fs::write(dist.join("js/app.js"), b"console.log(1)").unwrap();
        std::fs::write(dist.join("js/hello world.js"), b"console.log(2)").unwrap();
        let outside = dir.join("secret.txt");
        std::fs::write(&outside, b"top-secret").unwrap();

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, dist.join("evil.txt")).unwrap();
            std::os::unix::fs::symlink(&dir, dist.join("evil_dir")).unwrap();
        }

        let rules = parse_rules(&format!(
            "map-local example.com/assets \"{}\" fallback\n",
            dist.display()
        ));
        assert_eq!(rules.len(), 1);
        let eff = effect(&rules, "example.com", "/assets/js/app.js");
        assert_eq!(
            eff.action,
            Some(Action::MapLocal {
                path_prefix: "/assets".to_string(),
                file: Some(dist.clone()),
                status: 200,
                fallback: true,
            })
        );

        let root_index = resolve_map_local_target(Some(&dist), "/assets", "/assets?v=1")
            .unwrap()
            .unwrap();
        assert_eq!(std::fs::read(&root_index).unwrap(), b"<h1>root</h1>");

        let js_file = resolve_map_local_target(Some(&dist), "/assets", "/assets/js/app.js?ts=9")
            .unwrap()
            .unwrap();
        assert_eq!(std::fs::read(&js_file).unwrap(), b"console.log(1)");

        let encoded =
            resolve_map_local_target(Some(&dist), "/assets", "/assets/js/hello%20world.js")
                .unwrap()
                .unwrap();
        assert_eq!(std::fs::read(&encoded).unwrap(), b"console.log(2)");

        // Missing file inside directory returns NotFound
        assert!(matches!(
            resolve_map_local_target(Some(&dist), "/assets", "/assets/js/missing.js"),
            Err(MapLocalError::NotFound(_))
        ));

        // Directory traversal (raw or percent-encoded) is blocked with Traversal
        for bad in [
            "/assets/../secret.txt",
            "/assets/%2e%2e/secret.txt",
            "/assets/js/..%2f..%2fsecret.txt",
            "/assets/\\..\\secret.txt",
        ] {
            assert!(
                matches!(
                    resolve_map_local_target(Some(&dist), "/assets", bad),
                    Err(MapLocalError::Traversal)
                ),
                "expected Traversal for {bad}"
            );
        }

        #[cfg(unix)]
        {
            assert!(matches!(
                resolve_map_local_target(Some(&dist), "/assets", "/assets/evil.txt"),
                Err(MapLocalError::Traversal)
            ));
            assert!(matches!(
                resolve_map_local_target(Some(&dist), "/assets", "/assets/evil_dir/secret.txt"),
                Err(MapLocalError::Traversal)
            ));
        }

        let quick_fb = append_quick_rule(
            "",
            QuickRule::Local,
            "example.com/assets",
            true,
            "./dist",
            "",
        )
        .unwrap();
        assert_eq!(quick_fb, "map-local example.com/assets ./dist fallback\n");

        // Windows paths with unescaped backslashes inside double quotes stay intact
        let win_rules = parse_rules(
            "map-local example.com/assets \"C:\\Users\\RUNNER~1\\AppData\\Local\\Temp\\zniff-mapdir-8100\\dist\" fallback\n",
        );
        assert_eq!(
            effect(&win_rules, "example.com", "/assets/js/app.js").action,
            Some(Action::MapLocal {
                path_prefix: "/assets".to_string(),
                file: Some(PathBuf::from(
                    "C:\\Users\\RUNNER~1\\AppData\\Local\\Temp\\zniff-mapdir-8100\\dist"
                )),
                status: 200,
                fallback: true,
            })
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
