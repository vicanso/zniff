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
                "description": "List captured HTTP(S)/WebSocket flows in zniff. Supports zniff filter expressions (e.g. `host:api.example.com`, `path:/v1`, `status:5xx`, `失败`, `slow:`, `body:error`, `header:auth`, `op:GetUser`, `pin:`, `app:curl`). Newest flows are returned first.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "filter": {
                            "type": "string",
                            "description": "Optional zniff filter expression."
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
                "description": "Get full details of a captured flow by id, including request/response headers, decoded bodies, connection metadata, timing breakdown (DNS/TCP/TLS/Wait/Download), and WebSocket/SSE messages.",
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
                "description": "Set or append zniff rewrite/mock rules (`replace`, `map-remote`, `map-local`, `delay`, `header`, `query`, `json`, `cors`, `throttle`, `break`, `break-res`, `cert`). Validates syntax before applying.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "rules": {
                            "type": "string",
                            "description": "Rules DSL text (one rule per line)."
                        },
                        "append": {
                            "type": "boolean",
                            "description": "When true, appends to existing rules instead of replacing them."
                        }
                    },
                    "required": ["rules"]
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
    let start = matched.len().saturating_sub(limit);
    let items: Vec<Value> = hub.with_session(|session| {
        let mut newest: Vec<_> = matched[start..]
            .iter()
            .filter_map(|(id, _)| session.get(*id).cloned())
            .collect();
        newest.reverse();
        newest
            .into_iter()
            .map(|f| {
                let tb = present::flow_timing_breakdown(&f);
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
    let req_body = format_body_preview(&flow.request_body, flow.request_truncated);
    let res_body = format_body_preview(&flow.response_body, flow.response_truncated);
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
    let Some(new_rules) = args.get("rules").and_then(Value::as_str) else {
        return tool_error("Missing required string parameter `rules`.");
    };
    let append = args.get("append").and_then(Value::as_bool).unwrap_or(false);
    let combined = if append {
        let existing = hub.rules_text();
        if existing.trim().is_empty() {
            new_rules.trim().to_string()
        } else {
            format!("{}\n{}", existing.trim_end(), new_rules.trim())
        }
    } else {
        new_rules.to_string()
    };
    let bad_lines = rules::invalid_rule_lines(&combined);
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
    hub.set_rules(&combined);
    let save_res =
        crate::config::update_mode(hub.ca.dir(), crate::config::is_dev_mode(), |config| {
            config.rules = combined.clone()
        });
    let parsed_count = rules::parse_rules(&combined).len();
    let mut resp = json!({
        "ok": true,
        "active_rule_count": parsed_count,
        "rules": combined,
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

fn format_body_preview(bytes: &[u8], truncated: bool) -> Value {
    if bytes.is_empty() {
        return json!({
            "size": 0,
            "truncated": truncated,
            "text": "",
        });
    }
    let slice = &bytes[..bytes.len().min(BODY_PREVIEW_CAP)];
    let preview_truncated = truncated || bytes.len() > BODY_PREVIEW_CAP;
    match std::str::from_utf8(slice) {
        Ok(text) => json!({
            "size": bytes.len(),
            "truncated": preview_truncated,
            "encoding": "utf-8",
            "text": text,
        }),
        Err(_) => json!({
            "size": bytes.len(),
            "truncated": preview_truncated,
            "encoding": "hex",
            "text": present::hex_dump_raw(slice),
        }),
    }
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
    use super::*;
    use crate::ca::Ca;
    use crate::session::{Direction, FlowDraft, Protocol};

    fn test_hub() -> Arc<Hub> {
        let dir = std::env::temp_dir().join(format!("zniff-mcp-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(Ca::open(&dir).unwrap());
        let hub = Arc::new(Hub::new(ca).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
        hub
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

        let set_req = br#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"set_rules","arguments":{"rules":"cors test.example.com"}}}"#;
        let set_res: Value =
            serde_json::from_slice(&handle_http_jsonrpc(hub.clone(), set_req).await.unwrap())
                .unwrap();
        assert_eq!(set_res["result"]["isError"], false);
        assert_eq!(hub.rules_revision(), 1);

        // Verify rules on disk
        let config_file = dir.join(crate::config::config_file_name(crate::config::is_dev_mode()));
        assert!(config_file.exists());
        let saved_text = std::fs::read_to_string(&config_file).unwrap();
        assert!(saved_text.contains("cors test.example.com"));

        // Syntax error does NOT persist and does NOT increment revision
        let bad_req = br#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"set_rules","arguments":{"rules":"cors test.example.com op=Invalid"}}}"#;
        let bad_res: Value =
            serde_json::from_slice(&handle_http_jsonrpc(hub.clone(), bad_req).await.unwrap())
                .unwrap();
        assert_eq!(bad_res["result"]["isError"], true);
        assert_eq!(hub.rules_revision(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
