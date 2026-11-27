use super::*;
use crate::ca::Ca;
use crate::hook_spec::{PreviewCardNode, PreviewTone, ScriptHookConfig};
use crate::proxy::Hub;
use crate::session::{Direction, FlowDraft, Protocol};

struct TempHookDir {
    path: PathBuf,
}

impl TempHookDir {
    fn new(tag: &str) -> Self {
        let nanos = crate::clock::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "zniff-hook-rt-{tag}-{}-{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp hook dir");
        Self { path }
    }

    fn write_hook(&self, file_name: &str, source: &str) {
        let dir = hooks_dir(&self.path);
        fs::create_dir_all(&dir).expect("create hooks dir");
        fs::write(dir.join(file_name), source).expect("write hook script");
    }
}

impl Drop for TempHookDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn open_test_hub(dir: &Path) -> Arc<Hub> {
    let ca = Arc::new(Ca::open(dir).expect("open ca"));
    Arc::new(Hub::new(ca).expect("hub"))
}

fn record_sample_flow(
    hub: &Hub,
    host: &str,
    path: &str,
    token: &str,
    req_body: &[u8],
    res_body: &[u8],
) -> u64 {
    let id = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: "POST".into(),
        host: host.into(),
        path: path.into(),
        url: format!("https://{host}{path}"),
        request_headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("X-Trace".into(), format!("trace-{host}")),
        ],
    });
    hub.append_body_bytes_for_test(id, Direction::Request, req_body);
    hub.set_response_for_test(
        id,
        200,
        vec![
            ("Content-Type".into(), "application/json".into()),
            ("X-Session-Key".into(), token.into()),
        ],
    );
    hub.append_body_bytes_for_test(id, Direction::Response, res_body);
    hub.mark_finished(id);
    id
}

#[tokio::test]
async fn flows_sdk_reads_inline_and_spilled_bodies_and_mutates_comments_and_pins() {
    let dir = TempHookDir::new("flows-spill");
    let hub = open_test_hub(&dir.path);

    let id_small = record_sample_flow(
        &hub,
        "api.vicanso.dev",
        "/v1/small",
        "tok-small",
        br#"{"hello":"world"}"#,
        br#"{"ok":true}"#,
    );
    // 24 KiB exceeds INLINE_BODY_CAP (16 KiB) so it spills to zniff-spill/ on mark_finished!
    let large_req = vec![b'R'; 24 * 1024];
    let large_res = vec![b'S'; 24 * 1024];
    let id_spilled = record_sample_flow(
        &hub,
        "api.vicanso.dev",
        "/v1/large",
        "tok-spilled",
        &large_req,
        &large_res,
    );
    let _other_id = record_sample_flow(
        &hub,
        "cdn.vicanso.dev",
        "/assets/a.js",
        "ignored",
        b"",
        b"console.log(1)",
    );

    dir.write_hook(
        "inspect.tsx",
        r#"
export function onRun(ctx) {
  const ids = zniff.flows.ids("host:api.vicanso.dev");
  let summaryParts: string[] = [];
  for (const id of ids) {
    const url = zniff.flows.url(id);
    const hdr = zniff.flows.requestHeader(id, "x-trace");
    const reqHdrs = zniff.flows.requestHeaders(id);
    const reqBody = zniff.flows.requestBody(id);
    const resHdr = zniff.flows.responseHeader(id, "x-session-key");
    const resBody = zniff.flows.responseBody(id);
    const info = zniff.flows.info(id);
    summaryParts.push(`${id}:${url}:${hdr}:${reqHdrs?.length}:${reqBody?.byteLength}:${resHdr}:${resBody?.byteLength}:${info?.status}`);
    if (resBody && resBody.byteLength > 20000) {
      zniff.flows.setComment(id, "spilled-inspected");
      zniff.flows.setPinned(id, true);
    }
  }
  return {
    summary: summaryParts.join("|"),
    cards: (
      <Section title="扫描完成">
        <Badge tone="ok">{String(ids.length)}</Badge>
        <KeyValue label="Host" value="api.vicanso.dev" />
      </Section>
    ),
  };
}
"#,
    );

    let report = eval_run_hook_async(
        &dir.path,
        &[],
        hub.hook_session_host(),
        "@inspect",
        "host:api.vicanso.dev",
        Some(id_spilled),
    )
    .await
    .expect("onRun succeeds");

    assert!(report.summary.contains(&format!("{id_spilled}:https://api.vicanso.dev/v1/large:trace-api.vicanso.dev:2:24576:tok-spilled:24576:200")));
    assert!(report.summary.contains(&format!(
        "{id_small}:https://api.vicanso.dev/v1/small:trace-api.vicanso.dev:2:17:tok-small:11:200"
    )));
    assert_eq!(report.cards.len(), 1);
    hub.with_session(|s| {
        assert_eq!(s.get(id_spilled).unwrap().comment, "spilled-inspected");
        assert!(s.get(id_spilled).unwrap().pinned);
        assert!(s.get(id_small).unwrap().comment.is_empty());
    });
}

#[tokio::test]
async fn crypto_codec_aes256gcm_decrypts_in_on_preview_with_tsx_cards() {
    let dir = TempHookDir::new("preview-crypto");
    let hub = open_test_hub(&dir.path);
    ensure_hook_types(&dir.path).expect("write zniff-hook.d.ts");
    assert!(hooks_dir(&dir.path).join("zniff-hook.d.ts").is_file());

    let key = [0x42_u8; 32];
    let nonce = [0x17_u8; 12];
    let plain = br#"{"secret":"vicanso-token","ok":true}"#;
    let encrypted = crate::hook_codec_crypto::aes_gcm_encrypt(
        &ring::aead::AES_256_GCM,
        &key,
        &nonce,
        plain,
        b"",
    )
    .unwrap();
    let gz_plain = crate::hook_codec_crypto::gzip_compress(plain).unwrap();
    let _ = gz_plain;

    dir.write_hook(
        "aes-preview.tsx",
        r#"
import type { PreviewHookContext } from "zniff/hook";
import { Section, Badge, KeyValue, Code } from "zniff/hook";

export function onPreview(ctx: PreviewHookContext) {
  const key = zniff.codec.hexDecode(ctx.vars.aes_key_hex);
  const nonce = zniff.codec.hexDecode(ctx.vars.nonce_hex);
  const raw = zniff.crypto.aes256GcmDecrypt(key, nonce, ctx.res.body);
  const text = zniff.codec.utf8Decode(raw);
  const digest = zniff.codec.hexEncode(zniff.crypto.sha256(raw));
  return {
    res: {
      body: text,
      contentType: "application/json",
      note: "AES-256-GCM decrypted",
    },
    cards: (
      <Section title="解密详情">
        <Badge tone="ok">AES-256-GCM</Badge>
        <KeyValue label="SHA-256" value={digest.slice(0, 16)} />
        <Code language="json">{text}</Code>
      </Section>
    ),
  };
}
"#,
    );

    let cfg = vec![ScriptHookConfig {
        id: "aes-preview".into(),
        name: "AES Preview".into(),
        script: "aes-preview.tsx".into(),
        enabled: true,
        match_pattern: "api.vicanso.dev".into(),
        stages: vec![HookStage::Preview],
        timeout_ms: 500,
        vars: BTreeMap::from([
            (
                "aes_key_hex".into(),
                crate::hook_codec_crypto::hex_encode_bytes(&key),
            ),
            (
                "nonce_hex".into(),
                crate::hook_codec_crypto::hex_encode_bytes(&nonce),
            ),
        ]),
    }];

    let resolved =
        resolve_script_hook(&dir.path, &cfg, "aes-preview", &BTreeMap::new(), 0).unwrap();
    let preview = eval_preview_hook_async(
        hub.hook_session_host(),
        resolved,
        HookPreviewInputOwned {
            flow_id: 9,
            is_response: true,
            method: "POST".into(),
            host: "api.vicanso.dev".into(),
            path: "/v1/cipher".into(),
            url: "https://api.vicanso.dev/v1/cipher".into(),
            status: Some(200),
            req_headers: vec![],
            req_body: vec![],
            res_headers: vec![("Content-Type".into(), "application/octet-stream".into())],
            res_body: encrypted,
        },
    )
    .await
    .expect("onPreview succeeds");

    assert_eq!(preview.side.body.as_deref(), Some(plain.as_slice()));
    assert_eq!(
        preview.side.content_type.as_deref(),
        Some("application/json")
    );
    assert_eq!(preview.side.note.as_deref(), Some("AES-256-GCM decrypted"));
    assert_eq!(preview.cards.len(), 1);
    match &preview.cards[0] {
        PreviewCardNode::Section {
            title, children, ..
        } => {
            assert_eq!(title, "解密详情");
            assert_eq!(children.len(), 3);
            assert!(matches!(
                &children[0],
                PreviewCardNode::Badge {
                    tone: PreviewTone::Ok,
                    ..
                }
            ));
        }
        other => panic!("unexpected root card: {other:?}"),
    }
}

#[tokio::test]
async fn live_request_hook_enforces_body_read_quota_and_circuits_after_three_timeouts() {
    let dir = TempHookDir::new("quota-circuit");
    let hub = open_test_hub(&dir.path);

    let mut flow_ids = Vec::new();
    for i in 0..20 {
        flow_ids.push(record_sample_flow(
            &hub,
            "api.vicanso.dev",
            &format!("/v1/item/{i}"),
            "k",
            b"req",
            b"res",
        ));
    }
    assert_eq!(flow_ids.len(), 20);

    dir.write_hook(
        "quota.tsx",
        r#"
export function onRequest(ctx) {
  for (const id of zniff.flows.ids("host:api.vicanso.dev")) {
    zniff.flows.responseBody(id);
  }
}
"#,
    );
    let resolved_quota =
        resolve_script_hook(&dir.path, &[], "@quota", &BTreeMap::new(), 200).unwrap();
    let err = eval_request_hook_async(
        hub.hook_session_host(),
        resolved_quota,
        HookRequestInputOwned {
            flow_id: 99,
            method: "GET".into(),
            scheme: "https".into(),
            host: "api.vicanso.dev".into(),
            url: "https://api.vicanso.dev/v1/next".into(),
            path: "/v1/next".into(),
            headers: vec![],
            body: vec![],
        },
    )
    .await
    .expect_err("17th body read in live onRequest must hit quota");
    assert!(err.contains("quota exceeded"));

    // 3 consecutive timeouts trip the hook circuit breaker without affecting subsequent calls.
    dir.write_hook(
        "spin.tsx",
        r#"
export function onRequest() {
  while (true) {}
}
"#,
    );
    let resolved_spin = resolve_script_hook(&dir.path, &[], "@spin", &BTreeMap::new(), 15).unwrap();
    crate::script_worker::reset_script_circuit(&resolved_spin.circuit_key);

    for _ in 0..2 {
        let e = eval_request_hook_async(
            hub.hook_session_host(),
            resolved_spin.clone(),
            HookRequestInputOwned {
                flow_id: 100,
                method: "GET".into(),
                scheme: "https".into(),
                host: "api.vicanso.dev".into(),
                url: "https://api.vicanso.dev/v1/spin".into(),
                path: "/v1/spin".into(),
                headers: vec![],
                body: vec![],
            },
        )
        .await
        .expect_err("timeout expected");
        assert!(e.contains("timed out"));
    }
    assert!(!crate::script_worker::is_circuit_tripped(
        &resolved_spin.circuit_key
    ));
    let third = eval_request_hook_async(
        hub.hook_session_host(),
        resolved_spin.clone(),
        HookRequestInputOwned {
            flow_id: 101,
            method: "GET".into(),
            scheme: "https".into(),
            host: "api.vicanso.dev".into(),
            url: "https://api.vicanso.dev/v1/spin".into(),
            path: "/v1/spin".into(),
            headers: vec![],
            body: vec![],
        },
    )
    .await
    .expect_err("3rd timeout trips breaker");
    assert!(third.contains("circuit breaker tripped"));
    assert!(crate::script_worker::is_circuit_tripped(
        &resolved_spin.circuit_key
    ));
    crate::script_worker::reset_script_circuit(&resolved_spin.circuit_key);
}

#[tokio::test]
async fn structured_param_helpers_and_four_stage_views_work_end_to_end() {
    use crate::hook_spec::{HookViewAction, PreviewCardNode, PreviewTone, TextSizeTier};

    let dir = TempHookDir::new("params-views");
    let hub = open_test_hub(&dir.path);
    ensure_hook_types(&dir.path).expect("sync zniff-hook.d.ts");

    dir.write_hook(
        "full.tsx",
        r#"
import { Section, KeyValue, Badge, Text, Divider, Callout, Table, Collapsible, Tabs, Tab, Row, Col, View } from "zniff/hook";

export function onRequest(ctx) {
  if (ctx.path.startsWith("/v1/mock")) {
    return {
      respondWith: {
        status: 201,
        setHeaders: { "X-Mock": "true", "X-Num": 7 },
        form: { status: "created", q: ctx.query.find(([k]) => k === "k")?.[1] ?? "" },
        view: <Badge tone="accent" copyValue="mock-copy">RES-MOCK</Badge>,
      },
      view: <Callout tone="warn" title="Short-Circuit">Intercepted</Callout>,
      comment: "mock-hit",
    };
  }
  const incoming = ctx.json();
  return {
    query: { page: 2, remove_me: null },
    setHeaders: { authorization: "Bearer next", "x-drop": null },
    json: { wrapped: incoming, rawLen: ctx.text().length, tag: ctx.form().length },
    comment: "req-patched",
    view: (
      <Section title="Req Stage" copyValue="sig-1" selectFlowId={42} setFilter="host:api.vicanso.dev">
        <KeyValue label="FirstQuery" value={ctx.query.map(([k, v]) => `${k}=${v}`).join(",")} />
        <Badge tone="ok">REQ-BADGE</Badge>
      </Section>
    ),
  };
}

export function onResponse(ctx) {
  const parsed = ctx.json();
  return {
    status: 202,
    setHeaders: { "X-Signed": "yes", "x-drop": null },
    json: { ok: parsed?.ok ?? false, qCount: ctx.query.length },
    view: <Divider />,
  };
}

export function onPreview(ctx) {
  const reqObj = ctx.req.json();
  return {
    view: (
      <Row gap={8} wrap justify="between">
        <Text tone="accent" bold size="base">Overview</Text>
        <Badge tone="ok" selectFlowId={ctx.id}>SELF</Badge>
      </Row>
    ),
    req: {
      json: { decReq: reqObj?.hello ?? "none" },
      note: "ReqDec",
      view: (
        <Collapsible title="ReqTree" defaultOpen={false}>
          <KeyValue label="Q" value={String(ctx.query.length)} />
        </Collapsible>
      ),
    },
    res: {
      form: [["resKey", ctx.res.text().slice(0, 8)]],
      note: "ResDec",
      view: (
        <Tabs>
          <Tab label="Metrics">
            <Table columns={["ColA", "ColB"]} rows={[["k", "v"]]} />
          </Tab>
          <Tab label="Raw">
            <Col gap={4}><View bordered padding={4}><Text mono size="xs">cell</Text></View></Col>
          </Tab>
        </Tabs>
      ),
    },
  };
}
"#,
    );

    let resolved = resolve_script_hook(&dir.path, &[], "@full", &BTreeMap::new(), 500).unwrap();

    // 1. onRequest incremental mutation + ctx.query/text()/json()/form() + view
    let req_out = eval_request_hook_async(
        hub.hook_session_host(),
        resolved.clone(),
        HookRequestInputOwned {
            flow_id: 1,
            method: "POST".into(),
            scheme: "https".into(),
            host: "api.vicanso.dev".into(),
            url: "https://api.vicanso.dev/v1/items?remove_me=1&page=1".into(),
            path: "/v1/items?remove_me=1&page=1".into(),
            headers: vec![
                ("Authorization".into(), "Bearer old".into()),
                ("X-Drop".into(), "gone".into()),
                ("Content-Type".into(), "text/plain".into()),
            ],
            body: br#"{"ok":true}"#.to_vec(),
        },
    )
    .await
    .expect("onRequest succeeds");

    match req_out {
        ScriptRequestAction::Mutate {
            path,
            headers,
            body,
            comment,
            view,
            ..
        } => {
            assert_eq!(path.as_deref(), Some("/v1/items?page=2"));
            let hdrs = headers.unwrap();
            assert!(!hdrs.iter().any(|(k, _)| k.eq_ignore_ascii_case("x-drop")));
            assert!(
                hdrs.iter()
                    .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && v == "Bearer next")
            );
            assert!(
                hdrs.iter()
                    .any(|(k, v)| k.eq_ignore_ascii_case("content-type")
                        && v == "application/json; charset=utf-8")
            );
            let parsed: serde_json::Value = serde_json::from_slice(&body.unwrap()).unwrap();
            assert_eq!(parsed["wrapped"]["ok"], true);
            assert_eq!(parsed["rawLen"], 11);
            assert_eq!(comment.as_deref(), Some("req-patched"));
            assert!(matches!(
                &view[0],
                PreviewCardNode::Section {
                    title,
                    action: HookViewAction {
                        copy_value: Some(c),
                        select_flow_id: Some(42),
                        set_filter: Some(f),
                    },
                    ..
                } if title == "Req Stage" && c == "sig-1" && f == "host:api.vicanso.dev"
            ));
        }
        other => panic!("expected Mutate, got {other:?}"),
    }

    // 2. onRequest respondWith with form + setHeaders + outer/inner views
    let mock_out = eval_request_hook_async(
        hub.hook_session_host(),
        resolved.clone(),
        HookRequestInputOwned {
            flow_id: 2,
            method: "GET".into(),
            scheme: "https".into(),
            host: "api.vicanso.dev".into(),
            url: "https://api.vicanso.dev/v1/mock?k=alpha".into(),
            path: "/v1/mock?k=alpha".into(),
            headers: vec![],
            body: vec![],
        },
    )
    .await
    .expect("respondWith succeeds");
    match mock_out {
        ScriptRequestAction::RespondWith {
            status,
            headers,
            body,
            view,
            res_view,
            ..
        } => {
            assert_eq!(status, 201);
            assert_eq!(body, b"status=created&q=alpha");
            assert!(headers.iter().any(|(k, v)| k == "X-Num" && v == "7"));
            assert!(headers.iter().any(|(k, v)| {
                k.eq_ignore_ascii_case("content-type")
                    && v == "application/x-www-form-urlencoded; charset=utf-8"
            }));
            assert!(matches!(&view[0], PreviewCardNode::Callout { .. }));
            assert!(matches!(
                &res_view[0],
                PreviewCardNode::Badge {
                    tone: PreviewTone::Accent,
                    text,
                    ..
                } if text == "RES-MOCK"
            ));
        }
        other => panic!("expected RespondWith, got {other:?}"),
    }

    // 3. onResponse incremental mutation + view
    let res_out = eval_response_hook_async(
        hub.hook_session_host(),
        resolved.clone(),
        HookResponseInputOwned {
            flow_id: 3,
            method: "POST".into(),
            host: "api.vicanso.dev".into(),
            path: "/v1/items?a=1&b=2".into(),
            url: "https://api.vicanso.dev/v1/items?a=1&b=2".into(),
            status: 200,
            headers: vec![
                ("Content-Type".into(), "text/plain".into()),
                ("X-Drop".into(), "stale".into()),
            ],
            body: br#"{"ok":true}"#.to_vec(),
        },
    )
    .await
    .expect("onResponse succeeds");
    assert_eq!(res_out.status, Some(202));
    assert!(matches!(
        res_out.view.as_slice(),
        [PreviewCardNode::Divider]
    ));
    let res_hdrs = res_out.headers.unwrap();
    assert!(
        !res_hdrs
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("x-drop"))
    );
    assert!(res_hdrs.iter().any(|(k, v)| k == "X-Signed" && v == "yes"));
    assert!(
        res_hdrs
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("content-type")
                && v == "application/json; charset=utf-8")
    );

    // 4. onPreview both sides (req.view, res.view, overview view)
    let prev_req = eval_preview_hook_async(
        hub.hook_session_host(),
        resolved.clone(),
        HookPreviewInputOwned {
            flow_id: 4,
            is_response: false,
            method: "POST".into(),
            host: "api.vicanso.dev".into(),
            path: "/v1/preview?x=9".into(),
            url: "https://api.vicanso.dev/v1/preview?x=9".into(),
            status: Some(200),
            req_headers: vec![("Content-Type".into(), "application/json".into())],
            req_body: br#"{"hello":"zniff"}"#.to_vec(),
            res_headers: vec![],
            res_body: b"abcdefghi".to_vec(),
        },
    )
    .await
    .expect("preview req succeeds");
    assert_eq!(prev_req.side.note.as_deref(), Some("ReqDec"));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(prev_req.side.body.as_deref().unwrap())
            .unwrap()["decReq"],
        "zniff"
    );
    assert!(matches!(
        &prev_req.cards[0],
        PreviewCardNode::Row { children, .. }
            if matches!(&children[0], PreviewCardNode::Text { size: Some(TextSizeTier::Base), bold: true, .. })
    ));
    assert!(matches!(
        &prev_req.side.view[0],
        PreviewCardNode::Collapsible {
            title,
            default_open: false,
            ..
        } if title == "ReqTree"
    ));

    let prev_res = eval_preview_hook_async(
        hub.hook_session_host(),
        resolved,
        HookPreviewInputOwned {
            flow_id: 4,
            is_response: true,
            method: "POST".into(),
            host: "api.vicanso.dev".into(),
            path: "/v1/preview?x=9".into(),
            url: "https://api.vicanso.dev/v1/preview?x=9".into(),
            status: Some(200),
            req_headers: vec![],
            req_body: vec![],
            res_headers: vec![],
            res_body: b"abcdefghijkl".to_vec(),
        },
    )
    .await
    .expect("preview res succeeds");
    assert_eq!(
        prev_res.side.body.as_deref(),
        Some(b"resKey=abcdefgh".as_slice())
    );
    assert!(matches!(
        &prev_res.side.view[0],
        PreviewCardNode::Tabs { items }
            if items.len() == 2 && items[0].label == "Metrics" && items[1].label == "Raw"
    ));
}
