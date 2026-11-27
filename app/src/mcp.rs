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
use crate::proxy::{BreakDecision, BreakSide, Hub, ReplayOverride, replay_once};
use crate::rules;
use crate::session::{FilteredStats, SessionFacets};

const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";

fn mcp_lang() -> crate::i18n::Lang {
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(val) = std::env::var(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return crate::i18n::lang_from_tag(trimmed);
            }
        }
    }
    crate::i18n::Lang::En
}

fn mcp_localize_error(err: &str) -> String {
    crate::i18n::localize_error(mcp_lang(), err)
}

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
    let mut list = json!({
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
                "description": "Set or append zniff rules and/or toggle `rules_enabled` / `breaks_enabled`. Validates syntax before applying. `<pat>` is `<host>[/<path>]` (`*` matches all, `*.example.com/v1`, `*/api`). Templates (one per line):\n- `replace <pat> req|res <from> -> [to] [op=<gql>]`\n- `map-remote <pat> <http(s)://target> [op=<gql>]`\n- `map-local <pat> [file_or_dir] [status=<code>] [delay=<ms|s>] [fallback] [op=<gql>]`\n- `delay <pat> <ms|s> [op=<gql>]`\n- `break <pat> [op=<gql>]` / `break-res <pat> [op=<gql>]`\n- `header <pat> req|res set <Name> <Value> [op=<gql>]` | `header <pat> req|res del <Name> [op=<gql>]`\n- `query <pat> set <Key> <Value> [op=<gql>]` | `query <pat> del <Key> [op=<gql>]`\n- `json <pat> req|res set <ptr_or_path> <json_val> [op=<gql>]` | `json <pat> req|res del <ptr_or_path> [op=<gql>]`\n- `cors <pat>`\n- `no-cache <pat> [op=<gql>]`\n- `block <pat> [status=<code,default 403>|drop|reset] [op=<gql>]`\n- `throttle <pat> <2g|3g|4g|rate>` | `throttle <pat> [down=<rate>] [up=<rate>]`\n- `cert <host> <cert.pem> [key.pem]`\n- `client-cert <host> <cert.pem> [key.pem]`\n- `resolve <host> <ip_or_host>[:<port>]` (aliases `dns`, `hosts`)",
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
                        "rule_groups": {
                            "type": "array",
                            "description": "Optional list of named toggleable rule groups (`[{name, enabled, rules}]`). Evaluated alongside workspace `rules` when enabled.",
                            "items": {
                                "type": "object",
                                "required": ["name"],
                                "properties": {
                                    "name": { "type": "string" },
                                    "enabled": { "type": "boolean" },
                                    "rules": { "type": "string" }
                                }
                            }
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
            },
            {
                "name": "clear_flows",
                "description": "Clear captured flows from the current zniff session (all flows or only unpinned flows).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "keep_pinned": {
                            "type": "boolean",
                            "description": "When true, keeps pinned/starred flows and clears only unpinned flows (default false)."
                        }
                    }
                }
            },
            {
                "name": "manage_breaks",
                "description": "Inspect and control requests/responses currently paused at `break` / `break-res` breakpoints.",
                "inputSchema": {
                    "type": "object",
                    "required": ["action"],
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["list", "continue", "continue_all", "drop"],
                            "description": "Breakpoint queue operation to execute."
                        },
                        "id": {
                            "type": "integer",
                            "description": "Optional paused flow id to promote to the queue head before `continue` or `drop`."
                        },
                        "status": {
                            "type": "integer",
                            "description": "Optional HTTP status code (100-599) override when continuing a response breakpoint."
                        },
                        "headers": {
                            "type": ["string", "object"],
                            "description": "Optional replacement headers (`Name: Value` lines or JSON object) when continuing."
                        },
                        "body": {
                            "type": "string",
                            "description": "Optional UTF-8 replacement body when continuing."
                        }
                    }
                }
            },
            {
                "name": "generate_openapi",
                "description": "Synthesize an OpenAPI 3.0.3 JSON specification from captured HTTP(S) flows matching an optional zniff filter.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "filter": {
                            "type": "string",
                            "description": "Optional zniff filter expression (e.g. `host:api.vicanso.dev status:2xx`). Empty includes all flows."
                        },
                        "redact": {
                            "type": "boolean",
                            "description": "When true (default false), redacts sensitive headers/tokens/fields using saved redaction settings before synthesis."
                        },
                        "format": {
                            "type": "string",
                            "enum": ["json", "yaml"],
                            "description": "Optional output format (`json` default, or `yaml` to include a YAML string in the response)."
                        }
                    }
                }
            }
        ]
    });
    if let Some(arr) = list["tools"].as_array_mut() {
        arr.extend(crate::mcp_tools::extra_tool_schemas());
    }
    list
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
        "resources/read" => match crate::mcp_resources::handle_resources_read(&hub, &params) {
            Ok(res) => Some(jsonrpc_ok(id, res)),
            Err((code, msg)) => Some(jsonrpc_error(id, code, msg)),
        },
        "prompts/get" => match crate::mcp_resources::handle_prompts_get(&hub, &params) {
            Ok(res) => Some(jsonrpc_ok(id, res)),
            Err((code, msg)) => Some(jsonrpc_error(id, code, msg)),
        },
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
                        "tools": {},
                        "resources": {},
                        "prompts": {}
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
        "resources/list" => Some(jsonrpc_ok(
            id.clone(),
            crate::mcp_resources::resources_list_value(),
        )),
        "prompts/list" => Some(jsonrpc_ok(
            id.clone(),
            crate::mcp_resources::prompts_list_value(),
        )),
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
        "clear_flows" => tool_clear_flows(&hub, args),
        "manage_breaks" => tool_manage_breaks(&hub, args),
        "generate_openapi" => tool_generate_openapi(&hub, args),
        "wait_for_flow" => crate::mcp_tools::tool_wait_for_flow(hub, args).await,
        "mock_flow" | "add_rule_from_flow" => crate::mcp_tools::tool_mock_flow(&hub, args),
        "diff_flows" => crate::mcp_tools::tool_diff_flows(&hub, args),
        "list_script_hooks" => crate::mcp_tools::tool_list_script_hooks(&hub),
        "run_script_hook" => crate::mcp_tools::tool_run_script_hook(hub, args).await,
        _ => tool_error(format!("Unknown tool: {name}")),
    }
}

pub(crate) fn flow_summary_json(f: &crate::session::Flow) -> Value {
    let tb = present::flow_timing_breakdown(f);
    let graphql_op = present::graphql_operation_label(&f.path, &f.request_body);
    json!({
        "id": f.id,
        "method": f.method,
        "url": f.url,
        "host": f.host,
        "path": f.path,
        "protocol": f.protocol.label(),
        "status": f.status,
        "error": f.error.as_deref().map(mcp_localize_error),
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
            "client_cert": if tb.client_cert.is_empty() { Value::Null } else { Value::String(tb.client_cert) },
        }
    })
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
        newest.iter().map(flow_summary_json).collect()
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
    let req_body = crate::mcp_tools::format_body_preview(
        &flow.request_body,
        flow.request_truncated,
        flow.request_expired,
        req_ct,
    );
    let res_body = crate::mcp_tools::format_body_preview(
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
        "error": flow.error.as_deref().map(mcp_localize_error),
        "elapsed_ms": flow.elapsed_ms(),
        "request_bytes": flow.req_bytes(),
        "response_bytes": flow.res_bytes(),
        "request_expired": flow.request_expired,
        "response_expired": flow.response_expired,
        "graphql_op": graphql_op,
        "rule_hit": flow.rule_hit,
        "rule_hits": flow
            .rule_hits
            .iter()
            .map(|hit| {
                json!({
                    "group": hit.group_name,
                    "line": hit.line_no,
                    "verb": hit.verb,
                })
            })
            .collect::<Vec<_>>(),
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
            "client_cert": if tb.client_cert.is_empty() { Value::Null } else { Value::String(tb.client_cert) },
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
    let allow_denied = hub.allow_denied_summary();
    let mut payload = json!({
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
    });
    if allow_denied.total > 0
        && let Some(obj) = payload.as_object_mut()
    {
        obj.insert("allow_denied_total".to_string(), json!(allow_denied.total));
        let hosts: Vec<Value> = allow_denied
            .hosts
            .into_iter()
            .map(|(host, count)| json!({"host": host, "count": count}))
            .collect();
        obj.insert("allow_denied_hosts".to_string(), json!(hosts));
    }
    tool_json(&payload)
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
        "rule_groups": hub.rule_groups(),
        "effective_rules_count": hub.effective_rules_count(),
        "bypass": hub.bypass_text(),
        "apply_rules": hub.rules_enabled(),
        "apply_breaks": hub.breaks_enabled(),
    }))
}

fn parse_mcp_rule_groups(args: &Value) -> Result<Option<Vec<crate::config::RuleGroup>>, Value> {
    let Some(raw) = args.get("rule_groups") else {
        return Ok(None);
    };
    let Ok(parsed) = serde_json::from_value::<Vec<crate::config::RuleGroup>>(raw.clone()) else {
        return Err(tool_error(
            "`rule_groups` must be an array of {name, enabled?, rules?}.",
        ));
    };
    let clean = crate::config::sanitize_rule_groups(parsed);
    for group in &clean {
        let bad = rules::invalid_rule_lines(&group.rules);
        if !bad.is_empty() {
            let lines = bad
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(tool_error(format!(
                "Invalid rule syntax in group `{}` on line(s): {lines}",
                group.name
            )));
        }
    }
    Ok(Some(clean))
}

fn tool_set_rules(hub: &Hub, args: &Value) -> Value {
    let new_rules = args.get("rules").and_then(Value::as_str);
    let rules_enabled = args.get("rules_enabled").and_then(Value::as_bool);
    let breaks_enabled = args.get("breaks_enabled").and_then(Value::as_bool);
    let parsed_groups = match parse_mcp_rule_groups(args) {
        Ok(groups) => groups,
        Err(err_reply) => return err_reply,
    };
    if new_rules.is_none()
        && parsed_groups.is_none()
        && rules_enabled.is_none()
        && breaks_enabled.is_none()
    {
        return tool_error(
            "Provide `rules`, `rule_groups`, and/or at least one switch (`rules_enabled`, `breaks_enabled`).",
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
    if let Some(ref groups) = parsed_groups {
        hub.set_rule_groups(groups);
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
            if let Some(ref groups) = parsed_groups {
                config.rule_groups = groups.clone();
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
        "effective_rules_count": hub.effective_rules_count(),
        "rules": current_rules,
        "rule_groups": hub.rule_groups(),
        "apply_rules": hub.rules_enabled(),
        "apply_breaks": hub.breaks_enabled(),
    });
    if let Err(err) = save_res {
        resp["warning"] = json!(format!("Failed to persist rules to disk: {err}"));
    }
    tool_json(&resp)
}

fn break_side_label(side: BreakSide) -> &'static str {
    match side {
        BreakSide::Request => "request",
        BreakSide::Response => "response",
    }
}

fn tool_clear_flows(hub: &Hub, args: &Value) -> Value {
    let keep_pinned = args
        .get("keep_pinned")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let cleared = if keep_pinned {
        hub.clear_unpinned()
    } else {
        let total = hub.with_session(|s| s.len());
        hub.clear_session();
        total
    };
    let remaining = hub.with_session(|s| s.len());
    tool_json(&json!({
        "ok": true,
        "keep_pinned": keep_pinned,
        "cleared": cleared,
        "remaining": remaining,
    }))
}

fn tool_manage_breaks(hub: &Hub, args: &Value) -> Value {
    let Some(action) = args.get("action").and_then(Value::as_str) else {
        return tool_error(
            "Missing required `action` (`list`, `continue`, `continue_all`, `drop`).",
        );
    };
    match action {
        "list" => {
            let queue = hub.paused_queue();
            let items: Vec<Value> = hub.with_session(|session| {
                queue
                    .iter()
                    .map(|&(id, side)| {
                        let flow = session.get(id);
                        json!({
                            "id": id,
                            "side": break_side_label(side),
                            "method": flow.map(|f| f.method.as_str()).unwrap_or(""),
                            "url": flow.map(|f| f.url.as_str()).unwrap_or(""),
                            "status": flow.and_then(|f| f.status),
                        })
                    })
                    .collect()
            });
            tool_json(&json!({
                "paused_count": items.len(),
                "queue": items,
            }))
        }
        "continue_all" => {
            let released = hub.resolve_all_breaks();
            tool_json(&json!({
                "ok": true,
                "action": action,
                "released": released,
                "remaining": hub.paused_queue_len(),
            }))
        }
        "continue" | "drop" => {
            if args
                .get("id")
                .is_some_and(|v| !v.is_null() && v.as_u64().is_none())
            {
                return tool_error("`id` must be a non-negative integer flow ID.");
            }
            if let Some(id) = args.get("id").and_then(Value::as_u64)
                && hub.paused().map(|(hid, _)| hid) != Some(id)
                && !hub.promote_break(id)
            {
                return tool_error(format!(
                    "Flow #{id} is not currently paused at a breakpoint."
                ));
            }
            let Some((head_id, side)) = hub.paused() else {
                return tool_error("No flow is currently paused at a breakpoint.");
            };
            let status = if action == "continue" {
                match args.get("status") {
                    None | Some(Value::Null) => None,
                    Some(v) => {
                        let Some(code) = v.as_u64().and_then(|n| u16::try_from(n).ok()) else {
                            return tool_error("`status` must be an integer between 100 and 599.");
                        };
                        if !(100..=599).contains(&code) {
                            return tool_error("`status` must be an integer between 100 and 599.");
                        }
                        Some(code)
                    }
                }
            } else {
                None
            };
            let headers = match args.get("headers") {
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
            let body = if action == "continue" {
                args.get("body")
                    .and_then(Value::as_str)
                    .map(|s| s.as_bytes().to_vec())
            } else {
                None
            };
            let ok = hub.resolve_break(BreakDecision {
                drop: action == "drop",
                headers: if action == "continue" { headers } else { None },
                body,
                status,
            });
            tool_json(&json!({
                "ok": ok,
                "id": head_id,
                "side": break_side_label(side),
                "action": action,
                "remaining": hub.paused_queue_len(),
            }))
        }
        other => tool_error(format!(
            "Unsupported `action`: {other}. Expected one of: list, continue, continue_all, drop."
        )),
    }
}

fn tool_generate_openapi(hub: &Hub, args: &Value) -> Value {
    let filter = args.get("filter").and_then(Value::as_str).unwrap_or("");
    let redact_on = args.get("redact").and_then(Value::as_bool).unwrap_or(false);
    let want_yaml = args
        .get("format")
        .and_then(Value::as_str)
        .is_some_and(|s| s.eq_ignore_ascii_case("yaml"));
    let (matched, _) = hub.matching_ids(filter);
    let flows = hub.with_session(|session| {
        matched
            .into_iter()
            .filter_map(|(id, _)| session.hydrated(id))
            .collect::<Vec<_>>()
    });
    let redact = crate::redact::RedactOptions {
        enabled: redact_on,
        ..Default::default()
    };
    let spec = crate::openapi::build_openapi_value(&flows, &redact);
    let (paths, operations) = crate::openapi::count_openapi_operations(&spec);
    if want_yaml {
        let yaml = crate::openapi::json_value_to_yaml(&spec);
        tool_json(&json!({
            "paths": paths,
            "operations": operations,
            "format": "yaml",
            "yaml": yaml,
            "spec": spec,
        }))
    } else {
        tool_json(&json!({
            "paths": paths,
            "operations": operations,
            "spec": spec,
        }))
    }
}

pub(crate) fn parse_headers_arg(val: &Value) -> Option<Vec<(String, String)>> {
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

pub(crate) fn tool_json(data: &Value) -> Value {
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

pub(crate) fn tool_error(msg: impl Into<String>) -> Value {
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
    let _ = stream.set_read_timeout(Some(Duration::from_secs(40)));
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
#[path = "mcp_tests.rs"]
mod tests;
