//! End-to-end QuickJS TypeScript SDK integration tests executed via `crate::hook_runtime`.

use std::{collections::BTreeMap, fs, io::Write, path::PathBuf, sync::Arc, time::UNIX_EPOCH};

use crate::ca::Ca;
use crate::hook_action::{
    HookPreviewInputOwned, HookRequestInputOwned, HookResponseInputOwned, ScriptRequestAction,
};
use crate::hook_codec_crypto::{aes_gcm_encrypt, hex_encode_bytes};
use crate::hook_runtime::{
    ensure_hook_types, eval_preview_hook_async, eval_request_hook_async, eval_response_hook_async,
    eval_run_hook_async, hooks_dir, resolve_script_hook,
};
use crate::hook_spec::{HookStage, PreviewCardNode, PreviewTone, ScriptHookConfig};
use crate::proxy::Hub;

struct TempSdkDir(PathBuf);

impl TempSdkDir {
    fn new(tag: &str) -> Self {
        let nanos = crate::clock::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("zniff-sdk-{tag}-{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("mkdir");
        Self(path)
    }

    fn write(&self, file_name: &str, source: &str) {
        let dir = hooks_dir(&self.0);
        fs::create_dir_all(&dir).expect("mkdir hooks");
        fs::write(dir.join(file_name), source).expect("write hook");
    }

    fn hub(&self) -> Arc<Hub> {
        Arc::new(Hub::new(Arc::new(Ca::open(&self.0).expect("open ca"))).expect("hub"))
    }
}

impl Drop for TempSdkDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn hdr<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn sample_proto_wire() -> Vec<u8> {
    let varint = |mut v: u64, out: &mut Vec<u8>| loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        out.push(if v == 0 { b } else { b | 0x80 });
        if v == 0 {
            break;
        }
    };
    let ld = |f: u32, data: &[u8], out: &mut Vec<u8>| {
        varint((u64::from(f) << 3) | 2, out);
        varint(data.len() as u64, out);
        out.extend_from_slice(data);
    };
    let mut nested = Vec::new();
    ld(1, b"auth-ok", &mut nested);
    let mut proto = Vec::new();
    ld(1, b"rpc.vicanso.dev", &mut proto);
    varint(2 << 3, &mut proto);
    varint(2026, &mut proto);
    ld(3, &nested, &mut proto);
    ld(4, b"alpha", &mut proto);
    ld(4, b"beta", &mut proto);
    proto
}

async fn run_req(
    dir: &TempSdkDir,
    hub: &Hub,
    script: &str,
    vars: BTreeMap<String, String>,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) -> Vec<(String, String)> {
    let resolved = resolve_script_hook(&dir.0, &[], script, &vars, 500).unwrap();
    let input = HookRequestInputOwned {
        flow_id: 1,
        method: "POST".into(),
        scheme: "https".into(),
        host: "api.vicanso.dev".into(),
        url: "https://api.vicanso.dev/v1/api".into(),
        path: "/v1/api".into(),
        headers,
        body,
    };
    match eval_request_hook_async(hub.hook_session_host(), resolved, input)
        .await
        .expect("onRequest succeeds")
    {
        ScriptRequestAction::Mutate {
            headers: Some(h), ..
        } => h,
        other => panic!("expected Mutate headers, got {other:?}"),
    }
}

#[tokio::test]
async fn compression_roundtrips_through_hook_runtime() {
    let dir = TempSdkDir::new("compress");
    let hub = dir.hub();
    let plain = br#"{"host":"api.vicanso.dev","codec":"all-formats"}"#;
    let mut br_body = Vec::new();
    brotli::CompressorWriter::new(&mut br_body, 4096, 5, 22)
        .write_all(plain)
        .unwrap();
    dir.write(
        "compress.tsx",
        r#"import type { ResponseHookContext, ResponseHookResult } from "zniff/hook";
export function onResponse(ctx: ResponseHookContext): ResponseHookResult {
  const s = zniff.codec.utf8Decode(zniff.codec.brotliDecompress(ctx.body));
  const gz = zniff.codec.utf8Decode(zniff.codec.gunzip(zniff.codec.gzip(s)));
  const fl = zniff.codec.utf8Decode(zniff.codec.inflate(zniff.codec.deflate(s)));
  const zl = zniff.codec.utf8Decode(zniff.codec.zlibDecompress(zniff.codec.zlibCompress(s)));
  const z1 = zniff.codec.utf8Decode(zniff.codec.zstdDecompress(zniff.codec.zstdCompress(s)));
  const z6 = zniff.codec.utf8Decode(zniff.codec.zstdDecompress(zniff.codec.zstdCompress(s, 6)));
  const ok = gz === s && fl === s && zl === s && z1 === s && z6 === s;
  return { headers: [...ctx.headers, ["X-All-Codec", ok ? "ok" : "fail"]], body: s };
}"#,
    );
    let resolved = resolve_script_hook(&dir.0, &[], "@compress", &BTreeMap::new(), 500).unwrap();
    let res = eval_response_hook_async(
        hub.hook_session_host(),
        resolved,
        HookResponseInputOwned {
            flow_id: 1,
            method: "GET".into(),
            host: "api.vicanso.dev".into(),
            path: "/v1/compressed".into(),
            url: "https://api.vicanso.dev/v1/compressed".into(),
            status: 200,
            headers: vec![("Content-Encoding".into(), "br".into())],
            body: br_body,
        },
    )
    .await
    .expect("onResponse compression succeeds");
    assert_eq!(hdr(&res.headers.unwrap(), "x-all-codec"), Some("ok"));
    assert_eq!(res.body.as_deref(), Some(plain.as_slice()));
}

#[tokio::test]
async fn url_form_jwt_crc32_inspection_through_hook_runtime() {
    let dir = TempSdkDir::new("inspect-codec");
    let hub = dir.hub();
    dir.write(
        "inspect.ts",
        r#"import type { RequestHookContext, RequestHookResult } from "zniff/hook";
export function onRequest(ctx: RequestHookContext): RequestHookResult {
  const auth = ctx.headers.find(([k]) => k.toLowerCase() === "authorization")?.[1] ?? "";
  const jwt = zniff.codec.jwtDecode(auth);
  const pairs = zniff.codec.formUrlDecode("?" + zniff.codec.utf8Decode(ctx.body));
  const formArr = zniff.codec.formUrlEncode(pairs);
  const formObj = zniff.codec.formUrlEncode({ user: "alice@vicanso.dev", note: "hello world!" });
  const b64u = zniff.codec.base64UrlEncode(new Uint8Array([0xfb, 0xff, 0xbf]));
  const ok = jwt !== null && zniff.codec.jwtDecode("broken.jwt") === null && formArr === formObj
    && zniff.codec.urlEncode("https://api.vicanso.dev/a b") === "https%3A%2F%2Fapi.vicanso.dev%2Fa%20b"
    && zniff.codec.urlDecode("a+b%20c") === "a+b c"
    && b64u === "-_-_" && zniff.codec.hexEncode(zniff.codec.base64UrlDecode(b64u)) === "fbffbf"
    && zniff.codec.crc32("123456789") === 0xCBF43926;
  return { headers: [...ctx.headers, ["X-Jwt-Sub", String(jwt?.payload?.sub ?? "")],
    ["X-Jwt-Sig", jwt?.signatureHex ?? ""], ["X-Form-Enc", formArr], ["X-Codec-Ok", ok ? "yes" : "no"]] };
}"#,
    );
    let b64u = |raw: &[u8]| {
        crate::session::base64_encode(raw)
            .trim_end_matches('=')
            .replace('+', "-")
            .replace('/', "_")
    };
    let jwt = format!(
        "Bearer {}.{}.{}",
        b64u(br#"{"alg":"HS256","typ":"JWT"}"#),
        b64u(br#"{"iss":"auth.vicanso.dev","sub":"u-vicanso"}"#),
        b64u(&[0xca, 0xfe, 0xba, 0xbe])
    );
    let hdrs = run_req(
        &dir,
        &hub,
        "inspect.ts",
        BTreeMap::new(),
        vec![("Authorization".into(), jwt)],
        b"user=alice%40vicanso.dev&note=hello+world%21".to_vec(),
    )
    .await;
    for (k, v) in [
        ("x-jwt-sub", "u-vicanso"),
        ("x-jwt-sig", "cafebabe"),
        ("x-codec-ok", "yes"),
        ("x-form-enc", "user=alice%40vicanso.dev&note=hello+world%21"),
    ] {
        assert_eq!(hdr(&hdrs, k), Some(v), "{k}");
    }
}

#[tokio::test]
async fn api_request_resigning_through_hook_runtime() {
    let dir = TempSdkDir::new("resign");
    let hub = dir.hub();
    dir.write(
        "resign.ts",
        r#"import type { RequestHookContext, RequestHookResult } from "zniff/hook";
export function onRequest(ctx: RequestHookContext): RequestHookResult {
  const s = ctx.vars.secret, h256 = zniff.crypto.hmacSha256Hex(s, ctx.body);
  const tsOk = zniff.crypto.timingSafeEqual(h256, h256) && !zniff.crypto.timingSafeEqual(h256, "0000");
  return { headers: [...ctx.headers,
    ["X-Md5", zniff.crypto.md5Hex(ctx.body)], ["X-Sha1", zniff.crypto.sha1Hex(ctx.body)],
    ["X-Sha256", zniff.crypto.sha256Hex(ctx.body)], ["X-Sha384", zniff.crypto.sha384Hex(ctx.body)],
    ["X-Sha512", zniff.crypto.sha512Hex(ctx.body)], ["X-Hmac-Md5", zniff.crypto.hmacMd5Hex(s, ctx.body)],
    ["X-Hmac-Sha1", zniff.crypto.hmacSha1Hex(s, ctx.body)], ["X-Hmac-Sha256", h256],
    ["X-Hmac-Sha512", zniff.crypto.hmacSha512Hex(s, ctx.body)], ["X-Timing-Safe", tsOk ? "ok" : "bad"]] };
}"#,
    );
    let vars = BTreeMap::from([("secret".into(), "Jefe".into())]);
    let hdrs = run_req(&dir, &hub, "resign.ts", vars, vec![], b"abc".to_vec()).await;
    assert_eq!(
        hdr(&hdrs, "x-md5"),
        Some("900150983cd24fb0d6963f7d28e17f72")
    );
    assert_eq!(
        hdr(&hdrs, "x-sha1"),
        Some("a9993e364706816aba3e25717850c26c9cd0d89d")
    );
    assert_eq!(
        hdr(&hdrs, "x-sha256"),
        Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    );
    for (k, len) in [
        ("x-sha384", 96),
        ("x-sha512", 128),
        ("x-hmac-md5", 32),
        ("x-hmac-sha1", 40),
        ("x-hmac-sha256", 64),
        ("x-hmac-sha512", 128),
    ] {
        assert_eq!(hdr(&hdrs, k).map(str::len), Some(len), "{k}");
    }
    assert_eq!(hdr(&hdrs, "x-timing-safe"), Some("ok"));
}

#[tokio::test]
async fn payload_crypto_and_kdf_through_hook_runtime() {
    let dir = TempSdkDir::new("kdf-cbc");
    let hub = dir.hub();
    dir.write(
        "kdf.ts",
        r#"import type { RunHookContext, RunHookResult } from "zniff/hook";
export function onRun(ctx: RunHookContext): RunHookResult {
  const pt = "host=api.vicanso.dev&action=transfer";
  const pbKey = zniff.crypto.pbkdf2Sha256("vicanso-pass", "salt-1", 1000, 32);
  const hk = zniff.crypto.hkdfSha256(pbKey, "", "cbc-key-iv", 32), iv = hk.slice(16, 32);
  const p128S = zniff.crypto.aes128CbcDecrypt("0123456789abcdef", iv, zniff.crypto.aes128CbcEncrypt("0123456789abcdef", iv, pt));
  const p128B = zniff.crypto.aes128CbcDecrypt(hk.slice(0, 16), iv, zniff.crypto.aes128CbcEncrypt(hk.slice(0, 16), iv, pt));
  const p256S = zniff.crypto.aes256CbcDecrypt("0123456789abcdef0123456789abcdef", iv, zniff.crypto.aes256CbcEncrypt("0123456789abcdef0123456789abcdef", iv, pt));
  const p256B = zniff.crypto.aes256CbcDecrypt(pbKey, iv, zniff.crypto.aes256CbcEncrypt(pbKey, iv, pt));
  const n12 = zniff.crypto.randomBytes(12);
  const chPt = zniff.crypto.chacha20Poly1305Decrypt(pbKey, n12, zniff.crypto.chacha20Poly1305Encrypt(pbKey, n12, pt, "api.vicanso.dev"), "api.vicanso.dev");
  const u1 = zniff.crypto.uuidV4(), u2 = zniff.crypto.uuidV4();
  const re = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
  const ok = [p128S, p128B, p256S, p256B, chPt].every(b => zniff.codec.utf8Decode(b) === pt)
    && n12.length === 12 && re.test(u1) && re.test(u2) && u1 !== u2;
  return { summary: `${ctx.stage}:${ok ? "ok" : "fail"}:${u1}` };
}"#,
    );
    let report = eval_run_hook_async(&dir.0, &[], hub.hook_session_host(), "@kdf.ts", "", None)
        .await
        .expect("onRun kdf & encryption succeeds");
    assert!(report.summary.starts_with("run:ok:"), "{}", report.summary);
}

#[tokio::test]
async fn encrypted_compressed_protobuf_preview_and_frozen_sdk() {
    let dir = TempSdkDir::new("proto-preview");
    let hub = dir.hub();
    ensure_hook_types(&dir.0).expect("write zniff-hook.d.ts");
    let compressed = zstd::stream::encode_all(sample_proto_wire().as_slice(), 3).unwrap();
    let (key, nonce) = ([0x5a_u8; 32], [0x9c_u8; 12]);
    let enc = aes_gcm_encrypt(
        &ring::aead::AES_256_GCM,
        &key,
        &nonce,
        &compressed,
        b"rpc.vicanso.dev",
    )
    .unwrap();
    dir.write(
        "proto-preview.tsx",
        r#"import type { PreviewHookContext, PreviewHookResult } from "zniff/hook";
import { Section, KeyValue, Badge, Code } from "zniff/hook";
export function onPreview(ctx: PreviewHookContext): PreviewHookResult {
  const frozen = Object.isFrozen(zniff) && Object.isFrozen(zniff.flows)
    && Object.isFrozen(zniff.codec) && Object.isFrozen(zniff.crypto);
  let writeBlocked = false;
  const mutableCodec: any = zniff.codec;
  try { mutableCodec.gzip = null; } catch { writeBlocked = true; }
  const raw = zniff.crypto.aes256GcmDecrypt(
    zniff.codec.hexDecode(ctx.vars.key_hex), zniff.codec.hexDecode(ctx.vars.nonce_hex), ctx.res.body, ctx.host);
  const wire = zniff.codec.protobufDecodeWire(zniff.codec.zstdDecompress(raw));
  return {
    res: { body: wire, contentType: "application/json", note: "Protobuf wire decoded" },
    cards: (
      <Section title="Protobuf 载荷">
        <Badge tone={frozen && writeBlocked ? "ok" : "danger"}>{frozen && writeBlocked ? "SDK FROZEN" : "MUTABLE"}</Badge>
        <KeyValue label="Service" value={String(wire["1"])} />
        <KeyValue label="Version" value={String(wire["2"])} />
        <Code language="json">{JSON.stringify(wire)}</Code>
      </Section>
    ),
  };
}"#,
    );
    let cfg = vec![ScriptHookConfig {
        id: "proto-preview".into(),
        name: "Proto Preview".into(),
        script: "proto-preview.tsx".into(),
        enabled: true,
        match_pattern: "rpc.vicanso.dev".into(),
        stages: vec![HookStage::Preview],
        timeout_ms: 500,
        vars: BTreeMap::from([
            ("key_hex".into(), hex_encode_bytes(&key)),
            ("nonce_hex".into(), hex_encode_bytes(&nonce)),
        ]),
    }];
    let resolved = resolve_script_hook(&dir.0, &cfg, "proto-preview", &BTreeMap::new(), 0).unwrap();
    let preview = eval_preview_hook_async(
        hub.hook_session_host(),
        resolved,
        HookPreviewInputOwned {
            flow_id: 77,
            is_response: true,
            method: "POST".into(),
            host: "rpc.vicanso.dev".into(),
            path: "/rpc.User/Get".into(),
            url: "https://rpc.vicanso.dev/rpc.User/Get".into(),
            status: Some(200),
            req_headers: vec![],
            req_body: vec![],
            res_body: enc,
            res_headers: vec![("Content-Type".into(), "application/x-protobuf".into())],
        },
    )
    .await
    .expect("onPreview protobuf wire succeeds");
    let body: serde_json::Value = serde_json::from_slice(&preview.side.body.unwrap()).unwrap();
    assert_eq!(body["1"], "rpc.vicanso.dev");
    assert_eq!(body["2"], 2026);
    assert_eq!(body["3"]["1"], "auth-ok");
    assert_eq!(body["4"], serde_json::json!(["alpha", "beta"]));
    let PreviewCardNode::Section { title, children } = &preview.cards[0] else {
        panic!("expected Section")
    };
    assert_eq!(title, "Protobuf 载荷");
    assert!(
        matches!(&children[0], PreviewCardNode::Badge { tone: PreviewTone::Ok, text } if text == "SDK FROZEN")
    );
    assert!(
        matches!(&children[1], PreviewCardNode::KeyValue { label, value } if label == "Service" && value == "rpc.vicanso.dev")
    );
}
