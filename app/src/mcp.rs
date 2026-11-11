//! Built-in Model Context Protocol (MCP) JSON-RPC 2.0 server and stdio bridge.
//!
//! Exposes captured flows, details, rule inspection/modification, and request
//! replay over loopback (`http://127.0.0.1:<port>/mcp`) and `zniff --mcp` stdio.

use std::io::{BufRead, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::present;
use crate::proxy::{Hub, ReplayOverride, replay_once};
use crate::rules;
use crate::session::{FilteredStats, SessionFacets};

const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";
const BODY_PREVIEW_CAP: usize = 32 * 1024;

pub fn mcp_config_json(port: u16) -> String {
    serde_json::to_string_pretty(&json!({
        "mcpServers": {
            "zniff": {
                "url": format!("http://127.0.0.1:{port}/mcp")
            }
        }
    }))
    .unwrap_or_default()
}

pub fn tools_list_value() -> Value {
    json!({
        "tools": [
            {
                "name": "list_flows",
                "description": "List captured HTTP(S)/WebSocket flows in zniff (newest first). Filter tokens are AND-combined (`|` or `,` for OR within a token): `host:`/`domain:` (`*.` wildcard), `path:`, method (`GET`, `POST`, ...), status (`200`, `4xx`, `5xx`, `失败`/`err`), `type:`/`ct:`, `body:` (req/res/WS), `header:`, `note:`/`comment:`, `repeat:`/`repeat:<id>`, `op:`/`gql:`/`operation:` (`*` wildcard), `ip:`/`client:`/`app:`/`proc:`, `pin:`, `slow:` (>=1s), `mute:`/`-term` (exclude), or bare words matching host/path/comment.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "filter": {
                            "type": "string",
                            "description": "Optional zniff filter expression (AND terms; use `|` or `,` inside a term for OR, e.g. `host:api.example.com|cdn.example.com 5xx -path:/health`)."
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of flows to return (1..=200, default 50)."
                        }
                    }
                }
            },
            {
                "name": "get_flow_detail",
                "description": "Get full details of a captured flow by id, including request/response headers, decoded bodies (with Protobuf/gRPC/MessagePack preview and spill expiration info), GraphQL operation label, connection metadata, timing breakdown (DNS/TCP/TLS/Wait/Download), and WebSocket/SSE messages.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Flow ID from list_flows."
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "get_session_summary",
                "description": "Get aggregate traffic statistics (counts by status class, total bytes, average latency, slowest flow) and domain/client-app facets for the session or a filter expression.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "filter": {
                            "type": "string",
                            "description": "Optional zniff filter expression to scope statistics and facets."
                        }
                    }
                }
            },
            {
                "name": "annotate_flow",
                "description": "Set or clear a flow's note/comment and/or pinned (starred, up to 32 flows) state.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Flow ID to annotate."
                        },
                        "comment": {
                            "type": "string",
                            "description": "Optional note text for the flow (pass empty string to clear)."
                        },
                        "pinned": {
                            "type": "boolean",
                            "description": "Optional pin/star state (`true` to pin and keep inline in memory, `false` to unpin)."
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "replay_flow",
                "description": "Replay an existing captured flow by id (or send a composed HTTP/HTTPS request when url is provided), optionally overriding method, url, headers, or body.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Optional source flow ID to replay."
                        },
                        "method": {
                            "type": "string",
                            "description": "Optional HTTP method override (e.g. GET, POST)."
                        },
                        "url": {
                            "type": "string",
                            "description": "Optional target URL override (required when id is omitted)."
                        },
                        "headers": {
                            "type": "string",
                            "description": "Optional raw request headers (`Name: Value` per line) to override."
                        },
                        "body": {
                            "type": "string",
                            "description": "Optional request body string override."
                        }
                    }
                }
            },
            {
                "name": "get_rules",
                "description": "Get the active zniff rewrite/breakpoint/mock rules text, TLS bypass list, and rule switch states.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "set_rules",
                "description": "Set or append zniff rules and/or toggle `rules_enabled` / `breaks_enabled`. Validates syntax before applying. `<pat>` is `<host>[/<path>]` (`*` matches all, `*.example.com/v1`, `*/api`). Templates (one per line):\n- `replace <pat> req|res <from> -> [to] [op=<gql>]`\n- `map-remote <pat> <http(s)://target> [op=<gql>]`\n- `map-local <pat> [file_or_dir] [status=<code>] [delay=<ms|s>] [fallback] [op=<gql>]`\n- `delay <pat> <ms|s> [op=<gql>]`\n- `break <pat> [op=<gql>]` / `break-res <pat> [op=<gql>]`\n- `header <pat> req|res set <Name> <Value> [op=<gql>]` | `header <pat> req|res del <Name> [op=<gql>]`\n- `query <pat> set <Key> <Value> [op=<gql>]` | `query <pat> del <Key> [op=<gql>]`\n- `json <pat> req|res set <ptr_or_path> <json_val> [op=<gql>]` | `json <pat> req|res del <ptr_or_path> [op=<gql>]`\n- `cors <pat>`\n- `no-cache <pat> [op=<gql>]`\n- `block <pat> [status=<code,default 403>|drop|reset] [op=<gql>]`\n- `throttle <pat> <2g|3g|4g|rate>` | `throttle <pat> [down=<rate>] [up=<rate>]`\n- `cert <host> <cert.pem> [key.pem]`\n- `client-cert <host> <cert.pem> [key.pem]`",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "rules": {
                            "type": "string",
                            "description": "Optional rules DSL text (one rule per line; `#` starts a comment). Omit to toggle switches without changing rules."
                        },
                        "append": {
                            "type": "boolean",
                            "description": "When true, appends `rules` to existing rules instead of replacing them."
                        },
                        "rules_enabled": {
                            "type": "boolean",
                            "description": "Optional switch to enable or disable rule evaluation."
                        },
                        "breaks_enabled": {
                            "type": "boolean",
                            "description": "Optional switch to enable or disable `break` / `break-res` interception."
                        }
                    }
                }
            }
        ]
    })
}

pub async fn handle_http_jsonrpc(hub: Arc<Hub>, body: &[u8]) -> Option<Vec<u8>> {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(err) => {
            let resp = jsonrpc_error(Value::Null, -32700, format!("Parse error: {err}"));
            return serde_json::to_vec(&resp).ok();
        }
    };
    if let Some(batch) = parsed.as_array() {
        let mut out = Vec::new();
        for item in batch {
            if let Some(res) = handle_single_request(Arc::clone(&hub), item).await {
                out.push(res);
            }
        }
        if out.is_empty() {
            None
        } else {
            serde_json::to_vec(&Value::Array(out)).ok()
        }
    } else {
        let res = handle_single_request(hub, &parsed).await?;
        serde_json::to_vec(&res).ok()
    }
}

async fn handle_single_request(hub: Arc<Hub>, req: &Value) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

    // JSON-RPC notifications have no `id` and expect no response object.
    let id = id?;

    if let Some(static_res) = handle_discovery_method(&id, method, &params) {
        return Some(static_res);
    }

    match method {
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let tool_res = call_tool(hub, name, &args).await;
            Some(jsonrpc_ok(id, tool_res))
        }
        _ => Some(jsonrpc_error(
            id,
            -32601,
            format!("Method not found: {method}"),
        )),
    }
}

fn handle_discovery_method(id: &Value, method: &str, params: &Value) -> Option<Value> {
    match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_PROTOCOL_VERSION);
            let version = match requested {
                "2025-03-26" | "2024-11-05" => requested,
                _ => DEFAULT_PROTOCOL_VERSION,
            };
            Some(jsonrpc_ok(
                id.clone(),
                json!({
                    "protocolVersion": version,
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "zniff",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            ))
        }
        "ping" => Some(jsonrpc_ok(id.clone(), json!({}))),
        "tools/list" => Some(jsonrpc_ok(id.clone(), tools_list_value())),
        "resources/list" => Some(jsonrpc_ok(id.clone(), json!({ "resources": [] }))),
        "prompts/list" => Some(jsonrpc_ok(id.clone(), json!({ "prompts": [] }))),
        _ => None,
    }
}

async fn call_tool(hub: Arc<Hub>, name: &str, args: &Value) -> Value {
    match name {
        "list_flows" => tool_list_flows(&hub, args),
        "get_flow_detail" => tool_get_flow_detail(&hub, args),
        "get_session_summary" => tool_get_session_summary(&hub, args),
        "annotate_flow" => tool_annotate_flow(&hub, args),
        "replay_flow" => tool_replay_flow(hub, args).await,
        "get_rules" => tool_get_rules(&hub),
        "set_rules" => tool_set_rules(&hub, args),
        _ => tool_error(format!("Unknown tool: {name}")),
    }
}

fn tool_list_flows(hub: &Hub, args: &Value) -> Value {
    let filter = args.get("filter").and_then(Value::as_str).unwrap_or("");
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .clamp(1, 200) as usize;
    let (matched, _) = hub.matching_ids(filter);
    let total_matched = matched.len();
    let start = total_matched.saturating_sub(limit);
    let items: Vec<Value> = hub.with_session(|session| {
        let mut newest: Vec<_> = matched[start..]
            .iter()
            .filter_map(|(id, _)| {
                let flow = session.get(*id)?;
                if flow.request_body.is_empty() && flow.request_spill.is_some() {
                    session.hydrated(*id)
                } else {
                    Some(flow.clone())
                }
            })
            .collect();
        newest.reverse();
        newest
            .into_iter()
            .map(|f| {
                let tb = present::flow_timing_breakdown(&f);
                let graphql_op = present::graphql_operation_label(&f.path, &f.request_body);
                json!({
                    "id": f.id,
                    "method": f.method,
                    "url": f.url,
                    "host": f.host,
                    "path": f.path,
                    "protocol": f.protocol.label(),
                    "status": f.status,
                    "error": f.error,
                    "client_ip": f.client_ip,
                    "elapsed_ms": f.elapsed_ms(),
                    "request_bytes": f.req_bytes(),
                    "response_bytes": f.res_bytes(),
                    "graphql_op": graphql_op,
                    "rule_hit": f.rule_hit,
                    "comment": f.comment,
                    "pinned": f.pinned,
                    "timing": {
                        "dns_ms": tb.dns_ms,
                        "tcp_ms": tb.tcp_ms,
                        "tls_ms": tb.tls_ms,
                        "wait_ms": tb.wait_ms,
                        "download_ms": tb.download_ms,
                        "reused": tb.reused,
                        "server_addr": tb.server_addr,
                        "tls_version": tb.tls_version,
                    }
                })
            })
            .collect()
    });
    tool_json(&json!({
        "filter": filter,
        "count": items.len(),
        "total_matched": total_matched,
        "flows": items,
    }))
}

fn tool_get_flow_detail(hub: &Hub, args: &Value) -> Value {
    let Some(id) = args.get("id").and_then(Value::as_u64) else {
        return tool_error("Missing required integer parameter `id`.");
    };
    let Some(flow) = hub.with_session(|session| session.hydrated(id)) else {
        return tool_error(format!("Flow #{id} not found in session."));
    };
    let tb = present::flow_timing_breakdown(&flow);
    let graphql_op = present::graphql_operation_label(&flow.path, &flow.request_body);
    let req_ct = present::content_type(&flow.request_headers);
    let res_ct = present::content_type(&flow.response_headers);
    let req_body = format_body_preview(
        &flow.request_body,
        flow.request_truncated,
        flow.request_expired,
        req_ct,
    );
    let res_body = format_body_preview(
        &flow.response_body,
        flow.response_truncated,
        flow.response_expired,
        res_ct,
    );
    let messages: Vec<Value> = flow
        .messages
        .iter()
        .map(|m| {
            json!({
                "direction": match m.direction {
                    crate::session::Direction::Request => "send",
                    crate::session::Direction::Response => "receive",
                },
                "kind": m.kind,
                "text": m.text,
            })
        })
        .collect();
    tool_json(&json!({
        "id": flow.id,
        "protocol": flow.protocol.label(),
        "method": flow.method,
        "url": flow.url,
        "host": flow.host,
        "path": flow.path,
        "client_ip": flow.client_ip,
        "status": flow.status,
        "error": flow.error,
        "elapsed_ms": flow.elapsed_ms(),
        "request_bytes": flow.req_bytes(),
        "response_bytes": flow.res_bytes(),
        "request_expired": flow.request_expired,
        "response_expired": flow.response_expired,
        "graphql_op": graphql_op,
        "rule_hit": flow.rule_hit,
        "comment": flow.comment,
        "pinned": flow.pinned,
        "timing": {
            "dns_ms": tb.dns_ms,
            "tcp_ms": tb.tcp_ms,
            "tls_ms": tb.tls_ms,
            "wait_ms": tb.wait_ms,
            "download_ms": tb.download_ms,
            "total_ms": tb.total_ms,
            "reused": tb.reused,
            "server_addr": tb.server_addr,
            "tls_version": tb.tls_version,
        },
        "request_headers": flow.request_headers,
        "request_body": req_body,
        "response_headers": flow.response_headers,
        "response_body": res_body,
        "messages": messages,
    }))
}

fn tool_get_session_summary(hub: &Hub, args: &Value) -> Value {
    let filter = args.get("filter").and_then(Value::as_str).unwrap_or("");
    // Plan under the Session lock and run any spilled-body pread outside the lock.
    let (matched, _) = hub.matching_ids(filter);
    let (session_total, memory_bytes, dropped, stats, facets) = hub.with_session(|session| {
        let flows: Vec<&crate::session::Flow> = matched
            .iter()
            .filter_map(|(id, _)| session.get(*id))
            .collect();
        let stats = FilteredStats::from_flows(flows.iter().copied());
        let facets = SessionFacets::from_flows(flows.iter().copied());
        (
            session.len(),
            session.memory_bytes(),
            session.dropped(),
            stats,
            facets,
        )
    });
    let slowest_flow = stats.max_elapsed.map(|(id, elapsed_ms, target)| {
        json!({
            "id": id,
            "elapsed_ms": elapsed_ms,
            "target": target,
        })
    });
    let domains: Vec<Value> = facets
        .domains
        .into_iter()
        .map(|entry| {
            json!({
                "label": entry.label,
                "token": entry.token,
                "count": entry.count,
                "error_count": entry.error_count,
            })
        })
        .collect();
    let apps: Vec<Value> = facets
        .apps
        .into_iter()
        .map(|entry| {
            json!({
                "label": entry.label,
                "token": entry.token,
                "count": entry.count,
                "error_count": entry.error_count,
            })
        })
        .collect();
    tool_json(&json!({
        "filter": filter,
        "session_total": session_total,
        "matched_count": stats.total_count,
        "memory_bytes": memory_bytes,
        "dropped": dropped,
        "stats": {
            "total_count": stats.total_count,
            "s2xx": stats.s2xx,
            "s3xx": stats.s3xx,
            "s4xx": stats.s4xx,
            "s5xx": stats.s5xx,
            "failed": stats.failed,
            "pending": stats.pending,
            "total_req_bytes": stats.total_req_bytes,
            "total_res_bytes": stats.total_res_bytes,
            "total_bytes": stats.total_bytes,
            "avg_elapsed_ms": stats.avg_elapsed_ms,
            "slowest_flow": slowest_flow,
        },
        "facets": {
            "domains": domains,
            "apps": apps,
        },
    }))
}

fn tool_annotate_flow(hub: &Hub, args: &Value) -> Value {
    let Some(id) = args.get("id").and_then(Value::as_u64) else {
        return tool_error("Missing required integer parameter `id`.");
    };
    let comment_arg = args.get("comment").and_then(Value::as_str);
    let pinned_arg = args.get("pinned").and_then(Value::as_bool);
    if comment_arg.is_none() && pinned_arg.is_none() {
        return tool_error("Provide at least one of `comment` or `pinned`.");
    }
    if hub.with_session(|session| session.get(id).is_none()) {
        return tool_error(format!("Flow #{id} not found in session."));
    }
    if let Some(want_pinned) = pinned_arg
        && !hub.set_pinned(id, want_pinned)
    {
        return tool_error(format!(
            "Cannot pin flow #{id}: maximum pinned flows limit ({}) reached or flow disappeared.",
            crate::session::PIN_CAP
        ));
    }
    if let Some(note) = comment_arg
        && !hub.set_comment(id, note)
    {
        return tool_error(format!("Flow #{id} not found in session."));
    }
    let Some((comment, pinned)) =
        hub.with_session(|session| session.get(id).map(|f| (f.comment.clone(), f.pinned)))
    else {
        return tool_error(format!("Flow #{id} not found in session."));
    };
    tool_json(&json!({
        "ok": true,
        "id": id,
        "comment": comment,
        "pinned": pinned,
    }))
}

async fn tool_replay_flow(hub: Arc<Hub>, args: &Value) -> Value {
    let id = args.get("id").and_then(Value::as_u64);
    let source = match id {
        Some(flow_id) => {
            let Some(found) = hub.with_session(|session| session.hydrated(flow_id)) else {
                return tool_error(format!("Flow #{flow_id} not found."));
            };
            Some(found)
        }
        None => None,
    };
    let method = args
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);
    let url = args.get("url").and_then(Value::as_str).map(str::to_string);
    let headers = args.get("headers").and_then(parse_headers_arg);
    let body = args
        .get("body")
        .and_then(Value::as_str)
        .map(|s| s.as_bytes().to_vec());

    let edit = ReplayOverride {
        method,
        url,
        headers,
        body,
    };
    match replay_once(Arc::clone(&hub), source, edit).await {
        Ok(new_id) => tool_get_flow_detail(&hub, &json!({ "id": new_id })),
        Err(err) => tool_error(format!("Replay failed: {err:#}")),
    }
}

fn tool_get_rules(hub: &Hub) -> Value {
    tool_json(&json!({
        "rules": hub.rules_text(),
        "bypass": hub.bypass_text(),
        "apply_rules": hub.rules_enabled(),
        "apply_breaks": hub.breaks_enabled(),
    }))
}

fn tool_set_rules(hub: &Hub, args: &Value) -> Value {
    let new_rules = args.get("rules").and_then(Value::as_str);
    let rules_enabled = args.get("rules_enabled").and_then(Value::as_bool);
    let breaks_enabled = args.get("breaks_enabled").and_then(Value::as_bool);
    if new_rules.is_none() && rules_enabled.is_none() && breaks_enabled.is_none() {
        return tool_error(
            "Provide `rules` and/or at least one switch (`rules_enabled`, `breaks_enabled`).",
        );
    }
    let append = args.get("append").and_then(Value::as_bool).unwrap_or(false);
    let combined = if let Some(new_rules) = new_rules {
        let next = if append {
            let existing = hub.rules_text();
            if existing.trim().is_empty() {
                new_rules.trim().to_string()
            } else {
                format!("{}\n{}", existing.trim_end(), new_rules.trim())
            }
        } else {
            new_rules.to_string()
        };
        let bad_lines = rules::invalid_rule_lines(&next);
        if !bad_lines.is_empty() {
            return tool_error(format!(
                "Invalid rule syntax on line(s): {}",
                bad_lines
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Some(next)
    } else {
        None
    };

    if let Some(ref text) = combined {
        hub.set_rules(text);
    }
    if let Some(enabled) = rules_enabled {
        hub.set_rules_enabled(enabled);
    }
    if let Some(enabled) = breaks_enabled {
        hub.set_breaks_enabled(enabled);
    }

    let save_res =
        crate::config::update_mode(hub.ca.dir(), crate::config::is_dev_mode(), |config| {
            if let Some(ref text) = combined {
                config.rules = text.clone();
            }
            if let Some(enabled) = rules_enabled {
                config.rules_enabled = enabled;
            }
            if let Some(enabled) = breaks_enabled {
                config.breaks_enabled = enabled;
            }
        });
    let current_rules = hub.rules_text();
    let parsed_count = rules::parse_rules(&current_rules).len();
    let mut resp = json!({
        "ok": true,
        "active_rule_count": parsed_count,
        "rules": current_rules,
        "apply_rules": hub.rules_enabled(),
        "apply_breaks": hub.breaks_enabled(),
    });
    if let Err(err) = save_res {
        resp["warning"] = json!(format!("Failed to persist rules to disk: {err}"));
    }
    tool_json(&resp)
}

fn parse_headers_arg(val: &Value) -> Option<Vec<(String, String)>> {
    if let Some(text) = val.as_str() {
        let pairs = text
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() {
                    return None;
                }
                let (k, v) = line.split_once(':')?;
                Some((k.trim().to_string(), v.trim().to_string()))
            })
            .collect();
        return Some(pairs);
    }
    if let Some(obj) = val.as_object() {
        let pairs = obj
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
            .collect();
        return Some(pairs);
    }
    None
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

fn format_body_preview(
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

fn tool_json(data: &Value) -> Value {
    let text = serde_json::to_string_pretty(data).unwrap_or_else(|_| "{}".to_string());
    json!({
        "content": [
            {
                "type": "text",
                "text": text
            }
        ],
        "isError": false
    })
}

fn tool_error(msg: impl Into<String>) -> Value {
    json!({
        "content": [
            {
                "type": "text",
                "text": msg.into()
            }
        ],
        "isError": true
    })
}

fn jsonrpc_ok(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn jsonrpc_error(id: Value, code: i64, message: String) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    })
}

/// Runs the `zniff --mcp [port]` stdio bridge.
pub fn run_stdio_bridge(port: u16) -> i32 {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<Value>(trimmed) else {
            let err = jsonrpc_error(Value::Null, -32700, "Parse error".into());
            if let Ok(text) = serde_json::to_string(&err) {
                let _ = writeln!(stdout, "{text}");
                let _ = stdout.flush();
            }
            continue;
        };
        // Notifications have no `id`
        let Some(id) = parsed.get("id").cloned() else {
            continue;
        };
        let method = parsed.get("method").and_then(Value::as_str).unwrap_or("");
        let params = parsed.get("params").cloned().unwrap_or_else(|| json!({}));
        if let Some(local) = handle_discovery_method(&id, method, &params) {
            if let Ok(text) = serde_json::to_string(&local) {
                let _ = writeln!(stdout, "{text}");
                let _ = stdout.flush();
            }
            continue;
        }
        let reply = match post_local_mcp(port, trimmed.as_bytes()) {
            Ok(Some(bytes)) => String::from_utf8(bytes).ok(),
            Ok(None) => None,
            Err(err) => {
                let fallback = jsonrpc_ok(
                    id,
                    tool_error(format!(
                        "Cannot connect to zniff local proxy on 127.0.0.1:{port} ({err}). Make sure zniff is open and capture is started."
                    )),
                );
                serde_json::to_string(&fallback).ok()
            }
        };
        if let Some(text) = reply {
            let _ = writeln!(stdout, "{}", text.trim());
            let _ = stdout.flush();
        }
    }
    0
}

fn post_local_mcp(port: u16, body: &[u8]) -> std::io::Result<Option<Vec<u8>>> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let req = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(req.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
        return Ok(None);
    };
    let head = String::from_utf8_lossy(&raw[..pos]);
    if head.starts_with("HTTP/1.1 202") || head.starts_with("HTTP/1.0 202") {
        return Ok(None);
    }
    let resp_body = raw[pos + 4..].to_vec();
    if resp_body.is_empty() {
        Ok(None)
    } else {
        Ok(Some(resp_body))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::ca::Ca;
    use crate::session::{Direction, FlowDraft, Protocol};

    fn test_hub() -> Arc<Hub> {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("zniff-mcp-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
        hub
    }

    fn parse_tool_payload(res: &Value) -> Value {
        assert_eq!(res["isError"], false, "unexpected tool error: {res}");
        let text = res["content"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap()
    }

    #[tokio::test]
    async fn mcp_initialize_list_flows_detail_and_rules_work() {
        let hub = test_hub();
        let id = hub.with_session(|s| {
            let id = s.open(FlowDraft {
                protocol: Protocol::Http2,
                method: "POST".into(),
                host: "api.example.com".into(),
                path: "/v1/orders".into(),
                url: "https://api.example.com/v1/orders".into(),
                request_headers: vec![("Content-Type".into(), "application/json".into())],
            });
            s.append_body(id, Direction::Request, br#"{"item":"book"}"#);
            s.set_response(
                id,
                500,
                vec![("Content-Type".into(), "application/json".into())],
                None,
            );
            s.append_body(id, Direction::Response, br#"{"error":"db timeout"}"#);
            s.mark_finished(id);
            id
        });

        // 1. initialize
        let init_req = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#;
        let init_res: Value =
            serde_json::from_slice(&handle_http_jsonrpc(hub.clone(), init_req).await.unwrap())
                .unwrap();
        assert_eq!(init_res["result"]["serverInfo"]["name"], "zniff");

        // 2. notification returns None
        let notif = br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(handle_http_jsonrpc(hub.clone(), notif).await.is_none());

        // 3. list_flows with filter
        let list_req = br#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_flows","arguments":{"filter":"5xx host:api.example.com"}}}"#;
        let list_res: Value =
            serde_json::from_slice(&handle_http_jsonrpc(hub.clone(), list_req).await.unwrap())
                .unwrap();
        let list_text = list_res["result"]["content"][0]["text"].as_str().unwrap();
        assert!(list_text.contains("https://api.example.com/v1/orders"));

        // 4. get_flow_detail
        let detail_req = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "get_flow_detail",
                "arguments": { "id": id }
            }
        }))
        .unwrap();
        let detail_res: Value =
            serde_json::from_slice(&handle_http_jsonrpc(hub.clone(), &detail_req).await.unwrap())
                .unwrap();
        let detail_text = detail_res["result"]["content"][0]["text"].as_str().unwrap();
        assert!(detail_text.contains("db timeout"));

        // 5. set_rules & get_rules
        let set_req = br#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"set_rules","arguments":{"rules":"cors api.example.com"}}}"#;
        let set_res: Value =
            serde_json::from_slice(&handle_http_jsonrpc(hub.clone(), set_req).await.unwrap())
                .unwrap();
        assert_eq!(set_res["result"]["isError"], false);
        assert_eq!(hub.rules_text().trim(), "cors api.example.com");
    }

    #[tokio::test]
    async fn test_mcp_set_rules_persists_to_disk_and_increments_revision() {
        let dir = std::env::temp_dir().join(format!("zniff-mcp-persist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());

        assert_eq!(hub.rules_revision(), 0);

        let set_req = br#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"set_rules","arguments":{"rules":"cors test.example.com","rules_enabled":false,"breaks_enabled":false}}}"#;
        let set_res: Value =
            serde_json::from_slice(&handle_http_jsonrpc(hub.clone(), set_req).await.unwrap())
                .unwrap();
        let set_payload = parse_tool_payload(&set_res["result"]);
        assert_eq!(set_payload["apply_rules"], false);
        assert_eq!(set_payload["apply_breaks"], false);
        assert!(!hub.rules_enabled());
        assert!(!hub.breaks_enabled());
        assert_eq!(hub.rules_revision(), 1);

        // Toggle switches alone without resending `rules`
        let toggle_req = br#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"set_rules","arguments":{"rules_enabled":true}}}"#;
        let toggle_res: Value =
            serde_json::from_slice(&handle_http_jsonrpc(hub.clone(), toggle_req).await.unwrap())
                .unwrap();
        let toggle_payload = parse_tool_payload(&toggle_res["result"]);
        assert_eq!(toggle_payload["apply_rules"], true);
        assert_eq!(toggle_payload["apply_breaks"], false);
        assert_eq!(hub.rules_text().trim(), "cors test.example.com");

        // Verify rules and switches on disk
        let config_file = dir.join(crate::config::config_file_name(crate::config::is_dev_mode()));
        assert!(config_file.exists());
        let saved_cfg = crate::config::load_mode(&dir, crate::config::is_dev_mode());
        assert_eq!(saved_cfg.rules.trim(), "cors test.example.com");
        assert!(saved_cfg.rules_enabled);
        assert!(!saved_cfg.breaks_enabled);

        // Syntax error does NOT persist, does NOT flip switches, and does NOT increment revision
        let bad_req = br#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"set_rules","arguments":{"rules":"cors test.example.com op=Invalid","rules_enabled":false}}}"#;
        let bad_res: Value =
            serde_json::from_slice(&handle_http_jsonrpc(hub.clone(), bad_req).await.unwrap())
                .unwrap();
        assert_eq!(bad_res["result"]["isError"], true);
        assert!(hub.rules_enabled());
        assert_eq!(hub.rules_revision(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn format_body_preview_preserves_multibyte_utf8_at_32kb_boundary_and_decodes_binary() {
        // 32767 ASCII bytes + 3-byte UTF-8 '中' (0xE4 0xB8 0xAD) = 32770 bytes.
        // Byte 32767..32768 splits '中' after its first byte 0xE4.
        let mut split_utf8 = vec![b'a'; BODY_PREVIEW_CAP - 1];
        split_utf8.extend_from_slice("中尾".as_bytes());
        let preview = format_body_preview(&split_utf8, false, None, Some("application/json"));
        assert_eq!(preview["encoding"], "utf-8");
        assert_eq!(preview["truncated"], true);
        assert_eq!(preview["size"], split_utf8.len());
        assert_eq!(
            preview["text"].as_str().unwrap().len(),
            BODY_PREVIEW_CAP - 1
        );

        // Genuine binary invalid UTF-8 still uses hex, and attaches decoded_preview for MessagePack.
        // MsgPack fixmap(1) { "id": 42 } -> 0x81 0xa2 'i' 'd' 0x2a
        let msgpack = [0x81u8, 0xa2, b'i', b'd', 0x2a];
        let mp_preview = format_body_preview(&msgpack, false, None, Some("application/msgpack"));
        assert_eq!(mp_preview["encoding"], "hex");
        let decoded = mp_preview["decoded_preview"].as_str().unwrap();
        assert!(decoded.contains("\"id\": 42"), "{decoded}");

        // Protobuf bytes that happen to be valid ASCII control chars still attach decoded_preview.
        // Field 1 varint 7 (0x08 0x07), field 2 string "zniff" (0x12 0x05 "zniff")
        let proto = b"\x08\x07\x12\x05zniff";
        let pb_preview = format_body_preview(proto, false, None, Some("application/x-protobuf"));
        let pb_decoded = pb_preview["decoded_preview"].as_str().unwrap();
        assert!(pb_decoded.contains("zniff"), "{pb_decoded}");

        // Expired spilled body reports expired_bytes.
        let exp_preview = format_body_preview(&[], false, Some(65_536), Some("application/json"));
        assert_eq!(exp_preview["truncated"], true);
        assert_eq!(exp_preview["expired_bytes"], 65_536);
    }

    #[test]
    fn detail_and_list_expose_graphql_op_total_matched_and_expired_bytes() {
        let hub = test_hub();
        let mut ids = Vec::new();
        hub.with_session(|s| {
            for op_name in ["GetViewer", "UpdateCart", "ListOrders"] {
                let id = s.open(FlowDraft {
                    protocol: Protocol::Http2,
                    method: "POST".into(),
                    host: "gql.example.com".into(),
                    path: "/graphql".into(),
                    url: "https://gql.example.com/graphql".into(),
                    request_headers: vec![("Content-Type".into(), "application/json".into())],
                });
                let body = format!(
                    r#"{{"operationName":"{op_name}","query":"query {op_name} {{ ok }}"}}"#
                );
                s.append_body(id, Direction::Request, body.as_bytes());
                s.set_response(id, 200, vec![], None);
                s.mark_finished(id);
                ids.push(id);
            }
        });

        let listed = parse_tool_payload(&tool_list_flows(
            &hub,
            &json!({ "filter": "host:gql.example.com", "limit": 2 }),
        ));
        assert_eq!(listed["count"], 2);
        assert_eq!(listed["total_matched"], 3);
        assert_eq!(listed["flows"][0]["graphql_op"], "ListOrders");
        assert_eq!(listed["flows"][1]["graphql_op"], "UpdateCart");

        let detail = parse_tool_payload(&tool_get_flow_detail(&hub, &json!({ "id": ids[0] })));
        assert_eq!(detail["graphql_op"], "GetViewer");
        assert!(detail["request_expired"].is_null());
        assert!(detail["response_expired"].is_null());
    }

    #[test]
    fn session_summary_and_annotate_flow_work_and_rule_templates_parse() {
        let hub = test_hub();
        let (id1, id2) = hub.with_session(|s| {
            let id1 = s.open(FlowDraft {
                protocol: Protocol::Http2,
                method: "GET".into(),
                host: "api.example.com".into(),
                path: "/users".into(),
                url: "https://api.example.com/users".into(),
                request_headers: vec![],
            });
            s.set_client_ip(id1, "127.0.0.1 (curl)");
            s.set_response(id1, 200, vec![], None);
            s.append_body(id1, Direction::Response, b"hello");
            s.mark_finished(id1);

            let id2 = s.open(FlowDraft {
                protocol: Protocol::Http11,
                method: "POST".into(),
                host: "auth.example.com".into(),
                path: "/login".into(),
                url: "https://auth.example.com/login".into(),
                request_headers: vec![],
            });
            s.set_client_ip(id2, "127.0.0.1 (node)");
            s.set_response(id2, 502, vec![], None);
            s.mark_finished(id2);
            (id1, id2)
        });

        // annotate_flow sets comment and pins
        let ann = parse_tool_payload(&tool_annotate_flow(
            &hub,
            &json!({ "id": id2, "comment": "investigate 502", "pinned": true }),
        ));
        assert_eq!(ann["ok"], true);
        assert_eq!(ann["comment"], "investigate 502");
        assert_eq!(ann["pinned"], true);

        // get_session_summary aggregates stats and facets
        let summary = parse_tool_payload(&tool_get_session_summary(&hub, &json!({})));
        assert_eq!(summary["session_total"], 2);
        assert_eq!(summary["matched_count"], 2);
        assert_eq!(summary["stats"]["s2xx"], 1);
        assert_eq!(summary["stats"]["s5xx"], 1);
        assert_eq!(summary["facets"]["domains"].as_array().unwrap().len(), 2);
        assert_eq!(summary["facets"]["apps"].as_array().unwrap().len(), 2);

        // Filtered summary via note: filter
        let filtered = parse_tool_payload(&tool_get_session_summary(
            &hub,
            &json!({ "filter": "note:investigate pin:" }),
        ));
        assert_eq!(filtered["matched_count"], 1);
        assert_eq!(filtered["stats"]["s5xx"], 1);
        let _ = id1;

        // Every rule syntax template documented in set_rules must parse cleanly
        let sample_rules = "\
replace api.example.com/v1 res old -> new op=GetUser
map-remote api.example.com/v1 https://staging.example.com/v1 op=GetUser
map-local api.example.com/static ./public status=200 delay=50ms fallback op=GetUser
delay api.example.com/v1 200ms op=GetUser
break api.example.com/pay op=Checkout
break-res api.example.com/pay op=Checkout
header api.example.com req set X-Env staging op=GetUser
header api.example.com res del Server op=GetUser
query api.example.com/search set debug 1 op=Search
query api.example.com/search del utm_source op=Search
json api.example.com/v1 res set /data/flag true op=GetUser
json api.example.com/v1 req del /meta/secret op=GetUser
cors api.example.com
no-cache api.example.com/v1 op=GetUser
block ads.example.com status=451 op=Track
throttle api.example.com down=512kb up=256kb
cert internal.example.com ./server.crt ./server.key
client-cert mtls.example.com ./client.crt ./client.key";
        assert!(
            rules::invalid_rule_lines(sample_rules).is_empty(),
            "invalid lines: {:?}",
            rules::invalid_rule_lines(sample_rules)
        );
        assert_eq!(rules::parse_rules(sample_rules).len(), 18);

        let tools = tools_list_value();
        let names: Vec<&str> = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(names.contains(&"get_session_summary"));
        assert!(names.contains(&"annotate_flow"));
    }
}
