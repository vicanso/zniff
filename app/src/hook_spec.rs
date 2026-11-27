//! Cross-platform NSH/TSX packet hook specification, config schema,
//! declarative preview card IR, TypeScript definitions (`zniff-hook.d.ts`),
//! and lightweight TSX transpilation wrapper.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::layout_tsx::{MAX_LAYOUT_SCRIPT_BYTES, TsxScriptKind, compile_tsx_program};

/// Maximum number of `[[script_hooks]]` entries persisted in `config.toml`.
pub const SCRIPT_HOOK_CAP: usize = 32;

/// Default wall-clock budget for live `onRequest` / `onResponse` script hooks.
pub const DEFAULT_HOOK_TIMEOUT_MS: u64 = 80;

/// Minimum allowed hook timeout in milliseconds.
pub const MIN_HOOK_TIMEOUT_MS: u64 = 10;

/// Maximum allowed hook timeout in milliseconds (offline/preview/run).
pub const MAX_HOOK_TIMEOUT_MS: u64 = 10_000;

/// Maximum wall-clock budget allowed on live proxy `onRequest` / `onResponse` stages.
pub const MAX_LIVE_HOOK_TIMEOUT_MS: u64 = 500;

#[allow(unused_imports)]
pub use crate::hook_view_spec::{
    HookViewAction, HookViewNode, HookViewTabItem, MAX_PREVIEW_CARD_DEPTH, MAX_PREVIEW_CARD_NODES,
    MAX_PREVIEW_CARD_TEXT_BYTES, PreviewCardNode, PreviewTone, TextSizeTier,
    validate_preview_cards,
};

/// Maximum `vars` entries allowed per configured script hook.
pub const MAX_HOOK_VARS: usize = 32;
const MAX_VAR_KEY_BYTES: usize = 64;
const MAX_VAR_VAL_BYTES: usize = 4096;

/// Execution stages supported by a `zniff` TSX/TS hook script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookStage {
    Request,
    Response,
    Preview,
    Run,
}

impl HookStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Response => "response",
            Self::Preview => "preview",
            Self::Run => "run",
        }
    }
}

const fn default_true() -> bool {
    true
}

fn is_true(val: &bool) -> bool {
    *val
}

const fn default_hook_timeout_ms() -> u64 {
    DEFAULT_HOOK_TIMEOUT_MS
}

fn is_default_hook_timeout_ms(val: &u64) -> bool {
    *val == DEFAULT_HOOK_TIMEOUT_MS
}

/// Persisted configuration entry for one `<data_dir>/hooks/*.tsx` script hook (`[[script_hooks]]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptHookConfig {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub script: String,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(default, rename = "match", skip_serializing_if = "String::is_empty")]
    pub match_pattern: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<HookStage>,
    #[serde(
        default = "default_hook_timeout_ms",
        skip_serializing_if = "is_default_hook_timeout_ms"
    )]
    pub timeout_ms: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vars: BTreeMap<String, String>,
}

/// Summary metadata for a discovered or configured script hook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptHookSummary {
    pub id: String,
    pub name: String,
    pub file_name: String,
    pub enabled: bool,
    pub configured: bool,
    pub has_file: bool,
    pub match_pattern: String,
    pub stages: Vec<HookStage>,
    pub timeout_ms: u64,
    pub circuit_tripped: bool,
}

/// Validates and normalizes a hook ID (`[a-zA-Z0-9_-]`, 1–48 chars).
pub fn sanitize_hook_id(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let trimmed = trimmed
        .strip_suffix(".tsx")
        .or_else(|| trimmed.strip_suffix(".ts"))
        .unwrap_or(trimmed);
    if trimmed.is_empty() || trimmed.len() > 48 {
        return Err("hook ID must be 1–48 characters".to_string());
    }
    if trimmed.contains("..")
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("hook ID may only use letters, digits, `-`, and `_`".to_string());
    }
    let lower = trimmed.to_ascii_lowercase();
    if matches!(lower.as_str(), "default" | "zniff-hook" | "zniff-layout") {
        return Err(format!("`{trimmed}` is a reserved hook ID"));
    }
    Ok(trimmed.to_string())
}

/// Normalizes a `@<id_or_file>` target reference from rules DSL into the canonical base ID or file stem.
pub fn sanitize_hook_reference(raw: &str) -> Option<String> {
    let stripped = raw.trim().strip_prefix('@')?;
    sanitize_hook_script_file(stripped).ok()
}

/// Validates and normalizes a script filename relative to `<data_dir>/hooks/`.
pub fn sanitize_hook_script_file(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let trimmed = trimmed.strip_prefix('@').unwrap_or(trimmed).trim();
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err("hook script must be a plain file name inside hooks/".to_string());
    }
    let ext = if trimmed.ends_with(".ts") && !trimmed.ends_with(".tsx") {
        "ts"
    } else {
        "tsx"
    };
    let stem = sanitize_hook_id(trimmed)?;
    Ok(format!("{stem}.{ext}"))
}

/// Clamps and sanitizes a `[[script_hooks]]` list (caps count, deduplicates IDs,
/// cleans stages, timeout, and bounded `vars`).
pub fn sanitize_script_hooks(items: Vec<ScriptHookConfig>) -> Vec<ScriptHookConfig> {
    let mut out: Vec<ScriptHookConfig> = Vec::new();
    for item in items {
        let Ok(id) = sanitize_hook_id(&item.id) else {
            continue;
        };
        if out.iter().any(|seen| seen.id.eq_ignore_ascii_case(&id)) {
            continue;
        }
        let script = if item.script.trim().is_empty() {
            format!("{id}.tsx")
        } else {
            let Ok(file) = sanitize_hook_script_file(&item.script) else {
                continue;
            };
            file
        };
        let mut stages = Vec::new();
        for stage in item.stages {
            if !stages.contains(&stage) {
                stages.push(stage);
            }
        }
        let mut vars = BTreeMap::new();
        for (k, v) in item.vars {
            let key = k.trim();
            if key.is_empty() || key.len() > MAX_VAR_KEY_BYTES || v.len() > MAX_VAR_VAL_BYTES {
                continue;
            }
            vars.insert(key.to_string(), v);
            if vars.len() >= MAX_HOOK_VARS {
                break;
            }
        }
        out.push(ScriptHookConfig {
            id,
            name: item.name.trim().to_string(),
            script,
            enabled: item.enabled,
            match_pattern: item.match_pattern.trim().to_string(),
            stages,
            timeout_ms: item
                .timeout_ms
                .clamp(MIN_HOOK_TIMEOUT_MS, MAX_HOOK_TIMEOUT_MS),
            vars,
        });
        if out.len() >= SCRIPT_HOOK_CAP {
            break;
        }
    }
    out
}

/// Transpiles a `<data_dir>/hooks/*.tsx` script into plain JavaScript runnable on QuickJS.
pub fn transpile_hook_tsx(source: &str) -> Result<String, String> {
    if source.len() > MAX_LAYOUT_SCRIPT_BYTES {
        return Err(format!(
            "hook script exceeds size limit ({MAX_LAYOUT_SCRIPT_BYTES} bytes)"
        ));
    }
    if source.trim().is_empty() {
        return Err("hook script is empty".to_string());
    }
    compile_tsx_program(source, TsxScriptKind::Hook)
}

/// Ambient TypeScript definitions written into `<data_dir>/hooks/zniff-hook.d.ts`.
pub const ZNIFF_HOOK_DTS: &str = r#"// Auto-generated by zniff. Do not edit manually.
export type HookStage = "request" | "response" | "preview" | "run";
export type HeaderPair = [string, string];
export type PreviewTone = "neutral" | "ok" | "warn" | "danger" | "accent";
export type BodyInput = Uint8Array | ArrayBuffer | string;

export interface FlowInfo {
  readonly id: number;
  readonly protocol: string;
  readonly method: string;
  readonly url: string;
  readonly host: string;
  readonly path: string;
  readonly status: number | null;
  readonly error: string | null;
  readonly clientIp: string;
  readonly elapsedMs: number | null;
  readonly reqBytes: number;
  readonly resBytes: number;
  readonly comment: string;
  readonly pinned: boolean;
}

export interface HookActionProps {
  copyValue?: string;
  selectFlowId?: number;
  setFilter?: string;
}

export type TextSizeTier = "xs" | "sm" | "base";

export interface HookViewNode {
  readonly type:
    | "section"
    | "key_value"
    | "badge"
    | "code"
    | "view"
    | "row"
    | "col"
    | "text"
    | "divider"
    | "callout"
    | "table"
    | "collapsible"
    | "tabs";
}
export type PreviewCardNode = HookViewNode;

export type HookViewChild =
  | HookViewNode
  | HookViewNode[]
  | false
  | null
  | undefined;
export type PreviewCardChild = HookViewChild;

export function Section(props: HookActionProps & { title?: string; children?: HookViewChild }): HookViewNode;
export function KeyValue(props: HookActionProps & { label?: string; value?: string; children?: string }): HookViewNode;
export function Badge(props: HookActionProps & { tone?: PreviewTone; text?: string; children?: string }): HookViewNode;
export function Code(props: HookActionProps & { language?: string; code?: string; children?: string }): HookViewNode;
export function View(props: HookActionProps & { gap?: number; padding?: number; bordered?: boolean; children?: HookViewChild }): HookViewNode;
export function Row(props: HookActionProps & { gap?: number; wrap?: boolean; align?: "start" | "center" | "end"; justify?: "start" | "center" | "end" | "between"; children?: HookViewChild }): HookViewNode;
export function Col(props: HookActionProps & { gap?: number; children?: HookViewChild }): HookViewNode;
export function Text(props: HookActionProps & { tone?: PreviewTone; mono?: boolean; bold?: boolean; size?: TextSizeTier; text?: string; children?: string }): HookViewNode;
export function Divider(): HookViewNode;
export function Callout(props: HookActionProps & { tone?: PreviewTone; title?: string; text?: string; children?: HookViewChild | string }): HookViewNode;
export function Table(props: { columns: string[]; rows: (string | number | boolean | null | undefined)[][] }): HookViewNode;
export function Collapsible(props: { title: string; defaultOpen?: boolean; children?: HookViewChild }): HookViewNode;
export function Tabs(props: { children?: HookViewChild }): HookViewNode;
export function Tab(props: { label: string; children?: HookViewChild }): HookViewNode;

export interface RequestHookContext {
  readonly id: number;
  readonly stage: "request";
  readonly method: string;
  readonly url: string;
  readonly scheme: string;
  readonly host: string;
  readonly path: string;
  readonly query: HeaderPair[];
  readonly headers: HeaderPair[];
  readonly body: Uint8Array;
  readonly vars: Readonly<Record<string, string>>;
  text(): string;
  json<T = unknown>(): T | null;
  form(): HeaderPair[];
}

export interface RequestShortCircuitResponse {
  status: number;
  headers?: HeaderPair[] | Record<string, string>;
  setHeaders?: Record<string, string | number | boolean | null | undefined>;
  body?: BodyInput | Record<string, unknown> | unknown[];
  json?: unknown;
  form?: HeaderPair[] | Record<string, string>;
  view?: HookViewChild;
  cards?: HookViewChild;
}

export interface RequestHookResult {
  method?: string;
  path?: string;
  query?: HeaderPair[] | Record<string, string | number | boolean | null | undefined>;
  headers?: HeaderPair[] | Record<string, string>;
  setHeaders?: Record<string, string | number | boolean | null | undefined>;
  body?: BodyInput;
  json?: unknown;
  form?: HeaderPair[] | Record<string, string>;
  comment?: string;
  view?: HookViewChild;
  cards?: HookViewChild;
  respondWith?: RequestShortCircuitResponse;
}

export interface ResponseHookContext {
  readonly id: number;
  readonly stage: "response";
  readonly method: string;
  readonly url: string;
  readonly host: string;
  readonly path: string;
  readonly query: HeaderPair[];
  readonly status: number;
  readonly headers: HeaderPair[];
  readonly body: Uint8Array;
  readonly vars: Readonly<Record<string, string>>;
  text(): string;
  json<T = unknown>(): T | null;
  form(): HeaderPair[];
}

export interface ResponseHookResult {
  status?: number;
  headers?: HeaderPair[] | Record<string, string>;
  setHeaders?: Record<string, string | number | boolean | null | undefined>;
  body?: BodyInput;
  json?: unknown;
  form?: HeaderPair[] | Record<string, string>;
  comment?: string;
  view?: HookViewChild;
  cards?: HookViewChild;
}

export interface PreviewHookSide {
  readonly headers: HeaderPair[];
  readonly body: Uint8Array;
  readonly contentType: string | null;
  text(): string;
  json<T = unknown>(): T | null;
  form(): HeaderPair[];
}

export interface PreviewHookContext {
  readonly id: number;
  readonly stage: "preview";
  readonly side: "request" | "response";
  readonly method: string;
  readonly url: string;
  readonly host: string;
  readonly path: string;
  readonly query: HeaderPair[];
  readonly status: number | null;
  readonly headers: HeaderPair[];
  readonly body: Uint8Array;
  readonly req: PreviewHookSide;
  readonly res: PreviewHookSide;
  readonly vars: Readonly<Record<string, string>>;
  text(): string;
  json<T = unknown>(): T | null;
  form(): HeaderPair[];
}

export interface PreviewSideOverride {
  body?: BodyInput | Record<string, unknown> | unknown[];
  json?: unknown;
  form?: HeaderPair[] | Record<string, string>;
  contentType?: string;
  note?: string;
  view?: HookViewChild;
  cards?: HookViewChild;
}

export interface PreviewHookResult {
  req?: PreviewSideOverride;
  res?: PreviewSideOverride;
  body?: BodyInput | Record<string, unknown> | unknown[];
  json?: unknown;
  form?: HeaderPair[] | Record<string, string>;
  contentType?: string;
  note?: string;
  view?: HookViewChild;
  cards?: HookViewChild;
}

export interface RunHookContext {
  readonly stage: "run";
  readonly filter: string;
  readonly selectedId: number | null;
  readonly vars: Readonly<Record<string, string>>;
}

export interface RunHookResult {
  summary?: string;
  view?: HookViewChild;
  cards?: HookViewChild;
}

export interface ZniffFlowsApi {
  ids(filter?: string, limit?: number): number[];
  url(id: number): string | null;
  info(id: number): FlowInfo | null;
  requestHeaders(id: number): HeaderPair[] | null;
  requestHeader(id: number, name: string): string | null;
  requestBody(id: number): Uint8Array | null;
  requestText(id: number): string | null;
  responseHeaders(id: number): HeaderPair[] | null;
  responseHeader(id: number, name: string): string | null;
  responseBody(id: number): Uint8Array | null;
  responseText(id: number): string | null;
  setComment(id: number, comment: string): boolean;
  appendComment(id: number, note: string): boolean;
  setPinned(id: number, pinned: boolean): boolean;
}

export interface JwtDecoded {
  readonly header: Record<string, unknown>;
  readonly payload: Record<string, unknown>;
  readonly signingInput: string;
  readonly signatureHex: string;
}

export interface ZniffCodecApi {
  utf8Encode(text: string): Uint8Array;
  utf8Decode(bytes: Uint8Array | ArrayBuffer): string;
  base64Encode(input: BodyInput): string;
  base64Decode(text: string): Uint8Array;
  base64UrlEncode(input: BodyInput): string;
  base64UrlDecode(text: string): Uint8Array;
  hexEncode(input: BodyInput): string;
  hexDecode(text: string): Uint8Array;
  gzip(input: BodyInput): Uint8Array;
  gunzip(bytes: Uint8Array | ArrayBuffer): Uint8Array;
  deflate(input: BodyInput): Uint8Array;
  inflate(bytes: Uint8Array | ArrayBuffer): Uint8Array;
  zlibCompress(input: BodyInput): Uint8Array;
  zlibDecompress(bytes: Uint8Array | ArrayBuffer): Uint8Array;
  zstdCompress(input: BodyInput, level?: number): Uint8Array;
  zstdDecompress(bytes: Uint8Array | ArrayBuffer): Uint8Array;
  brotliDecompress(bytes: Uint8Array | ArrayBuffer): Uint8Array;
  urlEncode(text: string): string;
  urlDecode(text: string): string;
  formUrlEncode(entries: [string, string][] | Record<string, string>): string;
  formUrlDecode(query: string): [string, string][];
  crc32(input: BodyInput): number;
  jwtDecode(token: string): JwtDecoded | null;
  protobufDecodeWire(bytes: Uint8Array | ArrayBuffer): Record<string, unknown>;
}

export interface ZniffCryptoApi {
  md5(input: BodyInput): Uint8Array;
  md5Hex(input: BodyInput): string;
  sha1(input: BodyInput): Uint8Array;
  sha1Hex(input: BodyInput): string;
  sha256(input: BodyInput): Uint8Array;
  sha256Hex(input: BodyInput): string;
  sha384(input: BodyInput): Uint8Array;
  sha384Hex(input: BodyInput): string;
  sha512(input: BodyInput): Uint8Array;
  sha512Hex(input: BodyInput): string;
  hmacMd5(key: BodyInput, data: BodyInput): Uint8Array;
  hmacMd5Hex(key: BodyInput, data: BodyInput): string;
  hmacSha1(key: BodyInput, data: BodyInput): Uint8Array;
  hmacSha1Hex(key: BodyInput, data: BodyInput): string;
  hmacSha256(key: BodyInput, data: BodyInput): Uint8Array;
  hmacSha256Hex(key: BodyInput, data: BodyInput): string;
  hmacSha384(key: BodyInput, data: BodyInput): Uint8Array;
  hmacSha384Hex(key: BodyInput, data: BodyInput): string;
  hmacSha512(key: BodyInput, data: BodyInput): Uint8Array;
  hmacSha512Hex(key: BodyInput, data: BodyInput): string;
  aes128GcmEncrypt(
    key16: BodyInput,
    nonce12: BodyInput,
    plaintext: BodyInput,
    aad?: BodyInput,
  ): Uint8Array;
  aes128GcmDecrypt(
    key16: BodyInput,
    nonce12: BodyInput,
    ciphertextWithTag: Uint8Array | ArrayBuffer,
    aad?: BodyInput,
  ): Uint8Array;
  aes256GcmEncrypt(
    key32: BodyInput,
    nonce12: BodyInput,
    plaintext: BodyInput,
    aad?: BodyInput,
  ): Uint8Array;
  aes256GcmDecrypt(
    key32: BodyInput,
    nonce12: BodyInput,
    ciphertextWithTag: Uint8Array | ArrayBuffer,
    aad?: BodyInput,
  ): Uint8Array;
  chacha20Poly1305Encrypt(
    key32: BodyInput,
    nonce12: BodyInput,
    plaintext: BodyInput,
    aad?: BodyInput,
  ): Uint8Array;
  chacha20Poly1305Decrypt(
    key32: BodyInput,
    nonce12: BodyInput,
    ciphertextWithTag: Uint8Array | ArrayBuffer,
    aad?: BodyInput,
  ): Uint8Array;
  aes128CbcEncrypt(
    key16: BodyInput,
    iv16: BodyInput,
    plaintext: BodyInput,
  ): Uint8Array;
  aes128CbcDecrypt(
    key16: BodyInput,
    iv16: BodyInput,
    ciphertext: Uint8Array | ArrayBuffer,
  ): Uint8Array;
  aes256CbcEncrypt(
    key32: BodyInput,
    iv16: BodyInput,
    plaintext: BodyInput,
  ): Uint8Array;
  aes256CbcDecrypt(
    key32: BodyInput,
    iv16: BodyInput,
    ciphertext: Uint8Array | ArrayBuffer,
  ): Uint8Array;
  pbkdf2Sha256(
    password: BodyInput,
    salt: BodyInput,
    iterations: number,
    keyLen: number,
  ): Uint8Array;
  hkdfSha256(
    secret: BodyInput,
    salt: BodyInput,
    info: BodyInput,
    keyLen: number,
  ): Uint8Array;
  timingSafeEqual(a: BodyInput, b: BodyInput): boolean;
  randomBytes(len: number): Uint8Array;
  uuidV4(): string;
}

export interface ZniffHookSdk {
  readonly flows: ZniffFlowsApi;
  readonly codec: ZniffCodecApi;
  readonly crypto: ZniffCryptoApi;
}

declare global {
  const zniff: ZniffHookSdk;
  function Section(props: HookActionProps & { title?: string; children?: HookViewChild }): HookViewNode;
  function KeyValue(props: HookActionProps & { label?: string; value?: string; children?: string }): HookViewNode;
  function Badge(props: HookActionProps & { tone?: PreviewTone; text?: string; children?: string }): HookViewNode;
  function Code(props: HookActionProps & { language?: string; code?: string; children?: string }): HookViewNode;
  function View(props: HookActionProps & { gap?: number; padding?: number; bordered?: boolean; children?: HookViewChild }): HookViewNode;
  function Row(props: HookActionProps & { gap?: number; wrap?: boolean; align?: "start" | "center" | "end"; justify?: "start" | "center" | "end" | "between"; children?: HookViewChild }): HookViewNode;
  function Col(props: HookActionProps & { gap?: number; children?: HookViewChild }): HookViewNode;
  function Text(props: HookActionProps & { tone?: PreviewTone; mono?: boolean; bold?: boolean; size?: TextSizeTier; text?: string; children?: string }): HookViewNode;
  function Divider(): HookViewNode;
  function Callout(props: HookActionProps & { tone?: PreviewTone; title?: string; text?: string; children?: HookViewChild | string }): HookViewNode;
  function Table(props: { columns: string[]; rows: (string | number | boolean | null | undefined)[][] }): HookViewNode;
  function Collapsible(props: { title: string; defaultOpen?: boolean; children?: HookViewChild }): HookViewNode;
  function Tabs(props: { children?: HookViewChild }): HookViewNode;
  function Tab(props: { label: string; children?: HookViewChild }): HookViewNode;

  namespace JSX {
    type Element = HookViewNode;
    interface IntrinsicElements {}
  }
}

declare module "zniff/hook" {
  export {
    HookStage,
    HeaderPair,
    PreviewTone,
    TextSizeTier,
    HookActionProps,
    BodyInput,
    FlowInfo,
    HookViewNode,
    HookViewChild,
    PreviewCardNode,
    PreviewCardChild,
    Section,
    KeyValue,
    Badge,
    Code,
    View,
    Row,
    Col,
    Text,
    Divider,
    Callout,
    Table,
    Collapsible,
    Tabs,
    Tab,
    RequestHookContext,
    RequestShortCircuitResponse,
    RequestHookResult,
    ResponseHookContext,
    ResponseHookResult,
    PreviewHookSide,
    PreviewHookContext,
    PreviewSideOverride,
    PreviewHookResult,
    RunHookContext,
    RunHookResult,
    ZniffFlowsApi,
    JwtDecoded,
    ZniffCodecApi,
    ZniffCryptoApi,
    ZniffHookSdk,
  };
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_hook_id_and_script_rejects_traversal_and_reserved() {
        assert_eq!(sanitize_hook_id("biz-crypto").unwrap(), "biz-crypto");
        assert_eq!(sanitize_hook_id("auth_v2.tsx").unwrap(), "auth_v2");
        assert!(sanitize_hook_id("../evil").is_err());
        assert!(sanitize_hook_id("a/b").is_err());
        assert!(sanitize_hook_id("zniff-hook").is_err());
        assert!(sanitize_hook_id("").is_err());

        assert_eq!(
            sanitize_hook_script_file("@biz-crypto").unwrap(),
            "biz-crypto.tsx"
        );
        assert_eq!(sanitize_hook_script_file("dec.ts").unwrap(), "dec.ts");
        assert!(sanitize_hook_script_file("../dec.tsx").is_err());
        assert_eq!(
            sanitize_hook_reference("@biz-crypto"),
            Some("biz-crypto.tsx".into())
        );
        assert!(sanitize_hook_reference("http://127.0.0.1:9000").is_none());
    }

    #[test]
    fn sanitize_script_hooks_deduplicates_clamps_and_bounds_vars() {
        let mut big_vars = BTreeMap::new();
        for idx in 0..40 {
            big_vars.insert(format!("k{idx:02}"), format!("v{idx}"));
        }
        big_vars.insert("".into(), "ignored".into());
        big_vars.insert("too_big".into(), "x".repeat(MAX_VAR_VAL_BYTES + 1));

        let sanitized = sanitize_script_hooks(vec![
            ScriptHookConfig {
                id: "biz-crypto".into(),
                name: "  Decrypt API  ".into(),
                script: "biz-crypto".into(),
                enabled: true,
                match_pattern: " api.vicanso.dev/v1 ".into(),
                stages: vec![HookStage::Request, HookStage::Request, HookStage::Preview],
                timeout_ms: 2,
                vars: big_vars,
            },
            ScriptHookConfig {
                id: "BIZ-CRYPTO".into(),
                name: "dup".into(),
                script: "dup.tsx".into(),
                enabled: false,
                match_pattern: "*".into(),
                stages: vec![],
                timeout_ms: 99_999,
                vars: BTreeMap::new(),
            },
            ScriptHookConfig {
                id: "../bad".into(),
                name: "bad".into(),
                script: "bad.tsx".into(),
                enabled: true,
                match_pattern: "*".into(),
                stages: vec![],
                timeout_ms: 80,
                vars: BTreeMap::new(),
            },
        ]);

        assert_eq!(sanitized.len(), 1);
        assert_eq!(sanitized[0].name, "Decrypt API");
        assert_eq!(sanitized[0].script, "biz-crypto.tsx");
        assert_eq!(sanitized[0].match_pattern, "api.vicanso.dev/v1");
        assert_eq!(
            sanitized[0].stages,
            vec![HookStage::Request, HookStage::Preview]
        );
        assert_eq!(sanitized[0].timeout_ms, MIN_HOOK_TIMEOUT_MS);
        assert_eq!(sanitized[0].vars.len(), MAX_HOOK_VARS);
        assert!(!sanitized[0].vars.contains_key("too_big"));
    }

    #[test]
    fn preview_card_validation_enforces_depth_count_and_budget() {
        let valid = vec![PreviewCardNode::Section {
            title: "Crypto".into(),
            children: vec![
                PreviewCardNode::Badge {
                    tone: PreviewTone::Ok,
                    text: "AES-256-GCM".into(),
                    action: HookViewAction::default(),
                },
                PreviewCardNode::KeyValue {
                    label: "Host".into(),
                    value: "api.vicanso.dev".into(),
                    action: HookViewAction::default(),
                },
                PreviewCardNode::Code {
                    language: Some("json".into()),
                    code: r#"{"ok":true}"#.into(),
                    action: HookViewAction::default(),
                },
            ],
            action: HookViewAction::default(),
        }];
        assert!(validate_preview_cards(&valid).is_ok());

        let mut deep = PreviewCardNode::Badge {
            tone: PreviewTone::Neutral,
            text: "leaf".into(),
            action: HookViewAction::default(),
        };
        for d in 0..MAX_PREVIEW_CARD_DEPTH {
            deep = PreviewCardNode::Section {
                title: format!("d{d}"),
                children: vec![deep],
                action: HookViewAction::default(),
            };
        }
        assert!(validate_preview_cards(&[deep]).is_err());

        let huge_text = vec![PreviewCardNode::Code {
            language: None,
            code: "a".repeat(MAX_PREVIEW_CARD_TEXT_BYTES + 1),
            action: HookViewAction::default(),
        }];
        assert!(validate_preview_cards(&huge_text).is_err());
    }

    #[test]
    fn transpiles_multi_stage_hook_script_with_flows_loop_and_jsx_cards() {
        let src = r#"
import type { RequestHookContext, PreviewHookContext } from "zniff/hook";
import { Section, KeyValue, Badge, Code } from "zniff/hook";

export function onRequest(ctx: RequestHookContext) {
  for (const id of zniff.flows.ids("host:api.vicanso.dev 200", 5)) {
    const token = zniff.flows.responseHeader(id, "x-session-key");
    if (token) {
      const ok = token.length > 0 ? "1" : "0";
      return { headers: [...ctx.headers, ["x-key-flag", ok]] };
    }
  }
}

export function onPreview(ctx: PreviewHookContext) {
  return {
    res: { body: ctx.res.body, contentType: "application/json", note: "AES" },
    cards: (
      <Section title="解密结果">
        <Badge tone="ok">AES-256-GCM</Badge>
        <KeyValue label="Host" value={ctx.host} />
        <Code language="json">{"{}"}</Code>
      </Section>
    ),
  };
}
export function onRun(ctx) {
  return { summary: String(zniff.flows.ids(ctx.filter).length) };
}
"#;
        let js = transpile_hook_tsx(src).expect("transpile hook TSX");
        assert!(!js.contains("import "));
        assert!(!js.contains("export "));
        assert!(js.contains("function onRequest(ctx)"));
        assert!(js.contains("function onPreview(ctx)"));
        assert!(js.contains("function onRun(ctx)"));
        assert!(js.contains("for (const id of zniff.flows.ids"));
        assert!(js.contains("token.length > 0 ? \"1\" : \"0\""));
        assert!(js.contains("__zniff_h(Section"));
        assert!(js.contains("__zniff_h(Badge"));
        assert!(js.contains("__zniff_h(KeyValue"));
        assert!(js.contains("__zniff_h(Code"));
    }
}
