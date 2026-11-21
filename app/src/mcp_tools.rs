//! Extended MCP tools (`wait_for_flow`, `mock_flow`, `diff_flows`)
//! and body-preview helpers.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::mcp::{flow_summary_json, parse_headers_arg, tool_error, tool_json};
use crate::present::{self, DiffKind};
use crate::proxy::Hub;
use crate::rules;
use crate::session::Flow;

pub(crate) const BODY_PREVIEW_CAP: usize = 32 * 1024;
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 5_000;
const MIN_WAIT_TIMEOUT_MS: u64 = 100;
const MAX_WAIT_TIMEOUT_MS: u64 = 30_000;
const MAX_DIFF_LINES_OUT: usize = 200;

pub(crate) fn extra_tool_schemas() -> Vec<Value> {
    vec![
        json!({
            "name": "wait_for_flow",
            "description": "Asynchronously wait until a captured flow matching the optional filter arrives in the session (optionally strictly after `after_id`), or time out cleanly returning `{ matched: false, timeout_ms }`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filter": {
                        "type": "string",
                        "description": "Optional zniff filter expression (same syntax as `list_flows`)."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "description": "Wait timeout in milliseconds (100..=30000, default 5000)."
                    },
                    "after_id": {
                        "type": "integer",
                        "description": "Optional flow ID threshold; only flows with `id > after_id` will match."
                    }
                }
            }
        }),
        json!({
            "name": "mock_flow",
            "description": "Create and apply a rule (`map-local`, `delay`, or `block`) from an existing captured flow (also callable as `add_rule_from_flow`). For `map-local`, writes response body to `<data_dir>/mocks/` and appends the rule (plus optional `header` rules) to active Hub rules.",
            "inputSchema": {
                "type": "object",
                "required": ["flow_id"],
                "properties": {
                    "flow_id": {
                        "type": "integer",
                        "description": "Source captured flow ID."
                    },
                    "rule_type": {
                        "type": "string",
                        "enum": ["map-local", "delay", "block"],
                        "description": "Rule kind to create (default `map-local`)."
                    },
                    "status": {
                        "type": "integer",
                        "description": "Optional HTTP status code (100..=599). Defaults to the flow's status (or 200) for `map-local`, and 403 for `block`."
                    },
                    "body": {
                        "type": "string",
                        "description": "Optional UTF-8 mock response body for `map-local`. Defaults to the captured flow's response body."
                    },
                    "headers": {
                        "type": ["string", "object"],
                        "description": "Optional response headers (`Name: Value` lines or JSON object) to append as `header <pattern> res set` rules."
                    },
                    "delay": {
                        "type": ["string", "integer"],
                        "description": "Optional delay duration for `delay` rules (e.g. `500ms`, `1s`, or integer milliseconds; default `1000ms`)."
                    }
                }
            }
        }),
        json!({
            "name": "diff_flows",
            "description": "Compute a structured line-level diff between two captured flows across URL/method/status, request headers/body, and response headers/body.",
            "inputSchema": {
                "type": "object",
                "required": ["base_id", "target_id"],
                "properties": {
                    "base_id": {
                        "type": "integer",
                        "description": "Baseline flow ID."
                    },
                    "target_id": {
                        "type": "integer",
                        "description": "Target flow ID to compare against the baseline."
                    }
                }
            }
        }),
    ]
}

fn find_matching_flow_id(hub: &Hub, filter: &str, after_id: Option<u64>) -> Option<u64> {
    let min_id = after_id.unwrap_or(0);
    let (matched, _) = hub.matching_ids(filter);
    matched
        .into_iter()
        .map(|(id, _)| id)
        .find(|&id| id > min_id)
}

fn load_flow_summary(hub: &Hub, id: u64) -> Option<Value> {
    hub.with_session(|session| {
        let flow = session.get(id)?;
        let hydrated = if flow.request_body.is_empty() && flow.request_spill.is_some() {
            session.hydrated(id)?
        } else {
            flow.clone()
        };
        Some(flow_summary_json(&hydrated))
    })
}

pub(crate) async fn tool_wait_for_flow(hub: Arc<Hub>, args: &Value) -> Value {
    let filter = args.get("filter").and_then(Value::as_str).unwrap_or("");
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_WAIT_TIMEOUT_MS)
        .clamp(MIN_WAIT_TIMEOUT_MS, MAX_WAIT_TIMEOUT_MS);
    let after_id = match args.get("after_id") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_u64() {
            Some(id) => Some(id),
            None => return tool_error("`after_id` must be a non-negative integer."),
        },
    };

    let started = tokio::time::Instant::now();
    let deadline = started + Duration::from_millis(timeout_ms);
    let mut rx = hub.subscribe_state();

    loop {
        if let Some(id) = find_matching_flow_id(&hub, filter, after_id)
            && let Some(flow) = load_flow_summary(&hub, id)
        {
            return tool_json(&json!({
                "matched": true,
                "elapsed_ms": started.elapsed().as_millis() as u64,
                "flow": flow,
            }));
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return tool_json(&json!({
                "matched": false,
                "timeout_ms": timeout_ms,
                "filter": filter,
            }));
        }
        match tokio::time::timeout(deadline - now, rx.changed()).await {
            Ok(Ok(())) => continue,
            Ok(Err(_)) | Err(_) => {
                if let Some(id) = find_matching_flow_id(&hub, filter, after_id)
                    && let Some(flow) = load_flow_summary(&hub, id)
                {
                    return tool_json(&json!({
                        "matched": true,
                        "elapsed_ms": started.elapsed().as_millis() as u64,
                        "flow": flow,
                    }));
                }
                return tool_json(&json!({
                    "matched": false,
                    "timeout_ms": timeout_ms,
                    "filter": filter,
                }));
            }
        }
    }
}

fn parse_optional_status(args: &Value) -> Result<Option<u16>, Value> {
    match args.get("status") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let Some(code) = v.as_u64().and_then(|n| u16::try_from(n).ok()) else {
                return Err(tool_error(
                    "`status` must be an integer between 100 and 599.",
                ));
            };
            if !(100..=599).contains(&code) {
                return Err(tool_error(
                    "`status` must be an integer between 100 and 599.",
                ));
            }
            Ok(Some(code))
        }
    }
}

fn infer_mock_extension(flow: &Flow, body_bytes: &[u8]) -> &'static str {
    let ct = present::content_type(&flow.response_headers)
        .unwrap_or("")
        .to_ascii_lowercase();
    if ct.contains("json") {
        "json"
    } else if ct.contains("html") {
        "html"
    } else if ct.contains("xml") {
        "xml"
    } else if ct.contains("javascript") || ct.contains("ecmascript") {
        "js"
    } else if ct.contains("css") {
        "css"
    } else if ct.contains("text/") {
        "txt"
    } else if std::str::from_utf8(body_bytes).is_ok_and(|s| {
        let t = s.trim();
        (t.starts_with('{') && t.ends_with('}')) || (t.starts_with('[') && t.ends_with(']'))
    }) {
        "json"
    } else {
        "txt"
    }
}

fn write_mock_body_file(hub: &Hub, flow: &Flow, body_bytes: &[u8]) -> Result<PathBuf, String> {
    let mocks_dir = hub.ca.dir().join("mocks");
    std::fs::create_dir_all(&mocks_dir)
        .map_err(|err| format!("Failed to create mocks directory: {err}"))?;
    let clean_path = flow.path.split(['?', '#']).next().unwrap_or("");
    let raw_slug = format!("{}_{}", flow.host, clean_path);
    let mut slug = String::new();
    for ch in raw_slug.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('_') {
            slug.push('_');
        }
        if slug.len() >= 48 {
            break;
        }
    }
    let slug = slug.trim_matches('_');
    let ext = infer_mock_extension(flow, body_bytes);
    let filename = if slug.is_empty() {
        format!("flow-{}.{ext}", flow.id)
    } else {
        format!("flow-{}-{slug}.{ext}", flow.id)
    };
    let path = mocks_dir.join(filename);
    std::fs::write(&path, body_bytes)
        .map_err(|err| format!("Failed to write mock file {}: {err}", path.display()))?;
    Ok(path)
}

fn parse_delay_arg(args: &Value) -> Result<String, Value> {
    match args.get("delay") {
        None | Some(Value::Null) => Ok("1000ms".to_string()),
        Some(v) => {
            if let Some(ms) = v.as_u64() {
                Ok(format!("{ms}ms"))
            } else if let Some(s) = v.as_str() {
                let trimmed = s.trim();
                if trimmed.is_empty() || trimmed.contains(char::is_whitespace) {
                    Err(tool_error(
                        "`delay` must be a non-empty duration like `500ms`, `2s`, or integer ms.",
                    ))
                } else {
                    Ok(trimmed.to_string())
                }
            } else {
                Err(tool_error(
                    "`delay` must be a duration string (e.g. `500ms`) or integer milliseconds.",
                ))
            }
        }
    }
}

fn append_rule_line(existing: &str, line: &str) -> String {
    if existing.trim().is_empty() {
        line.to_string()
    } else {
        format!("{}\n{line}", existing.trim_end())
    }
}

fn build_header_rule_lines(
    pattern: &str,
    headers: &[(String, String)],
) -> Result<Vec<String>, Value> {
    let mut lines = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        let name = name.trim();
        let value = value.trim();
        if name.is_empty()
            || name.contains(char::is_whitespace)
            || value.contains('\n')
            || value.contains('\r')
        {
            return Err(tool_error(format!(
                "Invalid header `{name}: {value}` for rule generation."
            )));
        }
        lines.push(format!("header {pattern} res set {name} {value}"));
    }
    Ok(lines)
}

fn build_map_local_rules(
    hub: &Hub,
    flow: &Flow,
    existing: &str,
    status_opt: Option<u16>,
    args: &Value,
) -> Result<(String, Option<String>, Option<String>), Value> {
    let status = status_opt.or(flow.status).unwrap_or(200);
    let explicit_body = args.get("body").and_then(Value::as_str);
    let file_path = if let Some(body_str) = explicit_body {
        Some(write_mock_body_file(hub, flow, body_str.as_bytes()).map_err(tool_error)?)
    } else if !flow.response_body.is_empty() {
        Some(write_mock_body_file(hub, flow, &flow.response_body).map_err(tool_error)?)
    } else {
        None
    };
    let file_str = file_path.as_ref().map(|p| p.to_string_lossy().into_owned());
    let Some(updated) = rules::append_map_local_line(
        existing,
        &flow.host,
        &flow.path,
        file_str.as_deref(),
        Some(status),
    ) else {
        return Err(tool_error(
            "Failed to format `map-local` rule for this flow.",
        ));
    };
    let added = updated.lines().last().map(str::to_string);
    Ok((updated, file_str, added))
}

pub(crate) fn tool_mock_flow(hub: &Hub, args: &Value) -> Value {
    let Some(flow_id) = args
        .get("flow_id")
        .or_else(|| args.get("id"))
        .and_then(Value::as_u64)
    else {
        return tool_error("Missing required integer parameter `flow_id`.");
    };
    let Some(flow) = hub.with_session(|session| session.hydrated(flow_id)) else {
        return tool_error(format!("Flow #{flow_id} not found in session."));
    };
    let Some(pattern) = rules::request_pattern(&flow.host, &flow.path) else {
        return tool_error(format!(
            "Cannot build rule pattern from host `{}` and path `{}`.",
            flow.host, flow.path
        ));
    };
    let rule_type = args
        .get("rule_type")
        .and_then(Value::as_str)
        .unwrap_or("map-local")
        .trim()
        .to_ascii_lowercase();
    let status_opt = match parse_optional_status(args) {
        Ok(s) => s,
        Err(err) => return err,
    };
    let headers_opt = match args.get("headers") {
        None | Some(Value::Null) => None,
        Some(v) => match parse_headers_arg(v) {
            Some(parsed) => Some(parsed),
            None => {
                return tool_error(
                    "`headers` must be a `Name: Value` multiline string or string-valued JSON object.",
                );
            }
        },
    };

    let existing = hub.rules_text();
    let mut added_rules = Vec::new();
    let mut mock_file: Option<String> = None;
    let mut next_rules = match rule_type.as_str() {
        "map-local" => match build_map_local_rules(hub, &flow, &existing, status_opt, args) {
            Ok((updated, file_str, added)) => {
                mock_file = file_str;
                if let Some(line) = added {
                    added_rules.push(line);
                }
                updated
            }
            Err(err) => return err,
        },
        "delay" => {
            let delay_spec = match parse_delay_arg(args) {
                Ok(d) => d,
                Err(err) => return err,
            };
            let line = format!("delay {pattern} {delay_spec}");
            added_rules.push(line.clone());
            append_rule_line(&existing, &line)
        }
        "block" => {
            let code = status_opt.unwrap_or(403);
            let line = format!("block {pattern} status={code}");
            added_rules.push(line.clone());
            append_rule_line(&existing, &line)
        }
        other => {
            return tool_error(format!(
                "Unsupported `rule_type`: `{other}`. Expected one of: map-local, delay, block."
            ));
        }
    };

    if let Some(ref headers) = headers_opt {
        let header_lines = match build_header_rule_lines(&pattern, headers) {
            Ok(lines) => lines,
            Err(err) => return err,
        };
        for line in header_lines {
            next_rules = append_rule_line(&next_rules, &line);
            added_rules.push(line);
        }
    }

    let bad = rules::invalid_rule_lines(&next_rules);
    if !bad.is_empty() {
        return tool_error(format!("Generated rule syntax invalid on line(s): {bad:?}"));
    }

    hub.set_rules(&next_rules);
    let save_res =
        crate::config::update_mode(hub.ca.dir(), crate::config::is_dev_mode(), |config| {
            config.rules = next_rules.clone();
        });
    let current = hub.rules_text();
    let mut resp = json!({
        "ok": true,
        "flow_id": flow.id,
        "rule_type": rule_type,
        "pattern": pattern,
        "added_rules": added_rules,
        "mock_file": mock_file,
        "active_rule_count": rules::parse_rules(&current).len(),
        "effective_rules_count": hub.effective_rules_count(),
        "apply_rules": hub.rules_enabled(),
    });
    if let Err(err) = save_res {
        resp["warning"] = json!(format!("Failed to persist rules to disk: {err}"));
    }
    tool_json(&resp)
}

fn format_headers_for_diff(headers: &[(String, String)]) -> String {
    let mut sorted: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    sorted.sort_by(|a, b| {
        a.0.to_ascii_lowercase()
            .cmp(&b.0.to_ascii_lowercase())
            .then_with(|| a.1.cmp(b.1))
    });
    sorted
        .into_iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_body_for_diff(bytes: &[u8], ct: Option<&str>) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => present::pretty_body(ct, text),
        Err(_) => present::hex_dump_raw(&bytes[..bytes.len().min(4096)]),
    }
}

fn diff_section_json(before: &str, after: &str) -> (bool, Value) {
    let diff = present::line_diff(before, after);
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut unchanged = 0usize;
    let mut lines = Vec::new();
    for item in diff {
        let kind_str = match item.kind {
            DiffKind::Same => {
                unchanged += 1;
                "same"
            }
            DiffKind::Add => {
                added += 1;
                "add"
            }
            DiffKind::Remove => {
                removed += 1;
                "remove"
            }
        };
        if lines.len() < MAX_DIFF_LINES_OUT && item.kind != DiffKind::Same {
            lines.push(json!({
                "kind": kind_str,
                "text": item.text,
            }));
        }
    }
    let changed = added > 0 || removed > 0;
    (
        changed,
        json!({
            "changed": changed,
            "added": added,
            "removed": removed,
            "unchanged": unchanged,
            "lines": lines,
        }),
    )
}

pub(crate) fn tool_diff_flows(hub: &Hub, args: &Value) -> Value {
    let Some(base_id) = args.get("base_id").and_then(Value::as_u64) else {
        return tool_error("Missing required integer parameter `base_id`.");
    };
    let Some(target_id) = args.get("target_id").and_then(Value::as_u64) else {
        return tool_error("Missing required integer parameter `target_id`.");
    };
    let (base, target) =
        hub.with_session(|session| (session.hydrated(base_id), session.hydrated(target_id)));
    let Some(base) = base else {
        return tool_error(format!("Baseline flow #{base_id} not found in session."));
    };
    let Some(target) = target else {
        return tool_error(format!("Target flow #{target_id} not found in session."));
    };

    let base_url_line = format!("{} {}", base.method, base.url);
    let target_url_line = format!("{} {}", target.method, target.url);
    let (url_changed, url_diff) = diff_section_json(&base_url_line, &target_url_line);

    let (req_hdr_changed, req_hdr_diff) = diff_section_json(
        &format_headers_for_diff(&base.request_headers),
        &format_headers_for_diff(&target.request_headers),
    );
    let base_req_body = format_body_for_diff(
        &base.request_body,
        present::content_type(&base.request_headers),
    );
    let target_req_body = format_body_for_diff(
        &target.request_body,
        present::content_type(&target.request_headers),
    );
    let (req_body_changed, mut req_body_diff) = diff_section_json(&base_req_body, &target_req_body);
    req_body_diff["base_bytes"] = json!(base.req_bytes());
    req_body_diff["target_bytes"] = json!(target.req_bytes());

    let (res_hdr_changed, res_hdr_diff) = diff_section_json(
        &format_headers_for_diff(&base.response_headers),
        &format_headers_for_diff(&target.response_headers),
    );
    let base_res_body = format_body_for_diff(
        &base.response_body,
        present::content_type(&base.response_headers),
    );
    let target_res_body = format_body_for_diff(
        &target.response_body,
        present::content_type(&target.response_headers),
    );
    let (res_body_changed, mut res_body_diff) = diff_section_json(&base_res_body, &target_res_body);
    res_body_diff["base_bytes"] = json!(base.res_bytes());
    res_body_diff["target_bytes"] = json!(target.res_bytes());

    let status_changed = base.status != target.status;
    let identical = !url_changed
        && !status_changed
        && !req_hdr_changed
        && !req_body_changed
        && !res_hdr_changed
        && !res_body_changed;

    tool_json(&json!({
        "base_id": base_id,
        "target_id": target_id,
        "identical": identical,
        "url": {
            "changed": url_changed,
            "base": base.url,
            "target": target.url,
            "method_changed": base.method != target.method,
            "base_method": base.method,
            "target_method": target.method,
            "status_changed": status_changed,
            "base_status": base.status,
            "target_status": target.status,
            "diff": url_diff,
        },
        "request_headers": req_hdr_diff,
        "request_body": req_body_diff,
        "response_headers": res_hdr_diff,
        "response_body": res_body_diff,
    }))
}

fn truncate_utf8_preview(text: &str, cap: usize) -> &str {
    if text.len() <= cap {
        return text;
    }
    let mut end = cap;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

pub(crate) fn format_body_preview(
    bytes: &[u8],
    truncated: bool,
    expired: Option<usize>,
    content_type: Option<&str>,
) -> Value {
    if bytes.is_empty() {
        let mut obj = json!({
            "size": 0,
            "truncated": truncated || expired.is_some(),
            "text": "",
        });
        if let Some(exp) = expired {
            obj["expired_bytes"] = json!(exp);
        }
        return obj;
    }
    let slice = &bytes[..bytes.len().min(BODY_PREVIEW_CAP)];
    let preview_truncated = truncated || bytes.len() > BODY_PREVIEW_CAP;
    let decoded = present::pretty_binary_body(content_type, bytes)
        .map(|s| truncate_utf8_preview(&s, BODY_PREVIEW_CAP).to_string());
    let mut obj = match std::str::from_utf8(slice) {
        Ok(text) => json!({
            "size": bytes.len(),
            "truncated": preview_truncated,
            "encoding": "utf-8",
            "text": text,
        }),
        Err(err) if err.error_len().is_none() && err.valid_up_to() > 0 => {
            let valid = std::str::from_utf8(&slice[..err.valid_up_to()]).unwrap_or("");
            json!({
                "size": bytes.len(),
                "truncated": true,
                "encoding": "utf-8",
                "text": valid,
            })
        }
        Err(_) => json!({
            "size": bytes.len(),
            "truncated": preview_truncated,
            "encoding": "hex",
            "text": present::hex_dump_raw(slice),
        }),
    };
    if let Some(decoded_preview) = decoded {
        obj["decoded_preview"] = json!(decoded_preview);
    }
    if let Some(exp) = expired {
        obj["expired_bytes"] = json!(exp);
    }
    obj
}
