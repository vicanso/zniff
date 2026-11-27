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
        PreviewCardNode::Section { title, children } => {
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
