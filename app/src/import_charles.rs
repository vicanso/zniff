//! Charles Proxy XML rule and `.chlsj` JSON session importer.
//!
//! Pure-Rust zero-dependency XML tree parser and rule translator for Map Local,
//! Map Remote, Rewrite, Breakpoints, DNS Spoofing, No Caching, and Throttling,
//! plus Charles `.chlsj` JSON session ingestion.

#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(not(target_arch = "wasm32"))]
use serde_json::Value;

use crate::import_whistle::RuleImportReport;
use crate::rules::{parse_single_rule, quote_rule_token};

#[derive(Debug, Default)]
struct XmlNode {
    tag: String,
    attrs: Vec<(String, String)>,
    children: Vec<XmlNode>,
    text: String,
}

impl XmlNode {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn child(&self, name: &str) -> Option<&XmlNode> {
        self.children
            .iter()
            .find(|c| c.tag.eq_ignore_ascii_case(name))
    }

    fn child_text(&self, name: &str) -> Option<String> {
        if let Some(child) = self.child(name) {
            let t = child.text.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
        self.attr(name)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }

    fn bool_flag(&self, name: &str, default: bool) -> bool {
        match self.child_text(name) {
            Some(v) => !v.eq_ignore_ascii_case("false") && v != "0",
            None => default,
        }
    }
}

/// Returns true when `text` looks like a Charles XML export document.
pub fn looks_like_charles_xml(text: &str) -> bool {
    let trimmed = text.trim_start();
    if !trimmed.starts_with('<') {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    [
        "<charles-export",
        "<maplocal",
        "<map>",
        "<mapmapping",
        "<rewrite",
        "<breakpoints",
        "<breakpoint",
        "<dnsspoofing",
        "<nocaching",
        "<throttle",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// Converts Charles XML rule exports into `zniff` rules.
pub fn convert_charles_xml(xml: &str, source_name: &str) -> RuleImportReport {
    let roots = parse_xml_nodes(xml);
    let mut ctx = ConvertCtx::default();
    for node in &roots {
        walk_charles_node(node, &mut ctx);
    }
    RuleImportReport {
        source_name: source_name.to_string(),
        rules_text: ctx.lines.join("\n"),
        bypass_hosts: Vec::new(),
        converted: ctx.converted,
        unmapped: ctx.unmapped,
        skipped_comments: ctx.disabled,
    }
}

#[derive(Default)]
struct ConvertCtx {
    lines: Vec<String>,
    converted: usize,
    unmapped: usize,
    disabled: usize,
}

impl ConvertCtx {
    fn push_rule(&mut self, rule: String, enabled: bool, raw_label: &str) {
        if parse_single_rule(&rule).is_some() {
            if enabled {
                self.lines.push(rule);
                self.converted += 1;
            } else {
                self.lines.push(format!("# {rule}"));
                self.disabled += 1;
            }
        } else {
            self.lines.push(format!("# [unmapped charles] {raw_label}"));
            self.unmapped += 1;
        }
    }

    fn push_unmapped(&mut self, desc: &str) {
        self.lines.push(format!("# [unmapped charles] {desc}"));
        self.unmapped += 1;
    }
}

fn walk_charles_node(node: &XmlNode, ctx: &mut ConvertCtx) {
    let tag = node.tag.to_ascii_lowercase();
    match tag.as_str() {
        "maplocalmapping" => {
            convert_map_local(node, ctx);
            return;
        }
        "mapmapping" => {
            convert_map_remote(node, ctx);
            return;
        }
        "rewriteset" => {
            convert_rewrite_set(node, ctx);
            return;
        }
        "breakpoint" => {
            convert_breakpoint(node, ctx);
            return;
        }
        "dnsspoofingmapping" => {
            convert_dns_spoof(node, ctx);
            return;
        }
        "nocaching" => {
            convert_no_caching(node, ctx);
            return;
        }
        "throttle" => {
            convert_throttle(node, ctx);
            return;
        }
        _ => {}
    }
    for child in &node.children {
        walk_charles_node(child, ctx);
    }
}

fn location_pattern(loc: &XmlNode) -> Option<String> {
    let host = loc
        .child_text("host")
        .unwrap_or_else(|| "*".to_string())
        .trim()
        .to_string();
    let host = if host.is_empty() {
        "*".to_string()
    } else {
        host
    };
    if host.contains(char::is_whitespace) {
        return None;
    }
    let path = loc.child_text("path").unwrap_or_default();
    let path = path.trim();
    if path.is_empty() || path == "/" || path == "*" {
        Some(host)
    } else if path.contains(char::is_whitespace) {
        None
    } else if path.starts_with('/') {
        Some(format!("{host}{path}"))
    } else {
        Some(format!("{host}/{path}"))
    }
}

fn convert_map_local(node: &XmlNode, ctx: &mut ConvertCtx) {
    let enabled = node.bool_flag("enabled", true);
    let Some(loc) = node.child("sourceLocation") else {
        ctx.push_unmapped("mapLocalMapping without sourceLocation");
        return;
    };
    let Some(pat) = location_pattern(loc) else {
        ctx.push_unmapped("mapLocalMapping invalid pattern");
        return;
    };
    let Some(dest) = node.child_text("dest") else {
        ctx.push_unmapped(&format!("mapLocal {pat} missing dest"));
        return;
    };
    let rule = format!("map-local {pat} {}", quote_rule_token(&dest));
    ctx.push_rule(rule, enabled, &format!("mapLocal {pat}"));
}

fn convert_map_remote(node: &XmlNode, ctx: &mut ConvertCtx) {
    let enabled = node.bool_flag("enabled", true);
    let (Some(src), Some(dst)) = (node.child("sourceLocation"), node.child("destLocation")) else {
        ctx.push_unmapped("mapMapping missing sourceLocation or destLocation");
        return;
    };
    let Some(pat) = location_pattern(src) else {
        ctx.push_unmapped("mapMapping invalid source pattern");
        return;
    };
    let Some(dest_host) = dst.child_text("host") else {
        ctx.push_unmapped(&format!("mapRemote {pat} missing dest host"));
        return;
    };
    let proto = dst
        .child_text("protocol")
        .unwrap_or_else(|| "https".to_string())
        .to_ascii_lowercase();
    let port_part = dst
        .child_text("port")
        .filter(|p| p.parse::<u16>().is_ok_and(|n| n > 0))
        .map(|p| format!(":{p}"))
        .unwrap_or_default();
    let dest_path = dst.child_text("path").unwrap_or_default();
    let clean_path = if dest_path.is_empty() {
        String::new()
    } else if dest_path.starts_with('/') {
        dest_path
    } else {
        format!("/{dest_path}")
    };
    let target = format!("{proto}://{dest_host}{port_part}{clean_path}");
    ctx.push_rule(
        format!("map-remote {pat} {target}"),
        enabled,
        &format!("mapRemote {pat}"),
    );
}

fn convert_rewrite_set(set: &XmlNode, ctx: &mut ConvertCtx) {
    let set_enabled = set.bool_flag("active", true) && set.bool_flag("enabled", true);
    let set_name = set.child_text("name").unwrap_or_else(|| "rewrite".into());
    let mut patterns = Vec::new();
    if let Some(locs) = set.child("locations") {
        for lm in locs
            .children
            .iter()
            .filter(|c| c.tag.eq_ignore_ascii_case("locationMatch"))
        {
            if lm.bool_flag("enabled", true)
                && let Some(loc) = lm.child("location")
                && let Some(pat) = location_pattern(loc)
            {
                patterns.push(pat);
            }
        }
    }
    if patterns.is_empty() {
        patterns.push("*".to_string());
    }
    let Some(rules_node) = set.child("rules") else {
        return;
    };
    for rule_node in rules_node
        .children
        .iter()
        .filter(|c| c.tag.eq_ignore_ascii_case("rewriteRule"))
    {
        let rule_enabled = set_enabled && rule_node.bool_flag("enabled", true);
        let rtype = rule_node
            .child_text("ruleType")
            .unwrap_or_default()
            .to_ascii_uppercase();
        let req_side = rule_node.bool_flag("request", false)
            || rule_node
                .child_text("where")
                .is_some_and(|w| w.eq_ignore_ascii_case("REQUEST"));
        let res_side = rule_node.bool_flag("response", false)
            || rule_node
                .child_text("where")
                .is_some_and(|w| w.eq_ignore_ascii_case("RESPONSE"));
        let sides: Vec<&str> = match (req_side, res_side) {
            (true, false) => vec!["req"],
            (false, true) => vec!["res"],
            (true, true) => vec!["req", "res"],
            (false, false) => vec!["res"],
        };
        for pat in &patterns {
            emit_rewrite_rule(pat, &rtype, &sides, rule_node, rule_enabled, &set_name, ctx);
        }
    }
}

fn emit_rewrite_rule(
    pat: &str,
    rtype: &str,
    sides: &[&str],
    node: &XmlNode,
    enabled: bool,
    set_name: &str,
    ctx: &mut ConvertCtx,
) {
    match rtype {
        "ADD_HEADER" | "MODIFY_HEADER" => {
            let name = node
                .child_text("newName")
                .or_else(|| node.child_text("matchName"));
            let val = node.child_text("newValue").unwrap_or_default();
            if let Some(name) = name {
                for side in sides {
                    let rule = format!("header {pat} {side} set {name} {}", quote_rule_token(&val));
                    ctx.push_rule(rule, enabled, set_name);
                }
            } else {
                ctx.push_unmapped(&format!("{set_name}:{rtype}"));
            }
        }
        "REMOVE_HEADER" => {
            if let Some(name) = node
                .child_text("matchName")
                .or_else(|| node.child_text("newName"))
            {
                for side in sides {
                    ctx.push_rule(format!("header {pat} {side} del {name}"), enabled, set_name);
                }
            } else {
                ctx.push_unmapped(&format!("{set_name}:{rtype}"));
            }
        }
        "ADD_QUERY_PARAM" | "MODIFY_QUERY_PARAM" => {
            let name = node
                .child_text("newName")
                .or_else(|| node.child_text("matchName"));
            let val = node.child_text("newValue").unwrap_or_default();
            if let Some(name) = name {
                ctx.push_rule(
                    format!("query {pat} set {name} {}", quote_rule_token(&val)),
                    enabled,
                    set_name,
                );
            } else {
                ctx.push_unmapped(&format!("{set_name}:{rtype}"));
            }
        }
        "REMOVE_QUERY_PARAM" => {
            if let Some(name) = node
                .child_text("matchName")
                .or_else(|| node.child_text("newName"))
            {
                ctx.push_rule(format!("query {pat} del {name}"), enabled, set_name);
            } else {
                ctx.push_unmapped(&format!("{set_name}:{rtype}"));
            }
        }
        "BODY" => {
            let from = node.child_text("matchValue").unwrap_or_default();
            let to = node.child_text("newValue").unwrap_or_default();
            if from.is_empty() {
                ctx.push_unmapped(&format!("{set_name}:BODY (empty matchValue)"));
            } else {
                for side in sides {
                    let rule = format!(
                        "replace {pat} {side} {} -> {}",
                        quote_rule_token(&from),
                        quote_rule_token(&to)
                    );
                    ctx.push_rule(rule, enabled, set_name);
                }
            }
        }
        _ => ctx.push_unmapped(&format!("{set_name}:{rtype}")),
    }
}

fn convert_breakpoint(node: &XmlNode, ctx: &mut ConvertCtx) {
    let enabled = node.bool_flag("enabled", true);
    let Some(loc) = node.child("location") else {
        return;
    };
    let Some(pat) = location_pattern(loc) else {
        ctx.push_unmapped("breakpoint invalid location");
        return;
    };
    let req = node.bool_flag("request", true);
    let res = node.bool_flag("response", false);
    if req {
        ctx.push_rule(format!("break {pat}"), enabled, &format!("break {pat}"));
    }
    if res {
        ctx.push_rule(
            format!("break-res {pat}"),
            enabled,
            &format!("break-res {pat}"),
        );
    }
}

fn convert_dns_spoof(node: &XmlNode, ctx: &mut ConvertCtx) {
    let enabled = node.bool_flag("enabled", true);
    let host = node.child_text("name").or_else(|| node.child_text("host"));
    let addr = node.child_text("address");
    match (host, addr) {
        (Some(h), Some(a)) => {
            ctx.push_rule(format!("resolve {h} {a}"), enabled, &format!("dns {h} {a}"))
        }
        _ => ctx.push_unmapped("dnsSpoofingMapping missing host/address"),
    }
}

fn convert_no_caching(node: &XmlNode, ctx: &mut ConvertCtx) {
    let enabled = node.bool_flag("enabled", true);
    for child in &node.children {
        if child.tag.eq_ignore_ascii_case("location")
            && let Some(pat) = location_pattern(child)
        {
            ctx.push_rule(
                format!("no-cache {pat}"),
                enabled,
                &format!("no-cache {pat}"),
            );
        } else {
            convert_no_caching(child, ctx);
        }
    }
}

fn convert_throttle(node: &XmlNode, ctx: &mut ConvertCtx) {
    let enabled = node.bool_flag("active", false) || node.bool_flag("enabled", false);
    let down = node
        .child_text("bandwidthDown")
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&v| v > 0);
    let up = node
        .child_text("bandwidthUp")
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&v| v > 0);
    let rule = match (down, up) {
        (Some(d), Some(u)) => format!("throttle * down={d}kbps up={u}kbps"),
        (Some(d), None) => format!("throttle * down={d}kbps"),
        (None, Some(u)) => format!("throttle * up={u}kbps"),
        (None, None) => return,
    };
    ctx.push_rule(rule, enabled, "throttle");
}

fn parse_xml_nodes(xml: &str) -> Vec<XmlNode> {
    let mut stack: Vec<XmlNode> = vec![XmlNode::default()];
    let bytes = xml.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if xml[i..].starts_with("<!--") {
                if let Some(end) = xml[i + 4..].find("-->") {
                    i += 4 + end + 3;
                    continue;
                }
                break;
            }
            if xml[i..].starts_with("<![CDATA[") {
                if let Some(end) = xml[i + 9..].find("]]>") {
                    if let Some(top) = stack.last_mut() {
                        top.text.push_str(&xml[i + 9..i + 9 + end]);
                    }
                    i += 9 + end + 3;
                    continue;
                }
                break;
            }
            if xml[i..].starts_with("<?") {
                if let Some(end) = xml[i + 2..].find("?>") {
                    i += 2 + end + 2;
                    continue;
                }
                break;
            }
            let Some(rel_end) = find_tag_end(xml, i + 1) else {
                break;
            };
            let raw_tag = xml[i + 1..rel_end].trim();
            i = rel_end + 1;
            if raw_tag.is_empty() || raw_tag.starts_with('!') {
                continue;
            }
            if let Some(closing) = raw_tag.strip_prefix('/') {
                let closing_name = closing.trim();
                while stack.len() > 1 {
                    let popped = stack.pop().expect("len > 1");
                    let matches = popped.tag.eq_ignore_ascii_case(closing_name);
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(popped);
                    }
                    if matches {
                        break;
                    }
                }
                continue;
            }
            let self_closing = raw_tag.ends_with('/');
            let header = raw_tag.strip_suffix('/').unwrap_or(raw_tag).trim();
            let (tag_name, attrs) = parse_tag_name_and_attrs(header);
            let node = XmlNode {
                tag: tag_name,
                attrs,
                children: Vec::new(),
                text: String::new(),
            };
            if self_closing {
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                }
            } else {
                stack.push(node);
            }
        } else {
            let next = xml[i..].find('<').map_or(xml.len(), |off| i + off);
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&unescape_xml(&xml[i..next]));
            }
            i = next;
        }
    }
    while stack.len() > 1 {
        let popped = stack.pop().expect("len > 1");
        if let Some(parent) = stack.last_mut() {
            parent.children.push(popped);
        }
    }
    stack.into_iter().next().unwrap_or_default().children
}

fn find_tag_end(xml: &str, start: usize) -> Option<usize> {
    let mut in_quote: Option<u8> = None;
    let bytes = xml.as_bytes();
    for (idx, &b) in bytes[start..].iter().enumerate() {
        if let Some(q) = in_quote {
            if b == q {
                in_quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            in_quote = Some(b);
        } else if b == b'>' {
            return Some(start + idx);
        }
    }
    None
}

fn parse_tag_name_and_attrs(header: &str) -> (String, Vec<(String, String)>) {
    let mut parts = header.splitn(2, char::is_whitespace);
    let tag = parts.next().unwrap_or("").trim().to_string();
    let rest = parts.next().unwrap_or("").trim();
    let mut attrs = Vec::new();
    let mut cur = rest;
    while !cur.is_empty() {
        let Some(eq) = cur.find('=') else {
            break;
        };
        let key = cur[..eq].trim().to_string();
        cur = cur[eq + 1..].trim_start();
        if let Some(q) = cur.chars().next().filter(|c| *c == '"' || *c == '\'') {
            cur = &cur[1..];
            let end = cur.find(q).unwrap_or(cur.len());
            attrs.push((key, unescape_xml(&cur[..end])));
            cur = cur.get(end + 1..).unwrap_or("").trim_start();
        } else {
            let end = cur.find(char::is_whitespace).unwrap_or(cur.len());
            attrs.push((key, unescape_xml(&cur[..end])));
            cur = cur[end..].trim_start();
        }
    }
    (tag, attrs)
}

fn unescape_xml(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut i = 0usize;
    while i < raw.len() {
        if raw.as_bytes()[i] == b'&'
            && let Some(semi) = raw[i + 1..].find(';')
        {
            let entity = &raw[i + 1..i + 1 + semi];
            let decoded = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                    u32::from_str_radix(&entity[2..], 16)
                        .ok()
                        .and_then(char::from_u32)
                }
                _ if entity.starts_with('#') => {
                    entity[1..].parse::<u32>().ok().and_then(char::from_u32)
                }
                _ => None,
            };
            if let Some(ch) = decoded {
                out.push(ch);
                i += semi + 2;
                continue;
            }
        }
        let ch = raw[i..].chars().next().expect("valid utf8 boundary");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Imports a Charles `.chlsj` JSON session array into `zniff` flows.
#[cfg(not(target_arch = "wasm32"))]
pub fn import_charles_chlsj(value: &Value) -> Vec<crate::har::Imported> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items.iter().filter_map(import_chlsj_item).collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn import_chlsj_item(entry: &Value) -> Option<crate::har::Imported> {
    use crate::session::ConnInfo;

    let obj = entry.as_object()?;
    let method = obj
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_string();
    let host = obj.get("host").and_then(Value::as_str).unwrap_or("");
    let port = obj.get("port").and_then(Value::as_u64);
    let scheme = obj
        .get("scheme")
        .and_then(Value::as_str)
        .unwrap_or(if port == Some(443) { "https" } else { "http" });
    let raw_path = obj.get("path").and_then(Value::as_str).unwrap_or("/");
    let query = obj
        .get("query")
        .and_then(Value::as_str)
        .filter(|q| !q.is_empty());
    let path_and_query = match query {
        Some(q) => format!("{raw_path}?{q}"),
        None => raw_path.to_string(),
    };
    let port_suffix = match (scheme, port) {
        ("http", Some(80)) | ("https", Some(443)) | (_, None) => String::new(),
        (_, Some(p)) => format!(":{p}"),
    };
    let url = if host.is_empty() {
        path_and_query.clone()
    } else {
        format!("{scheme}://{host}{port_suffix}{path_and_query}")
    };
    let (parsed_host, parsed_path) = crate::har::split_url(&url);
    let final_host = if parsed_host.is_empty() {
        host.to_string()
    } else {
        parsed_host
    };
    let protocol = obj
        .get("protocolVersion")
        .and_then(Value::as_str)
        .map(crate::har::protocol_from)
        .unwrap_or(crate::session::Protocol::Http11);

    let req = obj.get("request");
    let res = obj.get("response");
    let status = res
        .and_then(|r| r.get("status"))
        .or_else(|| obj.get("status"))
        .and_then(Value::as_u64)
        .and_then(|s| u16::try_from(s).ok())
        .filter(|&s| s > 0);
    let request_headers = chlsj_headers(req);
    let response_headers = chlsj_headers(res);
    let request_body = chlsj_body(req);
    let response_body = chlsj_body(res);

    let times = obj.get("times");
    let started = times
        .and_then(|t| t.get("start"))
        .and_then(parse_chlsj_time)
        .unwrap_or(UNIX_EPOCH);
    let response_at = times
        .and_then(|t| t.get("responseBegin"))
        .and_then(parse_chlsj_time);
    let finished_at = times
        .and_then(|t| t.get("end"))
        .and_then(parse_chlsj_time)
        .or(response_at)
        .or(Some(started));

    let request_bytes = req
        .and_then(|r| r.get("sizes"))
        .and_then(|s| s.get("body"))
        .and_then(Value::as_u64)
        .or_else(|| {
            (!request_body.is_empty()).then_some(u64::try_from(request_body.len()).unwrap_or(0))
        });
    let response_bytes = res
        .and_then(|r| r.get("sizes"))
        .and_then(|s| s.get("body"))
        .and_then(Value::as_u64)
        .or_else(|| {
            (!response_body.is_empty()).then_some(u64::try_from(response_body.len()).unwrap_or(0))
        });

    let mut conn_info = ConnInfo::default();
    if let Some(addr) = obj.get("remoteAddress").and_then(Value::as_str) {
        conn_info.server_addr = addr.to_string();
    }

    Some(crate::har::Imported {
        protocol,
        method,
        host: final_host,
        path: if parsed_path.is_empty() {
            path_and_query
        } else {
            parsed_path
        },
        url,
        status,
        error: None,
        request_headers,
        response_headers,
        request_body,
        response_body,
        started,
        response_at,
        finished_at,
        request_bytes,
        response_bytes,
        comment: obj
            .get("notes")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        conn_info,
        messages: Vec::new(),
        messages_truncated: false,
        message_times: Vec::new(),
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn chlsj_headers(side: Option<&Value>) -> Vec<(String, String)> {
    let Some(arr) = side
        .and_then(|s| s.get("header"))
        .and_then(|h| h.get("headers"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            let name = item.get("name").and_then(Value::as_str)?;
            let value = item.get("value").and_then(Value::as_str).unwrap_or("");
            Some((name.to_string(), value.to_string()))
        })
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn chlsj_body(side: Option<&Value>) -> Vec<u8> {
    let Some(body) = side.and_then(|s| s.get("body")) else {
        return Vec::new();
    };
    if let Some(encoded) = body.get("encoded").and_then(Value::as_str)
        && !encoded.is_empty()
    {
        return crate::har::b64_decode(encoded);
    }
    body.get("text")
        .and_then(Value::as_str)
        .map(|s| s.as_bytes().to_vec())
        .unwrap_or_default()
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_chlsj_time(val: &Value) -> Option<SystemTime> {
    if let Some(s) = val.as_str() {
        return crate::har::parse_iso(s);
    }
    let ms = val.as_u64()?;
    Some(UNIX_EPOCH + Duration::from_millis(ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_charles_xml_rules_across_tools() {
        let xml = r#"<?xml version='1.0' encoding='UTF-8' ?>
<charles-export>
  <mapLocal>
    <mappings>
      <mapLocalMapping>
        <sourceLocation><protocol>https</protocol><host>api.vicanso.dev</host><path>/v1/user</path></sourceLocation>
        <dest>/tmp/My Mocks/user.json</dest>
        <enabled>true</enabled>
      </mapLocalMapping>
      <mapLocalMapping>
        <sourceLocation><host>old.vicanso.dev</host></sourceLocation>
        <dest>/tmp/old.json</dest>
        <enabled>false</enabled>
      </mapLocalMapping>
    </mappings>
  </mapLocal>
  <map>
    <mappings>
      <mapMapping>
        <sourceLocation><host>api.vicanso.dev</host><path>/gw</path></sourceLocation>
        <destLocation><protocol>http</protocol><host>127.0.0.1</host><port>3000</port><path>/gw</path></destLocation>
        <enabled>true</enabled>
      </mapMapping>
    </mappings>
  </map>
  <rewrite>
    <sets>
      <rewriteSet>
        <active>true</active>
        <name>API Rewrite</name>
        <locations><locationMatch><location><host>api.vicanso.dev</host><path>/v1</path></location><enabled>true</enabled></locationMatch></locations>
        <rules>
          <rewriteRule><ruleType>ADD_HEADER</ruleType><where>REQUEST</where><newName>X-From</newName><newValue>charles &amp; co</newValue><enabled>true</enabled></rewriteRule>
          <rewriteRule><ruleType>BODY</ruleType><where>RESPONSE</where><matchValue>"prod"</matchValue><newValue>"dev"</newValue><enabled>true</enabled></rewriteRule>
        </rules>
      </rewriteSet>
    </sets>
  </rewrite>
  <breakpoints>
    <breakpoint><location><host>pay.vicanso.dev</host><path>/checkout</path></location><request>true</request><response>true</response><enabled>true</enabled></breakpoint>
  </breakpoints>
  <dnsSpoofing>
    <mappings><dnsSpoofingMapping><name>api.vicanso.dev</name><address>127.0.0.1</address><enabled>true</enabled></dnsSpoofingMapping></mappings>
  </dnsSpoofing>
  <noCaching>
    <locations><location><host>static.vicanso.dev</host></location></locations>
  </noCaching>
  <throttle>
    <active>true</active>
    <bandwidthDown>512</bandwidthDown>
    <bandwidthUp>256</bandwidthUp>
  </throttle>
</charles-export>"#;
        assert!(looks_like_charles_xml(xml));
        let report = convert_charles_xml(xml, "Charles XML");
        assert_eq!(report.unmapped, 0, "{}", report.rules_text);
        assert_eq!(report.skipped_comments, 1, "{}", report.rules_text);
        assert_eq!(report.converted, 9, "{}", report.rules_text);
        assert!(
            report
                .rules_text
                .contains("map-local api.vicanso.dev/v1/user \"/tmp/My Mocks/user.json\"")
        );
        assert!(
            report
                .rules_text
                .contains("# map-local old.vicanso.dev /tmp/old.json")
        );
        assert!(
            report
                .rules_text
                .contains("map-remote api.vicanso.dev/gw http://127.0.0.1:3000/gw")
        );
        assert!(
            report
                .rules_text
                .contains("header api.vicanso.dev/v1 req set X-From \"charles & co\"")
        );
        assert!(
            report
                .rules_text
                .contains("replace api.vicanso.dev/v1 res \"\\\"prod\\\"\" -> \"\\\"dev\\\"\"")
        );
        assert!(report.rules_text.contains("break pay.vicanso.dev/checkout"));
        assert!(
            report
                .rules_text
                .contains("break-res pay.vicanso.dev/checkout")
        );
        assert!(
            report
                .rules_text
                .contains("resolve api.vicanso.dev 127.0.0.1")
        );
        assert!(report.rules_text.contains("no-cache static.vicanso.dev"));
        assert!(
            report
                .rules_text
                .contains("throttle * down=512kbps up=256kbps")
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn imports_charles_chlsj_json_session() {
        let json = serde_json::json!([
            {
                "status": "COMPLETE",
                "method": "POST",
                "protocolVersion": "HTTP/2",
                "scheme": "https",
                "host": "api.vicanso.dev",
                "port": 443,
                "path": "/v1/orders",
                "query": "dry=1",
                "remoteAddress": "127.0.0.1:443",
                "notes": "from charles",
                "times": {
                    "start": "2026-11-19T08:00:00.000Z",
                    "responseBegin": "2026-11-19T08:00:00.050Z",
                    "end": "2026-11-19T08:00:00.120Z"
                },
                "request": {
                    "sizes": { "body": 10 },
                    "header": { "headers": [{"name": "Content-Type", "value": "application/json"}] },
                    "body": { "text": "{\"id\":123}" }
                },
                "response": {
                    "status": 201,
                    "sizes": { "body": 5 },
                    "header": { "headers": [{"name": "Content-Type", "value": "text/plain"}] },
                    "body": { "encoded": "aGVsbG8=" }
                }
            }
        ]);
        let items = import_charles_chlsj(&json);
        assert_eq!(items.len(), 1);
        let f = &items[0];
        assert_eq!(f.protocol, crate::session::Protocol::Http2);
        assert_eq!(f.method, "POST");
        assert_eq!(f.host, "api.vicanso.dev");
        assert_eq!(f.path, "/v1/orders?dry=1");
        assert_eq!(f.status, Some(201));
        assert_eq!(f.request_body, br#"{"id":123}"#);
        assert_eq!(f.response_body, b"hello");
        assert_eq!(f.comment.as_deref(), Some("from charles"));
        assert_eq!(f.conn_info.server_addr, "127.0.0.1:443");
    }
}
