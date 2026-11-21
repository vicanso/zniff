use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::ca::Ca;
use crate::mcp_tools::{BODY_PREVIEW_CAP, format_body_preview};
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

async fn call_jsonrpc(hub: Arc<Hub>, req: Value) -> Value {
    let bytes = serde_json::to_vec(&req).unwrap();
    let resp = handle_http_jsonrpc(hub, &bytes).await.unwrap();
    serde_json::from_slice(&resp).unwrap()
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

    // 1. initialize advertises tools, resources, and prompts capabilities
    let init_req = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#;
    let init_res: Value =
        serde_json::from_slice(&handle_http_jsonrpc(hub.clone(), init_req).await.unwrap()).unwrap();
    assert_eq!(init_res["result"]["serverInfo"]["name"], "zniff");
    assert!(init_res["result"]["capabilities"]["tools"].is_object());
    assert!(init_res["result"]["capabilities"]["resources"].is_object());
    assert!(init_res["result"]["capabilities"]["prompts"].is_object());

    // 2. notification returns None
    let notif = br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    assert!(handle_http_jsonrpc(hub.clone(), notif).await.is_none());

    // 3. list_flows with filter
    let list_req = br#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_flows","arguments":{"filter":"5xx host:api.example.com"}}}"#;
    let list_res: Value =
        serde_json::from_slice(&handle_http_jsonrpc(hub.clone(), list_req).await.unwrap()).unwrap();
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
        serde_json::from_slice(&handle_http_jsonrpc(hub.clone(), set_req).await.unwrap()).unwrap();
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
        serde_json::from_slice(&handle_http_jsonrpc(hub.clone(), set_req).await.unwrap()).unwrap();
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
        serde_json::from_slice(&handle_http_jsonrpc(hub.clone(), bad_req).await.unwrap()).unwrap();
    assert_eq!(bad_res["result"]["isError"], true);
    assert!(hub.rules_enabled());
    assert_eq!(hub.rules_revision(), 1);

    // Rule groups roundtrip through set_rules & get_rules
    let group_req = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 13,
        "method": "tools/call",
        "params": {
            "name": "set_rules",
            "arguments": {
                "rule_groups": [
                    {"name": "Dev", "enabled": true, "rules": "header api.vicanso.dev res set X-Dev 1"},
                    {"name": "Muted", "enabled": false, "rules": "block test.vicanso.dev"}
                ]
            }
        }
    }))
    .unwrap();
    let group_res: Value =
        serde_json::from_slice(&handle_http_jsonrpc(hub.clone(), &group_req).await.unwrap())
            .unwrap();
    let group_payload = parse_tool_payload(&group_res["result"]);
    assert_eq!(group_payload["effective_rules_count"], 2);
    assert_eq!(group_payload["rule_groups"].as_array().unwrap().len(), 2);

    let get_req = br#"{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"get_rules","arguments":{}}}"#;
    let get_res: Value =
        serde_json::from_slice(&handle_http_jsonrpc(hub.clone(), get_req).await.unwrap()).unwrap();
    let get_payload = parse_tool_payload(&get_res["result"]);
    assert_eq!(get_payload["effective_rules_count"], 2);
    assert_eq!(get_payload["rule_groups"][0]["name"], "Dev");
    assert_eq!(get_payload["rule_groups"][1]["enabled"], false);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn format_body_preview_preserves_multibyte_utf8_at_32kb_boundary_and_decodes_binary() {
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

    let msgpack = [0x81u8, 0xa2, b'i', b'd', 0x2a];
    let mp_preview = format_body_preview(&msgpack, false, None, Some("application/msgpack"));
    assert_eq!(mp_preview["encoding"], "hex");
    let decoded = mp_preview["decoded_preview"].as_str().unwrap();
    assert!(decoded.contains("\"id\": 42"), "{decoded}");

    let proto = b"\x08\x07\x12\x05zniff";
    let pb_preview = format_body_preview(proto, false, None, Some("application/x-protobuf"));
    let pb_decoded = pb_preview["decoded_preview"].as_str().unwrap();
    assert!(pb_decoded.contains("zniff"), "{pb_decoded}");

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
            let body =
                format!(r#"{{"operationName":"{op_name}","query":"query {op_name} {{ ok }}"}}"#);
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

    let ann = parse_tool_payload(&tool_annotate_flow(
        &hub,
        &json!({ "id": id2, "comment": "investigate 502", "pinned": true }),
    ));
    assert_eq!(ann["ok"], true);
    assert_eq!(ann["comment"], "investigate 502");
    assert_eq!(ann["pinned"], true);

    let summary = parse_tool_payload(&tool_get_session_summary(&hub, &json!({})));
    assert_eq!(summary["session_total"], 2);
    assert_eq!(summary["matched_count"], 2);
    assert_eq!(summary["stats"]["s2xx"], 1);
    assert_eq!(summary["stats"]["s5xx"], 1);
    assert_eq!(summary["facets"]["domains"].as_array().unwrap().len(), 2);
    assert_eq!(summary["facets"]["apps"].as_array().unwrap().len(), 2);

    let filtered = parse_tool_payload(&tool_get_session_summary(
        &hub,
        &json!({ "filter": "note:investigate pin:" }),
    ));
    assert_eq!(filtered["matched_count"], 1);
    assert_eq!(filtered["stats"]["s5xx"], 1);

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
client-cert mtls.example.com ./client.crt ./client.key
resolve api.example.com 127.0.0.1:8443";
    assert!(
        rules::invalid_rule_lines(sample_rules).is_empty(),
        "invalid lines: {:?}",
        rules::invalid_rule_lines(sample_rules)
    );
    assert_eq!(rules::parse_rules(sample_rules).len(), 19);

    let tools = tools_list_value();
    let names: Vec<&str> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(names.contains(&"get_session_summary"));
    assert!(names.contains(&"annotate_flow"));
    assert!(names.contains(&"clear_flows"));
    assert!(names.contains(&"manage_breaks"));
    assert!(names.contains(&"generate_openapi"));
    assert!(names.contains(&"wait_for_flow"));
    assert!(names.contains(&"mock_flow"));
    assert!(names.contains(&"diff_flows"));
    assert_eq!(names.len(), 13);

    let openapi_res = parse_tool_payload(&tool_generate_openapi(&hub, &json!({ "filter": "" })));
    assert_eq!(openapi_res["spec"]["openapi"], "3.0.3");
    assert!(openapi_res["operations"].as_u64().unwrap_or(0) >= 1);

    assert_eq!(
        crate::i18n::localize_error(
            crate::i18n::Lang::En,
            "客户端证书加载失败: 读取客户端证书 /tmp/a.crt: 证书文件 /tmp/a.crt 中未找到 CERTIFICATE",
        ),
        "Failed to load client certificate: reading client certificate /tmp/a.crt: no CERTIFICATE block found in /tmp/a.crt"
    );
    let _ = mcp_localize_error(
        "客户端证书加载失败: 读取客户端证书 /tmp/a.crt: 证书文件 /tmp/a.crt 中未找到 CERTIFICATE",
    );
    hub.with_session(|s| {
        let info = crate::session::ConnInfo {
            client_cert: "/tmp/mtls-client.pem".into(),
            ..Default::default()
        };
        s.set_conn_info(id1, info);
    });
    let detail = parse_tool_payload(&tool_get_flow_detail(&hub, &json!({ "id": id1 })));
    assert_eq!(detail["timing"]["client_cert"], "/tmp/mtls-client.pem");
    assert_eq!(detail["rule_hit"], "client-cert");
    assert_eq!(
        detail["rule_hits"],
        json!([{ "group": null, "line": 0, "verb": "client-cert" }])
    );

    hub.with_session(|s| {
        s.set_rule_hit_details(
            id1,
            "replace header",
            vec![
                crate::session::RuleHit {
                    group_name: None,
                    line_no: 3,
                    verb: "replace".into(),
                },
                crate::session::RuleHit {
                    group_name: Some("dev".into()),
                    line_no: 1,
                    verb: "header".into(),
                },
            ],
        );
    });
    let detail2 = parse_tool_payload(&tool_get_flow_detail(&hub, &json!({ "id": id1 })));
    assert_eq!(detail2["rule_hit"], "replace header");
    assert_eq!(
        detail2["rule_hits"],
        json!([
            { "group": null, "line": 3, "verb": "replace" },
            { "group": "dev", "line": 1, "verb": "header" }
        ])
    );

    let listed = parse_tool_payload(&tool_manage_breaks(&hub, &json!({ "action": "list" })));
    assert_eq!(listed["paused_count"], 0);
    let cont_err = tool_manage_breaks(&hub, &json!({ "action": "continue" }));
    assert_eq!(cont_err["isError"], true);
    let cont_all = parse_tool_payload(&tool_manage_breaks(
        &hub,
        &json!({ "action": "continue_all" }),
    ));
    assert_eq!(cont_all["released"], 0);

    let kept = parse_tool_payload(&tool_clear_flows(&hub, &json!({ "keep_pinned": true })));
    assert_eq!(kept["cleared"], 1);
    assert_eq!(kept["remaining"], 1);
    let cleared_all = parse_tool_payload(&tool_clear_flows(&hub, &json!({ "keep_pinned": false })));
    assert_eq!(cleared_all["cleared"], 1);
    assert_eq!(cleared_all["remaining"], 0);
}

#[tokio::test]
async fn mcp_wait_for_flow_matches_async_arrival_after_id_and_timeout() {
    let hub = test_hub();

    // 1. Clean timeout when no matching flow arrives (clamps below 100ms to 100ms).
    let timeout_res = call_jsonrpc(
        hub.clone(),
        json!({
            "jsonrpc": "2.0",
            "id": 100,
            "method": "tools/call",
            "params": {
                "name": "wait_for_flow",
                "arguments": { "filter": "host:never.example.com", "timeout_ms": 10 }
            }
        }),
    )
    .await;
    let timeout_payload = parse_tool_payload(&timeout_res["result"]);
    assert_eq!(timeout_payload["matched"], false);
    assert_eq!(timeout_payload["timeout_ms"], 100);

    // 2. Existing flow is ignored when `after_id` is set, and a newly arriving flow wakes the waiter.
    let first_id = hub.with_session(|s| {
        let id = s.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "GET".into(),
            host: "events.example.com".into(),
            path: "/hook".into(),
            url: "https://events.example.com/hook".into(),
            request_headers: vec![],
        });
        s.set_response(id, 200, vec![], None);
        s.mark_finished(id);
        id
    });

    let hub_bg = hub.clone();
    let producer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(40)).await;
        hub_bg.with_session(|s| {
            let id = s.open(FlowDraft {
                protocol: Protocol::Http2,
                method: "POST".into(),
                host: "events.example.com".into(),
                path: "/hook".into(),
                url: "https://events.example.com/hook".into(),
                request_headers: vec![],
            });
            s.set_response(id, 201, vec![], None);
            s.mark_finished(id);
            id
        })
    });

    let wait_res = call_jsonrpc(
        hub.clone(),
        json!({
            "jsonrpc": "2.0",
            "id": 101,
            "method": "tools/call",
            "params": {
                "name": "wait_for_flow",
                "arguments": {
                    "filter": "host:events.example.com path:/hook",
                    "timeout_ms": 2000,
                    "after_id": first_id
                }
            }
        }),
    )
    .await;
    let second_id = producer.await.unwrap();
    let wait_payload = parse_tool_payload(&wait_res["result"]);
    assert_eq!(wait_payload["matched"], true);
    assert_eq!(wait_payload["flow"]["id"], second_id);
    assert_eq!(wait_payload["flow"]["method"], "POST");
}

#[tokio::test]
async fn mcp_mock_flow_and_alias_create_mock_file_and_rules() {
    let dir = std::env::temp_dir().join(format!("zniff-mcp-mock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());

    let id = hub.with_session(|s| {
        let id = s.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "GET".into(),
            host: "api.example.com".into(),
            path: "/v1/profile?tab=info".into(),
            url: "https://api.example.com/v1/profile?tab=info".into(),
            request_headers: vec![],
        });
        s.set_response(
            id,
            200,
            vec![("Content-Type".into(), "application/json".into())],
            None,
        );
        s.append_body(id, Direction::Response, br#"{"name":"alice"}"#);
        s.mark_finished(id);
        id
    });

    // 1. mock_flow with custom body, status, and headers
    let mock_res = call_jsonrpc(
        hub.clone(),
        json!({
            "jsonrpc": "2.0",
            "id": 201,
            "method": "tools/call",
            "params": {
                "name": "mock_flow",
                "arguments": {
                    "flow_id": id,
                    "status": 201,
                    "body": "{\"mocked\":true}",
                    "headers": { "X-Mock": "yes" }
                }
            }
        }),
    )
    .await;
    let mock_payload = parse_tool_payload(&mock_res["result"]);
    assert_eq!(mock_payload["ok"], true);
    assert_eq!(mock_payload["rule_type"], "map-local");
    assert_eq!(mock_payload["pattern"], "api.example.com/v1/profile");
    let mock_file = mock_payload["mock_file"].as_str().unwrap();
    assert!(
        mock_file.contains("mocks"),
        "expected mocks dir: {mock_file}"
    );
    assert_eq!(
        std::fs::read_to_string(mock_file).unwrap(),
        "{\"mocked\":true}"
    );
    let rules_text = hub.rules_text();
    assert!(rules_text.contains("map-local api.example.com/v1/profile"));
    assert!(rules_text.contains("status=201"));
    assert!(rules_text.contains("header api.example.com/v1/profile res set X-Mock yes"));

    // 2. add_rule_from_flow alias for delay and block
    let delay_res = call_jsonrpc(
        hub.clone(),
        json!({
            "jsonrpc": "2.0",
            "id": 202,
            "method": "tools/call",
            "params": {
                "name": "add_rule_from_flow",
                "arguments": {
                    "flow_id": id,
                    "rule_type": "delay",
                    "delay": 250
                }
            }
        }),
    )
    .await;
    let delay_payload = parse_tool_payload(&delay_res["result"]);
    assert_eq!(delay_payload["ok"], true);
    assert!(
        hub.rules_text()
            .contains("delay api.example.com/v1/profile 250ms")
    );

    let block_res = call_jsonrpc(
        hub.clone(),
        json!({
            "jsonrpc": "2.0",
            "id": 203,
            "method": "tools/call",
            "params": {
                "name": "add_rule_from_flow",
                "arguments": {
                    "flow_id": id,
                    "rule_type": "block",
                    "status": 451
                }
            }
        }),
    )
    .await;
    let block_payload = parse_tool_payload(&block_res["result"]);
    assert_eq!(block_payload["ok"], true);
    assert!(
        hub.rules_text()
            .contains("block api.example.com/v1/profile status=451")
    );
    assert!(rules::invalid_rule_lines(&hub.rules_text()).is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn mcp_diff_flows_reports_url_header_and_body_differences() {
    let hub = test_hub();
    let (id1, id2) = hub.with_session(|s| {
        let id1 = s.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "POST".into(),
            host: "api.example.com".into(),
            path: "/v1/items".into(),
            url: "https://api.example.com/v1/items".into(),
            request_headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("X-Version".into(), "1".into()),
            ],
        });
        s.append_body(id1, Direction::Request, br#"{"count":1,"name":"a"}"#);
        s.set_response(
            id1,
            200,
            vec![("Content-Type".into(), "application/json".into())],
            None,
        );
        s.append_body(id1, Direction::Response, br#"{"ok":true,"total":10}"#);
        s.mark_finished(id1);

        let id2 = s.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "POST".into(),
            host: "api.example.com".into(),
            path: "/v2/items".into(),
            url: "https://api.example.com/v2/items".into(),
            request_headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("X-Version".into(), "2".into()),
            ],
        });
        s.append_body(id2, Direction::Request, br#"{"count":2,"name":"a"}"#);
        s.set_response(
            id2,
            500,
            vec![("Content-Type".into(), "application/json".into())],
            None,
        );
        s.append_body(id2, Direction::Response, br#"{"ok":false,"total":10}"#);
        s.mark_finished(id2);
        (id1, id2)
    });

    // Self-diff is identical
    let same_res = call_jsonrpc(
        hub.clone(),
        json!({
            "jsonrpc": "2.0",
            "id": 300,
            "method": "tools/call",
            "params": {
                "name": "diff_flows",
                "arguments": { "base_id": id1, "target_id": id1 }
            }
        }),
    )
    .await;
    let same_payload = parse_tool_payload(&same_res["result"]);
    assert_eq!(same_payload["identical"], true);

    // Cross-flow diff highlights url, status, headers, and bodies
    let diff_res = call_jsonrpc(
        hub.clone(),
        json!({
            "jsonrpc": "2.0",
            "id": 301,
            "method": "tools/call",
            "params": {
                "name": "diff_flows",
                "arguments": { "base_id": id1, "target_id": id2 }
            }
        }),
    )
    .await;
    let diff_payload = parse_tool_payload(&diff_res["result"]);
    assert_eq!(diff_payload["identical"], false);
    assert_eq!(diff_payload["url"]["changed"], true);
    assert_eq!(diff_payload["url"]["status_changed"], true);
    assert_eq!(diff_payload["url"]["base_status"], 200);
    assert_eq!(diff_payload["url"]["target_status"], 500);
    assert_eq!(diff_payload["request_headers"]["changed"], true);
    assert_eq!(diff_payload["request_body"]["changed"], true);
    assert_eq!(diff_payload["response_body"]["changed"], true);
    assert!(diff_payload["response_body"]["added"].as_u64().unwrap() >= 1);
    assert!(diff_payload["response_body"]["removed"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn mcp_resources_list_and_read_work() {
    let hub = test_hub();
    hub.set_rules("cors api.example.com");
    hub.with_session(|s| {
        let id = s.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "GET".into(),
            host: "api.example.com".into(),
            path: "/health".into(),
            url: "https://api.example.com/health".into(),
            request_headers: vec![],
        });
        s.set_response(id, 200, vec![], None);
        s.mark_finished(id);
    });

    let list_res = call_jsonrpc(
        hub.clone(),
        json!({ "jsonrpc": "2.0", "id": 400, "method": "resources/list", "params": {} }),
    )
    .await;
    let uris: Vec<&str> = list_res["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["uri"].as_str())
        .collect();
    assert!(uris.contains(&"zniff://flows/current"));
    assert!(uris.contains(&"zniff://rules/active"));

    let flows_res = call_jsonrpc(
        hub.clone(),
        json!({
            "jsonrpc": "2.0",
            "id": 401,
            "method": "resources/read",
            "params": { "uri": "zniff://flows/current" }
        }),
    )
    .await;
    let flows_text = flows_res["result"]["contents"][0]["text"].as_str().unwrap();
    let flows_json: Value = serde_json::from_str(flows_text).unwrap();
    assert_eq!(flows_json["session_total"], 1);
    assert_eq!(flows_json["flows"][0]["path"], "/health");

    let rules_res = call_jsonrpc(
        hub.clone(),
        json!({
            "jsonrpc": "2.0",
            "id": 402,
            "method": "resources/read",
            "params": { "uri": "zniff://rules/active" }
        }),
    )
    .await;
    let rules_text = rules_res["result"]["contents"][0]["text"].as_str().unwrap();
    let rules_json: Value = serde_json::from_str(rules_text).unwrap();
    assert_eq!(rules_json["rules"], "cors api.example.com");
    assert_eq!(rules_json["effective_rules_count"], 1);

    let err_res = call_jsonrpc(
        hub.clone(),
        json!({
            "jsonrpc": "2.0",
            "id": 403,
            "method": "resources/read",
            "params": { "uri": "zniff://unknown" }
        }),
    )
    .await;
    assert_eq!(err_res["error"]["code"], -32602);
}

#[tokio::test]
async fn mcp_prompts_list_and_get_redact_flow_context() {
    let hub = test_hub();

    let list_res = call_jsonrpc(
        hub.clone(),
        json!({ "jsonrpc": "2.0", "id": 500, "method": "prompts/list", "params": {} }),
    )
    .await;
    let prompts = list_res["result"]["prompts"].as_array().unwrap();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0]["name"], "analyze_failed_flow");

    // Empty session returns a graceful prompt message
    let empty_res = call_jsonrpc(
        hub.clone(),
        json!({
            "jsonrpc": "2.0",
            "id": 501,
            "method": "prompts/get",
            "params": { "name": "analyze_failed_flow" }
        }),
    )
    .await;
    let empty_text = empty_res["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap();
    assert!(empty_text.contains("No captured HTTP flows"));

    // Add a healthy flow and a failing flow with secrets
    let (_ok_id, fail_id) = hub.with_session(|s| {
        let ok_id = s.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "GET".into(),
            host: "api.example.com".into(),
            path: "/ok".into(),
            url: "https://api.example.com/ok".into(),
            request_headers: vec![],
        });
        s.set_response(ok_id, 200, vec![], None);
        s.mark_finished(ok_id);

        let fail_id = s.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "POST".into(),
            host: "api.example.com".into(),
            path: "/v1/pay?token=topsecret999".into(),
            url: "https://api.example.com/v1/pay?token=topsecret999".into(),
            request_headers: vec![
                ("Authorization".into(), "Bearer super-secret-token".into()),
                ("Content-Type".into(), "application/json".into()),
            ],
        });
        s.append_body(fail_id, Direction::Request, br#"{"amount":42}"#);
        s.set_response(fail_id, 502, vec![], None);
        s.append_body(fail_id, Direction::Response, br#"{"error":"bad gateway"}"#);
        s.mark_finished(fail_id);
        (ok_id, fail_id)
    });

    // Omitting flow_id automatically selects the failed flow and redacts secrets
    let auto_res = call_jsonrpc(
        hub.clone(),
        json!({
            "jsonrpc": "2.0",
            "id": 502,
            "method": "prompts/get",
            "params": { "name": "analyze_failed_flow" }
        }),
    )
    .await;
    let auto_text = auto_res["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap();
    assert!(auto_text.contains(&format!("# HTTP Flow #{fail_id}:")));
    assert!(auto_text.contains("bad gateway"));
    assert!(!auto_text.contains("super-secret-token"));
    assert!(!auto_text.contains("topsecret999"));

    // Explicit string flow_id also works
    let explicit_res = call_jsonrpc(
        hub.clone(),
        json!({
            "jsonrpc": "2.0",
            "id": 503,
            "method": "prompts/get",
            "params": {
                "name": "analyze_failed_flow",
                "arguments": { "flow_id": fail_id.to_string() }
            }
        }),
    )
    .await;
    assert_eq!(explicit_res["result"]["messages"][0]["role"], "user");

    // Unknown prompt returns -32602
    let unknown_res = call_jsonrpc(
        hub.clone(),
        json!({
            "jsonrpc": "2.0",
            "id": 504,
            "method": "prompts/get",
            "params": { "name": "does_not_exist" }
        }),
    )
    .await;
    assert_eq!(unknown_res["error"]["code"], -32602);
}
