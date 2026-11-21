//! MCP Resources (`resources/list`, `resources/read`) and
//! MCP Prompts (`prompts/list`, `prompts/get`).

use serde_json::{Value, json};

use crate::ai_context::format_flow_for_ai;
use crate::mcp::flow_summary_json;
use crate::proxy::Hub;
use crate::redact::RedactOptions;
use crate::session::{FilteredStats, Flow};

const RESOURCE_FLOWS_URI: &str = "zniff://flows/current";
const RESOURCE_RULES_URI: &str = "zniff://rules/active";
const PROMPT_ANALYZE_FLOW: &str = "analyze_failed_flow";

pub(crate) fn resources_list_value() -> Value {
    json!({
        "resources": [
            {
                "uri": RESOURCE_FLOWS_URI,
                "name": "Current Session Flows",
                "description": "Summary and statistics of captured HTTP(S)/WebSocket flows in the active zniff session.",
                "mimeType": "application/json"
            },
            {
                "uri": RESOURCE_RULES_URI,
                "name": "Active Rules",
                "description": "Active zniff rewrite/mock/breakpoint rules, rule groups, TLS bypass list, and switch states.",
                "mimeType": "application/json"
            }
        ]
    })
}

pub(crate) fn handle_resources_read(hub: &Hub, params: &Value) -> Result<Value, (i64, String)> {
    let Some(uri) = params.get("uri").and_then(Value::as_str) else {
        return Err((-32602, "Missing required `uri` parameter.".to_string()));
    };
    let payload = match uri {
        RESOURCE_FLOWS_URI => {
            let (matched, _) = hub.matching_ids("");
            let total_matched = matched.len();
            let start = total_matched.saturating_sub(100);
            let (session_total, stats, flows) = hub.with_session(|session| {
                let all_refs: Vec<&Flow> = matched
                    .iter()
                    .filter_map(|(id, _)| session.get(*id))
                    .collect();
                let stats = FilteredStats::from_flows(all_refs.iter().copied());
                let mut newest: Vec<Flow> = matched[start..]
                    .iter()
                    .filter_map(|(id, _)| {
                        let f = session.get(*id)?;
                        if f.request_body.is_empty() && f.request_spill.is_some() {
                            session.hydrated(*id)
                        } else {
                            Some(f.clone())
                        }
                    })
                    .collect();
                newest.reverse();
                let items: Vec<Value> = newest.iter().map(flow_summary_json).collect();
                (session.len(), stats, items)
            });
            json!({
                "session_total": session_total,
                "count": flows.len(),
                "stats": {
                    "total_count": stats.total_count,
                    "s2xx": stats.s2xx,
                    "s3xx": stats.s3xx,
                    "s4xx": stats.s4xx,
                    "s5xx": stats.s5xx,
                    "failed": stats.failed,
                    "pending": stats.pending,
                    "total_bytes": stats.total_bytes,
                    "avg_elapsed_ms": stats.avg_elapsed_ms,
                },
                "flows": flows,
            })
        }
        RESOURCE_RULES_URI => json!({
            "rules": hub.rules_text(),
            "rule_groups": hub.rule_groups(),
            "effective_rules_count": hub.effective_rules_count(),
            "bypass": hub.bypass_text(),
            "apply_rules": hub.rules_enabled(),
            "apply_breaks": hub.breaks_enabled(),
        }),
        other => {
            return Err((-32602, format!("Unknown resource URI: {other}")));
        }
    };
    let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string());
    Ok(json!({
        "contents": [
            {
                "uri": uri,
                "mimeType": "application/json",
                "text": text,
            }
        ]
    }))
}

pub(crate) fn prompts_list_value() -> Value {
    json!({
        "prompts": [
            {
                "name": PROMPT_ANALYZE_FLOW,
                "description": "Analyze a failed or slow HTTP flow using redacted request/response diagnostics.",
                "arguments": [
                    {
                        "name": "flow_id",
                        "description": "Optional flow ID to analyze. When omitted, selects the most recent failed or slow flow.",
                        "required": false
                    }
                ]
            }
        ]
    })
}

fn parse_prompt_flow_id(arguments: &Value) -> Result<Option<u64>, (i64, String)> {
    match arguments.get("flow_id") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            if let Some(id) = v.as_u64() {
                Ok(Some(id))
            } else if let Some(s) = v.as_str() {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    trimmed
                        .parse::<u64>()
                        .map(Some)
                        .map_err(|_| (-32602, format!("Invalid `flow_id` argument: `{trimmed}`.")))
                }
            } else {
                Err((
                    -32602,
                    "`flow_id` must be an integer or numeric string.".to_string(),
                ))
            }
        }
    }
}

fn pick_flow_for_analysis(
    hub: &Hub,
    explicit_id: Option<u64>,
) -> Result<Option<Flow>, (i64, String)> {
    if let Some(id) = explicit_id {
        let Some(flow) = hub.with_session(|session| session.hydrated(id)) else {
            return Err((-32602, format!("Flow #{id} not found in session.")));
        };
        return Ok(Some(flow));
    }
    let chosen_id = hub.with_session(|session| {
        let mut latest_failed: Option<u64> = None;
        let mut latest_slow: Option<u64> = None;
        let mut latest_any: Option<u64> = None;
        for flow in session.iter() {
            latest_any = Some(flow.id);
            let is_failed = flow.error.is_some() || flow.status.is_some_and(|s| s >= 400);
            if is_failed {
                latest_failed = Some(flow.id);
            }
            if flow.elapsed_ms().is_some_and(|ms| ms >= 1_000) {
                latest_slow = Some(flow.id);
            }
        }
        latest_failed.or(latest_slow).or(latest_any)
    });
    Ok(chosen_id.and_then(|id| hub.with_session(|session| session.hydrated(id))))
}

pub(crate) fn handle_prompts_get(hub: &Hub, params: &Value) -> Result<Value, (i64, String)> {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return Err((-32602, "Missing required `name` parameter.".to_string()));
    };
    if name != PROMPT_ANALYZE_FLOW {
        return Err((-32602, format!("Unknown prompt: {name}")));
    }
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let explicit_id = parse_prompt_flow_id(&arguments)?;
    let flow = pick_flow_for_analysis(hub, explicit_id)?;
    let (description, prompt_text) = match flow {
        Some(flow) => {
            let markdown = format_flow_for_ai(&flow, &RedactOptions::default());
            (
                format!("Analyze HTTP flow #{} ({} {})", flow.id, flow.method, flow.host),
                format!(
                    "Please analyze the following captured HTTP flow from zniff. Identify the root cause of any error, unexpected status code, or high latency, and suggest concrete debugging steps.\n\n{markdown}"
                ),
            )
        }
        None => (
            "Analyze failed or slow HTTP flow (no flows captured yet)".to_string(),
            "No captured HTTP flows are currently present in the zniff session. Trigger a request through the proxy or pass a specific `flow_id` once traffic is captured.".to_string(),
        ),
    };
    Ok(json!({
        "description": description,
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": prompt_text,
                }
            }
        ]
    }))
}
