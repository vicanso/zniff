//! End-to-end integration tests across live HTTP proxy forwarding, `zniff.flows.*`
//! cross-flow state correlation, `respondWith` short-circuiting, fail-open behavior,
//! zero unmatched-traffic script overhead, and MCP script-hook tools.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::ca::Ca;
use crate::hook_spec::{HookStage, ScriptHookConfig};
use crate::mcp;
use crate::proxy::{self, Hub};

struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    fn new(tag: &str) -> Self {
        let nanos = crate::clock::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("zniff-e2e-{tag}-{}-{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("mkdir");
        Self { path }
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

async fn send_proxy_request(proxy_addr: std::net::SocketAddr, req: &str) -> String {
    let mut sock = TcpStream::connect(proxy_addr).await.unwrap();
    sock.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    sock.read_to_end(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf).into_owned()
}

#[tokio::test]
async fn live_proxy_hook_correlates_prior_flow_mutates_and_short_circuits_with_zero_unmatched_cost()
{
    let dir = ScratchDir::new("proxy");
    let ca = Arc::new(Ca::open(&dir.path).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());

    let origin_hits = Arc::new(AtomicUsize::new(0));
    let origin_hits_bg = Arc::clone(&origin_hits);
    let last_origin_seen = Arc::new(std::sync::Mutex::new(String::new()));
    let last_origin_bg = Arc::clone(&last_origin_seen);

    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = origin.accept().await {
            let hits = Arc::clone(&origin_hits_bg);
            let seen = Arc::clone(&last_origin_bg);
            tokio::spawn(async move {
                let mut buf = [0_u8; 8192];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let raw = String::from_utf8_lossy(&buf[..n]).into_owned();
                hits.fetch_add(1, Ordering::Relaxed);
                *seen.lock().unwrap() = raw.clone();

                let resp = if raw.starts_with("POST http://login.vicanso.dev/auth ")
                    || raw.starts_with("POST /auth ")
                {
                    let body = r#"{"login":"ok"}"#;
                    format!(
                        "HTTP/1.1 200 OK\r\nX-Session-Key: secret-key-99\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                } else if raw.contains("Host: cdn.vicanso.dev") || raw.contains("/ping") {
                    "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\npong"
                        .to_string()
                } else {
                    let body = r#"{"origin":true}"#;
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });

    let hooks_dir = dir.path.join("hooks");
    std::fs::create_dir_all(&hooks_dir).unwrap();
    std::fs::write(
        hooks_dir.join("biz.tsx"),
        r#"
export function onRequest(ctx) {
  if (ctx.path === "/v1/mock") {
    return {
      respondWith: {
        status: 202,
        headers: [["X-Short-Circuit", "true"]],
        body: { mocked: true, env: ctx.vars.env },
      },
      comment: "short-circuit-mock",
    };
  }
  for (const id of zniff.flows.ids("host:login.vicanso.dev 200", 5)) {
    const key = zniff.flows.responseHeader(id, "x-session-key");
    if (key) {
      const mac = zniff.crypto.hmacSha256(zniff.codec.utf8Encode(key), ctx.body);
      return {
        method: "PUT",
        headers: [...ctx.headers, ["X-Session-Hmac", zniff.codec.hexEncode(mac)]],
        body: zniff.codec.utf8Encode("mutated-body"),
        comment: "signed-from-login",
      };
    }
  }
}

export function onResponse(ctx) {
  return {
    status: 201,
    headers: [...ctx.headers, ["X-Post-Hook", "done"]],
    body: JSON.stringify({ wrapped: JSON.parse(zniff.codec.utf8Decode(ctx.body)), env: ctx.vars.env }),
  };
}
"#,
    )
    .unwrap();

    hub.set_rules(&format!(
        "resolve login.vicanso.dev {origin_addr}\n\
         resolve api.vicanso.dev {origin_addr}\n\
         resolve cdn.vicanso.dev {origin_addr}\n"
    ));
    let hook_cfg = ScriptHookConfig {
        id: "biz".into(),
        name: "Biz".into(),
        script: "biz.tsx".into(),
        enabled: true,
        match_pattern: "api.vicanso.dev/v1".into(),
        stages: vec![HookStage::Request, HookStage::Response],
        timeout_ms: 500,
        vars: BTreeMap::from([("env".into(), "qa".into())]),
    };
    let compiled_hook_rules: Vec<_> =
        crate::rules::compile_script_hook_rules(std::slice::from_ref(&hook_cfg))
            .into_iter()
            .map(|entry| entry.rule)
            .collect();
    assert!(!crate::rules::has_request_hooks(
        &compiled_hook_rules,
        "cdn.vicanso.dev",
        "/ping"
    ));
    assert!(
        crate::rules::effect_with_ops(&compiled_hook_rules, "cdn.vicanso.dev", "/ping", &[])
            .hooks
            .is_empty()
    );
    assert!(
        crate::rules::effect_with_ops(&compiled_hook_rules, "login.vicanso.dev", "/auth", &[])
            .hooks
            .is_empty()
    );
    assert!(
        !crate::rules::effect_with_ops(&compiled_hook_rules, "api.vicanso.dev", "/v1/data", &[])
            .hooks
            .is_empty()
    );
    hub.set_script_hooks(&[hook_cfg]);
    hub.set_rules_enabled(true);

    let listener = proxy::listen().await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();
    proxy::spawn_test_serve(listener, Arc::clone(&hub));

    // 1) Unmatched traffic (cdn.vicanso.dev) must bypass hooks with zero hook overhead.
    let cdn_resp = send_proxy_request(
        proxy_addr,
        "GET http://cdn.vicanso.dev/ping HTTP/1.1\r\nHost: cdn.vicanso.dev\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(cdn_resp.ends_with("pong"), "{cdn_resp}");
    assert!(!cdn_resp.contains("X-Post-Hook"), "{cdn_resp}");

    // 2) Login request to login.vicanso.dev/auth seeds x-session-key in session without hooks.
    let login_resp = send_proxy_request(
        proxy_addr,
        "POST http://login.vicanso.dev/auth HTTP/1.1\r\nHost: login.vicanso.dev\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
    )
    .await;
    assert!(login_resp.contains("secret-key-99"), "{login_resp}");
    assert!(!login_resp.contains("X-Post-Hook"), "{login_resp}");

    // 3) Matched request to api.vicanso.dev/v1/data triggers onRequest + onResponse!
    let hits_before_api = origin_hits.load(Ordering::Relaxed);
    let api_resp = send_proxy_request(
        proxy_addr,
        "POST http://api.vicanso.dev/v1/data HTTP/1.1\r\nHost: api.vicanso.dev\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc",
    )
    .await;
    assert!(api_resp.starts_with("HTTP/1.1 201 Created"), "{api_resp}");
    assert!(api_resp.contains("X-Post-Hook: done"), "{api_resp}");
    assert!(api_resp.contains(r#""origin":true"#), "{api_resp}");
    assert!(api_resp.contains(r#""env":"qa""#), "{api_resp}");
    assert_eq!(origin_hits.load(Ordering::Relaxed), hits_before_api + 1);
    let upstream_raw = last_origin_seen.lock().unwrap().clone();
    assert!(upstream_raw.starts_with("PUT "), "{upstream_raw}");
    assert!(
        upstream_raw
            .to_ascii_lowercase()
            .contains("x-session-hmac:"),
        "{upstream_raw}"
    );
    assert!(upstream_raw.ends_with("mutated-body"), "{upstream_raw}");

    // 4) Short-circuit respondWith on api.vicanso.dev/v1/mock must NOT hit origin.
    let hits_before_mock = origin_hits.load(Ordering::Relaxed);
    let mock_resp = send_proxy_request(
        proxy_addr,
        "GET http://api.vicanso.dev/v1/mock HTTP/1.1\r\nHost: api.vicanso.dev\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(origin_hits.load(Ordering::Relaxed), hits_before_mock);
    assert!(
        mock_resp.starts_with("HTTP/1.1 202 Accepted"),
        "{mock_resp}"
    );
    assert!(mock_resp.contains("X-Short-Circuit: true"), "{mock_resp}");
    assert!(mock_resp.contains(r#""mocked":true"#), "{mock_resp}");

    // Verify flows stored in Session reflect unmatched bypass, method mutation, and hook comments.
    let flows = hub.with_session(|s| s.flows());
    let cdn_flow = flows.iter().find(|f| f.path == "/ping").unwrap();
    assert!(
        !cdn_flow.rule_hit.contains("hook"),
        "unmatched flow unexpectedly hit hook: {}",
        cdn_flow.rule_hit
    );
    assert!(cdn_flow.comment.is_empty(), "{}", cdn_flow.comment);
    let login_flow = flows.iter().find(|f| f.path == "/auth").unwrap();
    assert!(
        !login_flow.rule_hit.contains("hook"),
        "unmatched login flow unexpectedly hit hook: {}",
        login_flow.rule_hit
    );
    assert!(login_flow.comment.is_empty(), "{}", login_flow.comment);
    let api_flow = flows.iter().find(|f| f.path == "/v1/data").unwrap();
    assert!(api_flow.rule_hit.contains("hook"), "{}", api_flow.rule_hit);
    assert_eq!(api_flow.method, "PUT");
    assert!(
        api_flow.comment.contains("signed-from-login"),
        "{}",
        api_flow.comment
    );
    let mock_flow = flows.iter().find(|f| f.path == "/v1/mock").unwrap();
    assert_eq!(mock_flow.status, Some(202));
    assert!(mock_flow.comment.contains("short-circuit-mock"));
}

#[tokio::test]
async fn mcp_list_and_run_script_hooks_annotates_flows() {
    let dir = ScratchDir::new("mcp");
    let ca = Arc::new(Ca::open(&dir.path).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());

    let hooks_dir = dir.path.join("hooks");
    std::fs::create_dir_all(&hooks_dir).unwrap();
    std::fs::write(
        hooks_dir.join("audit.tsx"),
        r#"
export function onRun(ctx) {
  const ids = zniff.flows.ids(ctx.filter);
  for (const id of ids) {
    zniff.flows.setComment(id, "mcp-audited");
  }
  return {
    summary: `audited ${ids.length}`,
    cards: <Section title="MCP Audit"><Badge tone="ok">{String(ids.length)}</Badge></Section>,
  };
}
"#,
    )
    .unwrap();
    hub.set_script_hooks(&[ScriptHookConfig {
        id: "audit".into(),
        name: "Audit API".into(),
        script: "audit.tsx".into(),
        enabled: true,
        match_pattern: String::new(),
        stages: vec![HookStage::Run],
        timeout_ms: 500,
        vars: BTreeMap::new(),
    }]);

    let flow_id = hub.open(crate::session::FlowDraft {
        protocol: crate::session::Protocol::Http11,
        method: "GET".into(),
        host: "api.vicanso.dev".into(),
        path: "/v1/users".into(),
        url: "https://api.vicanso.dev/v1/users".into(),
        request_headers: vec![],
    });
    hub.mark_finished(flow_id);

    let list_req = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": "list_script_hooks", "arguments": {} }
    }))
    .unwrap();
    let list_raw = mcp::handle_http_jsonrpc(Arc::clone(&hub), &list_req)
        .await
        .unwrap();
    let list_json: Value = serde_json::from_slice(&list_raw).unwrap();
    let list_text = list_json["result"]["content"][0]["text"].as_str().unwrap();
    let list_payload: Value = serde_json::from_str(list_text).unwrap();
    assert_eq!(list_payload["count"], 1);
    assert_eq!(list_payload["hooks"][0]["id"], "audit");
    assert!(hooks_dir.join("zniff-hook.d.ts").is_file());

    let run_bytes = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "run_script_hook",
            "arguments": { "id": "audit", "filter": "host:api.vicanso.dev" }
        }
    }))
    .unwrap();
    let run_raw = mcp::handle_http_jsonrpc(Arc::clone(&hub), &run_bytes)
        .await
        .unwrap();
    let run_json: Value = serde_json::from_slice(&run_raw).unwrap();
    let run_text = run_json["result"]["content"][0]["text"].as_str().unwrap();
    let run_payload: Value = serde_json::from_str(run_text).unwrap();
    assert_eq!(run_payload["ok"], true);
    assert_eq!(run_payload["summary"], "audited 1");
    assert_eq!(run_payload["cards"][0]["title"], "MCP Audit");

    hub.with_session(|s| {
        assert_eq!(s.get(flow_id).unwrap().comment, "mcp-audited");
    });
}
