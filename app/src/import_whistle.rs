//! Whistle rule text importer and converter (`*.txt` / pasted rules).
//!
//! Translates common Whistle patterns, named triple-backtick value blocks, and
//! protocol operators into native `zniff` rules. Unmapped constructs are
//! preserved as `# [unmapped whistle] ...` comments so nothing is silently lost.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;

use serde_json::Value;

use crate::rules::{parse_single_rule, quote_rule_token};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleImportReport {
    pub source_name: String,
    pub rules_text: String,
    pub bypass_hosts: Vec<String>,
    pub converted: usize,
    pub unmapped: usize,
    pub skipped_comments: usize,
}

const WHISTLE_PREFIXES: &[&str] = &[
    "file://",
    "xfile://",
    "rawfile://",
    "xrawfile://",
    "statuscode://",
    "replacestatus://",
    "resdelay://",
    "reqdelay://",
    "rescors://",
    "disable://",
    "resspeed://",
    "reqspeed://",
    "reqheaders://",
    "resheaders://",
    "delete://",
    "urlparams://",
    "reqreplace://",
    "resreplace://",
    "enable://",
    "tlsintercept://",
    "ignore://",
    "http://",
    "https://",
];

/// Returns true when the text appears to contain Whistle-style rules.
pub fn looks_like_whistle(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.starts_with('<') || trimmed.contains("[[profiles]]") {
        return false;
    }
    let mut has_whistle_op = false;
    for raw in trimmed.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("```") {
            return true;
        }
        if parse_single_rule(line).is_some() {
            continue;
        }
        let tokens = split_whistle_tokens(line);
        if tokens.len() >= 2 && tokens.iter().any(|t| is_whistle_operator(t)) {
            has_whistle_op = true;
            break;
        }
    }
    has_whistle_op
}

/// Converts Whistle rule text into `zniff` rules and a conversion summary.
pub fn convert_whistle_rules(text: &str, source_name: &str) -> RuleImportReport {
    let (blocks, rule_lines) = extract_named_blocks(text);
    let mut out_lines = Vec::new();
    let mut bypass_seen = HashSet::new();
    let mut bypass_hosts = Vec::new();
    let mut converted = 0usize;
    let mut unmapped = 0usize;
    let mut skipped_comments = 0usize;

    for raw in rule_lines {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            skipped_comments += 1;
            out_lines.push(trimmed.to_string());
            continue;
        }
        let body = strip_inline_comment(trimmed).trim();
        if body.is_empty() {
            skipped_comments += 1;
            continue;
        }
        let tokens = split_whistle_tokens(body);
        if tokens.len() < 2 {
            out_lines.push(format!("# [unmapped whistle] {trimmed}"));
            unmapped += 1;
            continue;
        }
        let (pat_raw, op_tokens) = order_pattern_and_ops(&tokens);
        let Some(pattern) = normalize_whistle_pattern(pat_raw) else {
            out_lines.push(format!("# [unmapped whistle] {trimmed}"));
            unmapped += 1;
            continue;
        };

        let mut line_emitted = 0usize;
        let mut line_failed = false;
        for op_tok in op_tokens {
            let expanded = expand_block_ref(op_tok, &blocks);
            match translate_operator(&pattern, &expanded) {
                OpOutcome::Rules(lines) if !lines.is_empty() => {
                    for line in lines {
                        if parse_single_rule(&line).is_some() {
                            out_lines.push(line);
                            line_emitted += 1;
                        } else {
                            line_failed = true;
                        }
                    }
                }
                OpOutcome::Bypass(host) => {
                    if bypass_seen.insert(host.clone()) {
                        bypass_hosts.push(host.clone());
                    }
                    out_lines.push(format!("# [bypass] {host}"));
                    line_emitted += 1;
                }
                _ => {
                    line_failed = true;
                }
            }
        }
        converted += line_emitted;
        if line_failed {
            out_lines.push(format!("# [unmapped whistle] {trimmed}"));
            unmapped += 1;
        }
    }

    RuleImportReport {
        source_name: source_name.to_string(),
        rules_text: out_lines.join("\n"),
        bypass_hosts,
        converted,
        unmapped,
        skipped_comments,
    }
}

enum OpOutcome {
    Rules(Vec<String>),
    Bypass(String),
    Unmapped,
}

fn extract_named_blocks(text: &str) -> (HashMap<String, String>, Vec<String>) {
    let mut blocks = HashMap::new();
    let mut lines = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;

    for raw in text.lines() {
        let trimmed = raw.trim();
        if let Some((name, mut buf)) = current.take() {
            if trimmed == "```" {
                blocks.insert(name, buf.join("\n"));
            } else {
                buf.push(raw.to_string());
                current = Some((name, buf));
            }
            continue;
        }
        if let Some(after) = trimmed.strip_prefix("```") {
            let name = after.trim();
            if !name.is_empty() {
                current = Some((name.to_string(), Vec::new()));
                continue;
            }
        }
        lines.push(raw.to_string());
    }
    if let Some((name, buf)) = current {
        blocks.insert(name, buf.join("\n"));
    }
    (blocks, lines)
}

fn strip_inline_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_quote: Option<u8> = None;
    let mut paren_depth = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = in_quote {
            if b == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == q {
                in_quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            in_quote = Some(b);
        } else if b == b'(' {
            paren_depth += 1;
        } else if b == b')' {
            paren_depth = paren_depth.saturating_sub(1);
        } else if b == b'#' && paren_depth == 0 && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            return &line[..i];
        }
        i += 1;
    }
    line
}

fn split_whistle_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_quote: Option<char> = None;
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut escaped = false;

    for ch in line.chars() {
        if escaped {
            cur.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && in_quote.is_some() {
            cur.push(ch);
            escaped = true;
            continue;
        }
        if let Some(q) = in_quote {
            cur.push(ch);
            if ch == q {
                in_quote = None;
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            cur.push(ch);
            in_quote = Some(ch);
        } else if ch == '{' {
            brace_depth += 1;
            cur.push(ch);
        } else if ch == '}' {
            brace_depth = brace_depth.saturating_sub(1);
            cur.push(ch);
        } else if ch == '(' {
            paren_depth += 1;
            cur.push(ch);
        } else if ch == ')' {
            paren_depth = paren_depth.saturating_sub(1);
            cur.push(ch);
        } else if ch.is_whitespace() && brace_depth == 0 && paren_depth == 0 {
            if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn is_whistle_operator(tok: &str) -> bool {
    let lower = tok.to_ascii_lowercase();
    WHISTLE_PREFIXES.iter().any(|p| lower.starts_with(p)) || is_ip_target(tok)
}

fn is_ip_target(tok: &str) -> bool {
    if tok.parse::<IpAddr>().is_ok() {
        return true;
    }
    if let Some(rest) = tok.strip_prefix('[')
        && let Some((ip_str, after)) = rest.split_once(']')
    {
        return ip_str.parse::<IpAddr>().is_ok()
            && (after.is_empty()
                || after
                    .strip_prefix(':')
                    .is_some_and(|p| p.parse::<u16>().is_ok_and(|n| n > 0)));
    }
    if let Some((ip_str, port_str)) = tok.rsplit_once(':') {
        return ip_str.parse::<IpAddr>().is_ok() && port_str.parse::<u16>().is_ok_and(|n| n > 0);
    }
    false
}

fn order_pattern_and_ops(tokens: &[String]) -> (&str, Vec<&str>) {
    if tokens.len() >= 2 && is_whistle_operator(&tokens[0]) && !is_whistle_operator(&tokens[1]) {
        (&tokens[1], vec![tokens[0].as_str()])
    } else {
        (&tokens[0], tokens[1..].iter().map(String::as_str).collect())
    }
}

pub(crate) fn normalize_whistle_pattern(raw: &str) -> Option<String> {
    let mut s = raw.trim();
    if s.is_empty() || s.starts_with('/') {
        return None;
    }
    s = s.strip_prefix('^').unwrap_or(s);
    for scheme in ["https://", "http://", "*://", "wss://", "ws://"] {
        if s.len() >= scheme.len() && s[..scheme.len()].eq_ignore_ascii_case(scheme) {
            s = &s[scheme.len()..];
            break;
        }
    }
    if s.ends_with('$') {
        s = &s[..s.len() - 1];
    }
    for tail in ["/**", "/*"] {
        if let Some(prefix) = s.strip_suffix(tail)
            && !prefix.is_empty()
        {
            s = prefix;
        }
    }
    let s = s.trim();
    if s.is_empty() || s.contains(char::is_whitespace) {
        return None;
    }
    Some(s.to_string())
}

fn expand_block_ref(op_tok: &str, blocks: &HashMap<String, String>) -> String {
    let Some((proto, val)) = op_tok.split_once("://") else {
        return op_tok.to_string();
    };
    let trimmed = val.trim();
    if let Some(key) = trimmed.strip_prefix('{').and_then(|s| s.strip_suffix('}'))
        && let Some(body) = blocks.get(key.trim())
    {
        return format!("{proto}://{body}");
    }
    if let Some(inner) = trimmed.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        return format!("{proto}://{inner}");
    }
    if let Some(inner) = trimmed.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return format!("{proto}://{inner}");
    }
    op_tok.to_string()
}

fn translate_operator(pattern: &str, op: &str) -> OpOutcome {
    if is_ip_target(op) {
        if pattern.contains('/') {
            return OpOutcome::Unmapped;
        }
        return OpOutcome::Rules(vec![format!("resolve {pattern} {op}")]);
    }
    let Some((raw_scheme, raw_val)) = op.split_once("://") else {
        return OpOutcome::Unmapped;
    };
    let scheme = raw_scheme.to_ascii_lowercase();
    let val = raw_val.trim();
    match scheme.as_str() {
        "http" | "https" => OpOutcome::Rules(vec![format!("map-remote {pattern} {op}")]),
        "file" | "rawfile" => {
            if val.is_empty() {
                OpOutcome::Unmapped
            } else {
                OpOutcome::Rules(vec![format!(
                    "map-local {pattern} {}",
                    quote_rule_token(val)
                )])
            }
        }
        "xfile" | "xrawfile" => {
            if val.is_empty() {
                OpOutcome::Unmapped
            } else {
                OpOutcome::Rules(vec![format!(
                    "map-local {pattern} {} fallback",
                    quote_rule_token(val)
                )])
            }
        }
        "statuscode" | "replacestatus" => match val.parse::<u16>() {
            Ok(code) if (100..600).contains(&code) => {
                OpOutcome::Rules(vec![format!("map-local {pattern} status={code}")])
            }
            _ => OpOutcome::Unmapped,
        },
        "resdelay" | "reqdelay" => match val.trim_end_matches("ms").parse::<u64>() {
            Ok(ms) if ms > 0 => OpOutcome::Rules(vec![format!("delay {pattern} {ms}ms")]),
            _ => OpOutcome::Unmapped,
        },
        "rescors" => {
            if val == "*" || val.eq_ignore_ascii_case("enable") || val.eq_ignore_ascii_case("true")
            {
                OpOutcome::Rules(vec![format!("cors {pattern}")])
            } else {
                OpOutcome::Unmapped
            }
        }
        "disable" => match val.to_ascii_lowercase().as_str() {
            "cache" => OpOutcome::Rules(vec![format!("no-cache {pattern}")]),
            "intercept" | "https" | "capture" => {
                OpOutcome::Bypass(pattern.split('/').next().unwrap_or(pattern).to_string())
            }
            _ => OpOutcome::Unmapped,
        },
        "ignore" => {
            if val.eq_ignore_ascii_case("intercept") || val.eq_ignore_ascii_case("https") {
                OpOutcome::Bypass(pattern.split('/').next().unwrap_or(pattern).to_string())
            } else {
                OpOutcome::Unmapped
            }
        }
        "enable" if val.eq_ignore_ascii_case("abort") => {
            OpOutcome::Rules(vec![format!("block {pattern} drop")])
        }
        "resspeed" => parse_speed(val).map_or(OpOutcome::Unmapped, |kb| {
            OpOutcome::Rules(vec![format!("throttle {pattern} down={kb}kb/s")])
        }),
        "reqspeed" => parse_speed(val).map_or(OpOutcome::Unmapped, |kb| {
            OpOutcome::Rules(vec![format!("throttle {pattern} up={kb}kb/s")])
        }),
        "reqheaders" => translate_header_pairs(pattern, "req", val),
        "resheaders" => translate_header_pairs(pattern, "res", val),
        "urlparams" => translate_query_pairs(pattern, val),
        "reqreplace" => translate_replace_pairs(pattern, "req", val),
        "resreplace" => translate_replace_pairs(pattern, "res", val),
        "delete" => translate_delete_op(pattern, val),
        _ => OpOutcome::Unmapped,
    }
}

fn parse_speed(val: &str) -> Option<u64> {
    let cleaned = val
        .trim()
        .trim_end_matches("kb/s")
        .trim_end_matches("kb")
        .trim_end_matches('k')
        .trim();
    cleaned.parse::<u64>().ok().filter(|&n| n > 0)
}

fn parse_kv_entries(raw: &str, prefer_colon: bool) -> Vec<(String, String)> {
    let trimmed = raw.trim();
    if trimmed.starts_with('{')
        && let Ok(Value::Object(map)) = serde_json::from_str::<Value>(trimmed)
    {
        return map
            .into_iter()
            .map(|(k, v)| {
                let s = match v {
                    Value::String(s) => s,
                    other => other.to_string(),
                };
                (k, s)
            })
            .collect();
    }
    let sep_lines: Vec<&str> = if trimmed.contains('\n') {
        trimmed.lines().collect()
    } else if !prefer_colon && trimmed.contains('&') {
        trimmed.split('&').collect()
    } else {
        vec![trimmed]
    };
    let mut out = Vec::new();
    for line in sep_lines {
        let item = line.trim();
        if item.is_empty() || item.starts_with('#') {
            continue;
        }
        let split = if prefer_colon {
            item.split_once(':').or_else(|| item.split_once('='))
        } else {
            item.split_once('=').or_else(|| item.split_once(':'))
        };
        if let Some((k, v)) = split {
            let key = k.trim();
            if !key.is_empty() {
                out.push((key.to_string(), v.trim().to_string()));
            }
        }
    }
    out
}

fn translate_header_pairs(pattern: &str, side: &str, raw: &str) -> OpOutcome {
    let pairs = parse_kv_entries(raw, true);
    if pairs.is_empty() {
        return OpOutcome::Unmapped;
    }
    let rules = pairs
        .into_iter()
        .map(|(k, v)| format!("header {pattern} {side} set {} {}", k, quote_rule_token(&v)))
        .collect();
    OpOutcome::Rules(rules)
}

fn translate_query_pairs(pattern: &str, raw: &str) -> OpOutcome {
    let pairs = parse_kv_entries(raw, false);
    if pairs.is_empty() {
        return OpOutcome::Unmapped;
    }
    let rules = pairs
        .into_iter()
        .map(|(k, v)| format!("query {pattern} set {} {}", k, quote_rule_token(&v)))
        .collect();
    OpOutcome::Rules(rules)
}

fn translate_replace_pairs(pattern: &str, side: &str, raw: &str) -> OpOutcome {
    let pairs = parse_kv_entries(raw, true);
    if pairs.is_empty() {
        return OpOutcome::Unmapped;
    }
    let rules = pairs
        .into_iter()
        .map(|(from, to)| {
            format!(
                "replace {pattern} {side} {} -> {}",
                quote_rule_token(&from),
                quote_rule_token(&to)
            )
        })
        .collect();
    OpOutcome::Rules(rules)
}

fn translate_delete_op(pattern: &str, val: &str) -> OpOutcome {
    let mut out = Vec::new();
    for item in val
        .split([',', '|'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Some(name) = item.strip_prefix("reqHeaders.") {
            out.push(format!("header {pattern} req del {name}"));
        } else if let Some(name) = item.strip_prefix("resHeaders.") {
            out.push(format!("header {pattern} res del {name}"));
        } else if let Some(name) = item.strip_prefix("urlParams.") {
            out.push(format!("query {pattern} del {name}"));
        } else {
            return OpOutcome::Unmapped;
        }
    }
    if out.is_empty() {
        OpOutcome::Unmapped
    } else {
        OpOutcome::Rules(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_common_whistle_rules_and_preserves_unmapped() {
        let input = r#"
# Dev routing
api.vicanso.dev 127.0.0.1:8080
127.0.0.1 static.vicanso.dev
https://gw.vicanso.dev/api http://127.0.0.1:3000/api
cdn.vicanso.dev/mock file://(/tmp/My Mocks/ok.json)
cdn.vicanso.dev/assets xfile:///tmp/dist
api.vicanso.dev/err statusCode://503
api.vicanso.dev/slow resDelay://1500
api.vicanso.dev resCors://* disable://cache
api.vicanso.dev/limit resSpeed://200
api.vicanso.dev/abort enable://abort
pin.vicanso.dev disable://intercept
/regex-path/ resDelay://100
"#;
        assert!(looks_like_whistle(input));
        let report = convert_whistle_rules(input, "Whistle");
        assert_eq!(report.unmapped, 1, "{}", report.rules_text);
        assert!(report.converted >= 11, "{}", report.rules_text);
        assert_eq!(report.bypass_hosts, vec!["pin.vicanso.dev"]);
        assert!(
            report
                .rules_text
                .contains("resolve api.vicanso.dev 127.0.0.1:8080")
        );
        assert!(
            report
                .rules_text
                .contains("resolve static.vicanso.dev 127.0.0.1")
        );
        assert!(
            report
                .rules_text
                .contains("map-remote gw.vicanso.dev/api http://127.0.0.1:3000/api")
        );
        assert!(
            report
                .rules_text
                .contains("map-local cdn.vicanso.dev/mock \"/tmp/My Mocks/ok.json\"")
        );
        assert!(
            report
                .rules_text
                .contains("map-local cdn.vicanso.dev/assets /tmp/dist fallback")
        );
        assert!(
            report
                .rules_text
                .contains("map-local api.vicanso.dev/err status=503")
        );
        assert!(
            report
                .rules_text
                .contains("delay api.vicanso.dev/slow 1500ms")
        );
        assert!(report.rules_text.contains("cors api.vicanso.dev"));
        assert!(report.rules_text.contains("no-cache api.vicanso.dev"));
        assert!(
            report
                .rules_text
                .contains("throttle api.vicanso.dev/limit down=200kb/s")
        );
        assert!(
            report
                .rules_text
                .contains("block api.vicanso.dev/abort drop")
        );
        assert!(
            report
                .rules_text
                .contains("# [unmapped whistle] /regex-path/ resDelay://100")
        );
    }

    #[test]
    fn expands_named_blocks_and_inline_ops() {
        let input = r#"
```hdrs
X-Env: staging
X-Tag: hello world
```
api.vicanso.dev/v1 reqHeaders://{hdrs}
api.vicanso.dev/v1 resReplace://({"ok":false}={"ok":true})
api.vicanso.dev/v1 urlParams://(env=dev&debug=1)
api.vicanso.dev/v1 delete://reqHeaders.Cookie,urlParams.utm
"#;
        let report = convert_whistle_rules(input, "Whistle");
        assert_eq!(report.unmapped, 0, "{}", report.rules_text);
        assert_eq!(report.converted, 7, "{}", report.rules_text);
        assert!(
            report
                .rules_text
                .contains("header api.vicanso.dev/v1 req set X-Env staging")
        );
        assert!(
            report
                .rules_text
                .contains("header api.vicanso.dev/v1 req set X-Tag \"hello world\"")
        );
        assert!(
            report
                .rules_text
                .contains("query api.vicanso.dev/v1 set env dev")
        );
        assert!(
            report
                .rules_text
                .contains("header api.vicanso.dev/v1 req del Cookie")
        );
        assert!(
            report
                .rules_text
                .contains("query api.vicanso.dev/v1 del utm")
        );
    }

    #[test]
    fn does_not_misidentify_zniff_rules_or_toml() {
        assert!(!looks_like_whistle("delay api.vicanso.dev 200ms"));
        assert!(!looks_like_whistle("[[profiles]]\nname = \"dev\""));
        assert!(!looks_like_whistle("<charles-export></charles-export>"));
    }
}
