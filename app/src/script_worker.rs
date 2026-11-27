//! Lazy, bounded, warm-reused background worker for sandboxed QuickJS scripts.
//!
//! Design invariants (`.agents/rules/scripting-perf.md`):
//! - Never spawns a thread or allocates a QuickJS VM until a script actually runs.
//! - UI and Tokio async handlers never execute QuickJS directly on their threads.
//! - Enforces tiered memory/stack/wall-clock quotas and a 3-strike circuit breaker.
//! - Automatically evicts the VM and frees memory after 60s idle or tray shutdown.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rquickjs::{CaughtError, Context, Runtime};

use crate::layout_spec::{LayoutContext, LayoutSpec};
use crate::layout_tsx::transpile_layout_tsx;

/// Maximum QuickJS heap memory allowed for evaluating a layout script (8 MiB).
pub const LAYOUT_MEM_LIMIT_BYTES: usize = 8 * 1024 * 1024;

/// Maximum QuickJS call stack allowed for a layout script (256 KiB).
pub const LAYOUT_STACK_LIMIT_BYTES: usize = 256 * 1024;

/// Maximum wall-clock duration allowed per layout script evaluation (25 ms).
pub const LAYOUT_TIMEOUT_MS: u64 = 25;

/// Maximum QuickJS heap memory allowed for a packet transform script (16 MiB).
pub const PACKET_MEM_LIMIT_BYTES: usize = 16 * 1024 * 1024;

/// Maximum QuickJS call stack allowed for a packet transform script (256 KiB).
pub const PACKET_STACK_LIMIT_BYTES: usize = 256 * 1024;

/// Maximum wall-clock duration allowed per packet transform script (80 ms).
pub const PACKET_TIMEOUT_MS: u64 = 80;

/// Maximum input body bytes passed directly via `Uint8Array` to packet scripts (1 MiB).
pub const PACKET_MAX_INPUT_BYTES: usize = 1024 * 1024;

/// Maximum serialized IR output bytes accepted from a packet transform script (512 KiB).
pub const PACKET_MAX_OUTPUT_BYTES: usize = 512 * 1024;

/// Maximum aggregate bytes retained in the packet transform result LRU cache (8 MiB).
pub const PACKET_CACHE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Maximum entry count retained in the packet transform result LRU cache.
pub const PACKET_CACHE_MAX_ENTRIES: usize = 64;

/// Idle duration after which the worker thread drops the QuickJS VM and exits.
pub const SCRIPT_WORKER_IDLE_TTL: Duration = Duration::from_secs(60);

/// Bounded command queue depth for the script worker.
pub const SCRIPT_QUEUE_CAP: usize = 16;

/// Consecutive script timeouts before tripping the automatic circuit breaker.
pub const CIRCUIT_BREAKER_TRIP_COUNT: u32 = 3;

const TRANSPILER_CACHE_MAX_BYTES: usize = 1024 * 1024;
const TRANSPILER_CACHE_MAX_ENTRIES: usize = 32;
const VM_GC_THRESHOLD_BYTES: i64 = 1024 * 1024;
const VM_RECYCLE_THRESHOLD_BYTES: i64 = 4 * 1024 * 1024;
const WORKER_SYNC_WAIT_MS: u64 = 250;

pub(crate) const SANDBOX_PRELUDE: &str = r#"
const __zniff_fragment = Symbol("fragment");
function __zniff_push_child(out, child) {
  if (child === null || child === undefined || typeof child === "boolean") return;
  if (Array.isArray(child)) {
    for (let i = 0; i < child.length; i++) __zniff_push_child(out, child[i]);
  } else {
    out.push(child);
  }
}
function pane(name, size) {
  const node = { type: "pane", name };
  if (size !== undefined && size !== null) node.size = size;
  return node;
}
function row(...args) {
  let size;
  let items = args;
  if (
    args.length > 0 &&
    args[0] &&
    typeof args[0] === "object" &&
    !Array.isArray(args[0]) &&
    typeof args[0].type !== "string" &&
    ("size" in args[0] || "children" in args[0])
  ) {
    size = args[0].size;
    items = args.slice(1);
  }
  const children = [];
  __zniff_push_child(children, items);
  const node = { type: "row", children };
  if (size !== undefined && size !== null) node.size = size;
  return node;
}
function col(...args) {
  let size;
  let items = args;
  if (
    args.length > 0 &&
    args[0] &&
    typeof args[0] === "object" &&
    !Array.isArray(args[0]) &&
    typeof args[0].type !== "string" &&
    ("size" in args[0] || "children" in args[0])
  ) {
    size = args[0].size;
    items = args.slice(1);
  }
  const children = [];
  __zniff_push_child(children, items);
  const node = { type: "col", children };
  if (size !== undefined && size !== null) node.size = size;
  return node;
}
function Layout(props) {
  const kids = [];
  __zniff_push_child(kids, props && props.children);
  if (kids.length !== 1) {
    throw new Error("<Layout> must contain exactly one root node (<Row>, <Col>, or <Pane>)");
  }
  const out = { root: kids[0] };
  if (props && typeof props.name === "string") out.name = props.name;
  if (props && typeof props.footer === "string") out.footer = props.footer;
  return out;
}
function Row(props) {
  const kids = [];
  __zniff_push_child(kids, props && props.children);
  const node = { type: "row", children: kids };
  if (props && props.size !== undefined && props.size !== null) node.size = props.size;
  return node;
}
function Col(props) {
  const kids = [];
  __zniff_push_child(kids, props && props.children);
  const node = { type: "col", children: kids };
  if (props && props.size !== undefined && props.size !== null) node.size = props.size;
  return node;
}
function Pane(props) {
  if (!props || typeof props.name !== "string") {
    throw new Error("<Pane> requires a string `name` prop");
  }
  const node = { type: "pane", name: props.name };
  if (props.size !== undefined && props.size !== null) node.size = props.size;
  return node;
}
function __zniff_text_children(children) {
  if (children === null || children === undefined || typeof children === "boolean") return "";
  if (Array.isArray(children)) {
    let out = "";
    for (let i = 0; i < children.length; i++) out += __zniff_text_children(children[i]);
    return out;
  }
  return String(children);
}
function Section(props) {
  const kids = [];
  __zniff_push_child(kids, props && props.children);
  return { type: "section", title: props && props.title !== undefined && props.title !== null ? String(props.title) : "", children: kids };
}
function KeyValue(props) {
  const value = props && props.value !== undefined && props.value !== null
    ? String(props.value)
    : __zniff_text_children(props && props.children);
  return { type: "key_value", label: props && props.label !== undefined && props.label !== null ? String(props.label) : "", value };
}
function Badge(props) {
  const text = props && props.text !== undefined && props.text !== null
    ? String(props.text)
    : __zniff_text_children(props && props.children);
  const tone = props && typeof props.tone === "string" ? props.tone : "neutral";
  return { type: "badge", tone, text };
}
function Code(props) {
  const code = props && props.code !== undefined && props.code !== null
    ? String(props.code)
    : __zniff_text_children(props && props.children);
  const out = { type: "code", code };
  if (props && typeof props.language === "string" && props.language.length > 0) out.language = props.language;
  return out;
}
function __zniff_h(tag, props, ...children) {
  if (tag === __zniff_fragment) {
    const flat = [];
    __zniff_push_child(flat, children);
    return flat;
  }
  const merged = Object.assign({}, props || {});
  if (children.length > 0) merged.children = children;
  if (typeof tag === "function") return tag(merged);
  throw new Error("Unsupported TSX tag `" + String(tag) + "`: expected Layout, Row, Col, Pane, Section, KeyValue, Badge, or Code");
}
(() => {
  const deny = (label) => function () {
    throw new TypeError(label + " is disabled inside zniff scripts");
  };
  const replace = (fn, value) => {
    Object.defineProperty(Object.getPrototypeOf(fn), "constructor", {
      value, writable: false, enumerable: false, configurable: false,
    });
  };
  const blocked = deny("the Function constructor");
  blocked.prototype = Function.prototype;
  replace(function () {}, blocked);
  replace(async function () {}, deny("the AsyncFunction constructor"));
  replace(function* () {}, deny("the GeneratorFunction constructor"));
  replace(async function* () {}, deny("the AsyncGeneratorFunction constructor"));
  Object.defineProperty(globalThis, "Function", { value: blocked, writable: false, configurable: false });
  delete globalThis.eval;
  const defineSlot = (name) => {
    let slot = undefined;
    Object.defineProperty(globalThis, name, {
      get() { return slot; },
      set(v) { slot = v; },
      enumerable: false,
      configurable: false,
    });
  };
  defineSlot("__zniff_layout_export");
  defineSlot("__zniff_hook_default");
  defineSlot("__zniff_hook_exports");
  defineSlot("__zniff_hook_input");
  if (globalThis.zniff && typeof globalThis.zniff === "object") {
    Object.freeze(globalThis.zniff.flows);
    Object.freeze(globalThis.zniff.codec);
    Object.freeze(globalThis.zniff.crypto);
    Object.freeze(globalThis.zniff);
  }
  for (const proto of [
    Object.prototype, Array.prototype, Function.prototype, String.prototype,
    Number.prototype, Boolean.prototype, RegExp.prototype, Map.prototype, Set.prototype,
  ]) {
    Object.freeze(proto);
  }
  Object.freeze(globalThis);
})();
"#;

/// LRU cache strictly bounded by both aggregate byte length and entry count.
#[derive(Debug)]
pub struct ByteBoundedLru<K, V> {
    max_bytes: usize,
    max_entries: usize,
    total_bytes: usize,
    entries: Vec<(K, V, usize)>,
}

impl<K: PartialEq, V> ByteBoundedLru<K, V> {
    pub fn new(max_bytes: usize, max_entries: usize) -> Self {
        Self {
            max_bytes,
            max_entries: max_entries.max(1),
            total_bytes: 0,
            entries: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.total_bytes = 0;
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        let pos = self.entries.iter().position(|(k, _, _)| k == key)?;
        if pos + 1 != self.entries.len() {
            let item = self.entries.remove(pos);
            self.entries.push(item);
        }
        self.entries.last().map(|(_, v, _)| v)
    }

    pub fn insert(&mut self, key: K, value: V, byte_len: usize) -> bool {
        if byte_len > self.max_bytes {
            return false;
        }
        if let Some(pos) = self.entries.iter().position(|(k, _, _)| k == &key) {
            let (_, _, old_bytes) = self.entries.remove(pos);
            self.total_bytes = self.total_bytes.saturating_sub(old_bytes);
        }
        while !self.entries.is_empty()
            && (self.entries.len() >= self.max_entries
                || self.total_bytes.saturating_add(byte_len) > self.max_bytes)
        {
            let (_, _, dropped) = self.entries.remove(0);
            self.total_bytes = self.total_bytes.saturating_sub(dropped);
        }
        self.total_bytes = self.total_bytes.saturating_add(byte_len);
        self.entries.push((key, value, byte_len));
        true
    }
}

/// Outcome of evaluating a layout script in the worker, including timeout/circuit flags.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutEvalReport {
    pub result: Result<LayoutSpec, String>,
    pub timed_out: bool,
    pub circuit_tripped: bool,
}

/// Pending debounced active layout request submitted by UI resize.
#[derive(Debug, Clone)]
pub(crate) struct ActiveEvalRequest {
    pub seq: u64,
    pub script_id: String,
    pub source: String,
    pub ctx: LayoutContext,
}

/// Completed async active layout evaluation ready for the UI tick to consume.
#[derive(Debug, Clone)]
pub(crate) struct ActiveEvalResult {
    pub seq: u64,
    pub script_id: String,
    pub ctx: LayoutContext,
    pub report: LayoutEvalReport,
}

/// Pending debounced Settings draft validation request.
#[derive(Debug, Clone)]
pub(crate) struct DraftEvalRequest {
    pub seq: u64,
    pub source: String,
    pub ctx: LayoutContext,
}

/// Completed async draft validation result ready for the UI tick to consume.
#[derive(Debug, Clone)]
pub(crate) struct DraftEvalResult {
    pub seq: u64,
    pub result: Result<LayoutSpec, String>,
}

#[derive(Default)]
struct UiScriptMailbox {
    pending_active: Mutex<Option<ActiveEvalRequest>>,
    completed_active: Mutex<Option<ActiveEvalResult>>,
    pending_draft: Mutex<Option<DraftEvalRequest>>,
    completed_draft: Mutex<Option<DraftEvalResult>>,
    wake_pending: AtomicBool,
}

type WorkerCustomJob =
    Box<dyn FnOnce(&mut Option<WarmLayoutVm>, &mut ByteBoundedLru<String, Arc<String>>) + Send>;

enum WorkerCmd {
    WakeUi,
    ClearCaches,
    Shutdown,
    EvalSync {
        script_id: Option<String>,
        source: String,
        ctx: LayoutContext,
        reply: mpsc::SyncSender<LayoutEvalReport>,
    },
    EvalAsync {
        script_id: Option<String>,
        source: String,
        ctx: LayoutContext,
        reply: tokio::sync::oneshot::Sender<LayoutEvalReport>,
    },
    CustomJob(WorkerCustomJob),
}

struct ActiveWorker {
    tx: SyncSender<WorkerCmd>,
    epoch: u64,
    join: Option<JoinHandle<()>>,
}

static WORKER_SLOT: OnceLock<Mutex<Option<ActiveWorker>>> = OnceLock::new();
static WORKER_EPOCH: AtomicU64 = AtomicU64::new(1);
static LIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);
static VM_INIT_COUNT: AtomicU64 = AtomicU64::new(0);
static WARM_EVAL_COUNT: AtomicU64 = AtomicU64::new(0);
static MAILBOX: OnceLock<Arc<UiScriptMailbox>> = OnceLock::new();
static CIRCUIT_BREAKERS: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
static PACKET_CACHE: OnceLock<Mutex<ByteBoundedLru<String, Vec<u8>>>> = OnceLock::new();

fn worker_slot() -> &'static Mutex<Option<ActiveWorker>> {
    WORKER_SLOT.get_or_init(|| Mutex::new(None))
}

fn mailbox() -> &'static Arc<UiScriptMailbox> {
    MAILBOX.get_or_init(|| Arc::new(UiScriptMailbox::default()))
}

fn circuit_breakers() -> &'static Mutex<HashMap<String, u32>> {
    CIRCUIT_BREAKERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn packet_cache() -> &'static Mutex<ByteBoundedLru<String, Vec<u8>>> {
    PACKET_CACHE.get_or_init(|| {
        Mutex::new(ByteBoundedLru::new(
            PACKET_CACHE_MAX_BYTES,
            PACKET_CACHE_MAX_ENTRIES,
        ))
    })
}

/// Number of QuickJS `Runtime` instances initialized since process start.
pub fn vm_init_count() -> u64 {
    VM_INIT_COUNT.load(Ordering::Relaxed)
}

/// Number of warm layout script evaluations executed on reused VMs.
pub fn warm_eval_count() -> u64 {
    WARM_EVAL_COUNT.load(Ordering::Relaxed)
}

/// Returns true if the background script worker thread is currently alive.
pub fn is_worker_running() -> bool {
    LIVE_WORKERS.load(Ordering::Acquire) > 0
}

/// Returns true if `script_id` has tripped the consecutive-timeout circuit breaker.
pub fn is_circuit_tripped(script_id: &str) -> bool {
    if script_id.trim().is_empty() {
        return false;
    }
    circuit_breakers()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(script_id)
        .copied()
        .unwrap_or(0)
        >= CIRCUIT_BREAKER_TRIP_COUNT
}

/// Resets the consecutive-timeout counter for `script_id`.
pub fn reset_script_circuit(script_id: &str) {
    if script_id.trim().is_empty() {
        return;
    }
    circuit_breakers()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(script_id);
}

/// Clears all script circuit-breaker counters.
pub fn reset_all_script_circuits() {
    circuit_breakers()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}

fn record_eval_outcome(script_id: Option<&str>, timed_out: bool) -> bool {
    let Some(id) = script_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return false;
    };
    let mut map = circuit_breakers().lock().unwrap_or_else(|e| e.into_inner());
    if timed_out {
        let count = map.entry(id.to_string()).or_insert(0);
        *count = count.saturating_add(1);
        *count >= CIRCUIT_BREAKER_TRIP_COUNT
    } else {
        map.remove(id);
        false
    }
}

/// Stores a packet-transform cache entry under the strict 8 MiB / 64-entry LRU budget.
pub fn store_packet_cache_entry(key: String, bytes: Vec<u8>) -> bool {
    let len = bytes.len();
    packet_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, bytes, len)
}

/// Looks up a cached packet-transform result, promoting it to MRU.
pub fn get_packet_cache_entry(key: &str) -> Option<Vec<u8>> {
    packet_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key.to_string())
        .cloned()
}

/// Returns `(entry_count, total_bytes)` currently held in the packet transform cache.
pub fn packet_cache_stats() -> (usize, usize) {
    let guard = packet_cache().lock().unwrap_or_else(|e| e.into_inner());
    (guard.len(), guard.total_bytes())
}

/// Clears packet transform caches and, if the worker is alive, clears its transpiler cache.
pub fn clear_script_caches() {
    crate::webhook::clear_preview_cache();
    packet_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    let tx = worker_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|w| w.tx.clone());
    if let Some(tx) = tx {
        let _ = tx.try_send(WorkerCmd::ClearCaches);
    }
}

/// Requests the script worker (if running) to drop its QuickJS VM and exit immediately.
pub fn shutdown_script_worker() {
    packet_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    let active = worker_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    if let Some(mut active) = active {
        let _ = active.tx.try_send(WorkerCmd::Shutdown);
        drop(active.tx);
        active.join = None;
    }
}

/// Shuts down the script worker and joins its thread so VM teardown has completed.
#[cfg(test)]
pub(crate) fn shutdown_script_worker_blocking() {
    packet_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    let active = worker_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    if let Some(mut active) = active {
        let _ = active.tx.send(WorkerCmd::Shutdown);
        drop(active.tx);
        if let Some(handle) = active.join.take() {
            let _ = handle.join();
        }
    }
}

pub(crate) struct WarmLayoutVm {
    rt: Runtime,
    ctx: Context,
    created_at: Instant,
    deadline_ns: Arc<AtomicU64>,
    timed_out: Arc<AtomicBool>,
}

impl WarmLayoutVm {
    pub(crate) fn new() -> Result<Self, String> {
        let rt = Runtime::new().map_err(|err| format!("failed to init QuickJS runtime: {err}"))?;
        rt.set_memory_limit(LAYOUT_MEM_LIMIT_BYTES);
        rt.set_max_stack_size(LAYOUT_STACK_LIMIT_BYTES);
        let created_at = Instant::now();
        let deadline_ns = Arc::new(AtomicU64::new(0));
        let timed_out = Arc::new(AtomicBool::new(false));
        let dl = Arc::clone(&deadline_ns);
        let to = Arc::clone(&timed_out);
        rt.set_interrupt_handler(Some(Box::new(move || {
            let limit = dl.load(Ordering::Relaxed);
            if limit == 0 {
                return false;
            }
            let elapsed = created_at.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
            if elapsed >= limit {
                to.store(true, Ordering::Release);
                true
            } else {
                false
            }
        })));
        let ctx =
            Context::full(&rt).map_err(|err| format!("failed to create QuickJS context: {err}"))?;
        ctx.with(|cx| -> Result<(), String> {
            crate::hook_bridge::install_zniff_globals(&cx)
                .map_err(|err| format_js_error(&cx, err))?;
            cx.eval::<(), _>(SANDBOX_PRELUDE)
                .map_err(|err| format_js_error(&cx, err))
        })?;
        VM_INIT_COUNT.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            rt,
            ctx,
            created_at,
            deadline_ns,
            timed_out,
        })
    }

    fn eval_transpiled(
        &self,
        transpiled: &str,
        ctx: LayoutContext,
    ) -> (Result<LayoutSpec, String>, bool) {
        let ctx_json = match serde_json::to_string(&ctx) {
            Ok(s) => s,
            Err(err) => return (Err(err.to_string()), false),
        };
        let timeout_ns = LAYOUT_TIMEOUT_MS.saturating_mul(1_000_000);
        let now_ns = self
            .created_at
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX - timeout_ns - 1)) as u64;
        self.timed_out.store(false, Ordering::Release);
        self.deadline_ns
            .store(now_ns.saturating_add(timeout_ns).max(1), Ordering::Release);

        let wrapped = format!(
            "(() => {{\n\
             \"use strict\";\n\
             globalThis.__zniff_layout_export = undefined;\n\
             {transpiled}\n\
             const __exp = globalThis.__zniff_layout_export;\n\
             globalThis.__zniff_layout_export = undefined;\n\
             if (__exp === undefined || __exp === null) throw new Error('layout script exported nothing');\n\
             const __ctx = Object.freeze({ctx_json});\n\
             const __res = typeof __exp === 'function' ? __exp(__ctx) : __exp;\n\
             if (!__res || typeof __res !== 'object') throw new Error('layout script must return a LayoutSpec object');\n\
             return JSON.stringify(__res);\n\
             }})()"
        );

        let eval_res = self.ctx.with(|cx| {
            cx.eval::<String, _>(wrapped.as_str())
                .map_err(|err| format_js_error(&cx, err))
        });
        self.deadline_ns.store(0, Ordering::Release);
        let did_timeout = self.timed_out.load(Ordering::Acquire);
        if did_timeout {
            return (
                Err(format!(
                    "layout script timed out after {LAYOUT_TIMEOUT_MS} ms"
                )),
                true,
            );
        }
        let parsed =
            eval_res.and_then(|json| LayoutSpec::parse_json(&json).map_err(|e| e.to_string()));
        (parsed, false)
    }

    pub(crate) fn maintain_memory(&self) -> bool {
        let usage = self.rt.memory_usage();
        if usage.memory_used_size > VM_GC_THRESHOLD_BYTES {
            self.rt.run_gc();
        }
        self.rt.memory_usage().memory_used_size <= VM_RECYCLE_THRESHOLD_BYTES
    }

    /// Runs a scoped JS evaluation inside the warm VM under a custom memory & wall-clock budget.
    pub(crate) fn with_budget<R>(
        &self,
        mem_limit: usize,
        timeout_ms: u64,
        f: impl FnOnce(&Context) -> Result<R, String>,
    ) -> (Result<R, String>, bool) {
        let clamped_ms = timeout_ms.clamp(
            crate::hook_spec::MIN_HOOK_TIMEOUT_MS,
            crate::hook_spec::MAX_HOOK_TIMEOUT_MS,
        );
        self.rt.set_memory_limit(mem_limit);
        let timeout_ns = clamped_ms.saturating_mul(1_000_000);
        let now_ns = self
            .created_at
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX - timeout_ns - 1)) as u64;
        self.timed_out.store(false, Ordering::Release);
        self.deadline_ns
            .store(now_ns.saturating_add(timeout_ns).max(1), Ordering::Release);

        let result = f(&self.ctx);

        self.deadline_ns.store(0, Ordering::Release);
        self.rt.set_memory_limit(LAYOUT_MEM_LIMIT_BYTES);
        let did_timeout = self.timed_out.load(Ordering::Acquire);
        if did_timeout {
            (Err(format!("script timed out after {clamped_ms} ms")), true)
        } else {
            (result, false)
        }
    }
}

pub(crate) fn format_js_err(cx: &rquickjs::Ctx<'_>, err: rquickjs::Error) -> String {
    format_js_error(cx, err)
}

pub(crate) fn note_warm_eval() {
    WARM_EVAL_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_hook_eval_outcome(circuit_key: &str, timed_out: bool) -> bool {
    record_eval_outcome(Some(circuit_key), timed_out)
}

fn format_js_error(cx: &rquickjs::Ctx<'_>, err: rquickjs::Error) -> String {
    CaughtError::from_error(cx, err).to_string()
}

fn run_layout_eval(
    vm_slot: &mut Option<WarmLayoutVm>,
    transpiled_cache: &mut ByteBoundedLru<String, Arc<String>>,
    script_id: Option<&str>,
    source: &str,
    ctx: LayoutContext,
) -> LayoutEvalReport {
    if let Some(id) = script_id.map(str::trim).filter(|id| !id.is_empty())
        && is_circuit_tripped(id)
    {
        return LayoutEvalReport {
            result: Err(format!(
                "circuit breaker tripped after {CIRCUIT_BREAKER_TRIP_COUNT} consecutive timeouts (>{LAYOUT_TIMEOUT_MS} ms)"
            )),
            timed_out: false,
            circuit_tripped: true,
        };
    }

    let transpiled = if let Some(cached) = transpiled_cache.get(&source.to_string()) {
        Arc::clone(cached)
    } else {
        match transpile_layout_tsx(source) {
            Ok(js) => {
                let arc = Arc::new(js);
                let byte_len = source.len().saturating_add(arc.len());
                transpiled_cache.insert(source.to_string(), Arc::clone(&arc), byte_len);
                arc
            }
            Err(err) => {
                record_eval_outcome(script_id, false);
                return LayoutEvalReport {
                    result: Err(err),
                    timed_out: false,
                    circuit_tripped: false,
                };
            }
        }
    };

    if vm_slot.is_none() {
        match WarmLayoutVm::new() {
            Ok(vm) => *vm_slot = Some(vm),
            Err(err) => {
                return LayoutEvalReport {
                    result: Err(err),
                    timed_out: false,
                    circuit_tripped: false,
                };
            }
        }
    } else {
        WARM_EVAL_COUNT.fetch_add(1, Ordering::Relaxed);
    }

    let Some(vm) = vm_slot.as_ref() else {
        return LayoutEvalReport {
            result: Err("script VM unavailable".to_string()),
            timed_out: false,
            circuit_tripped: false,
        };
    };

    let (result, timed_out) = vm.eval_transpiled(transpiled.as_str(), ctx);
    let keep_vm = !timed_out && vm.maintain_memory();
    if !keep_vm {
        *vm_slot = None;
    }

    let tripped = record_eval_outcome(script_id, timed_out);
    let result = if tripped {
        Err(format!(
            "circuit breaker tripped after {CIRCUIT_BREAKER_TRIP_COUNT} consecutive timeouts (>{LAYOUT_TIMEOUT_MS} ms)"
        ))
    } else {
        result
    };

    LayoutEvalReport {
        result,
        timed_out,
        circuit_tripped: tripped,
    }
}

fn ensure_worker_sender() -> Result<SyncSender<WorkerCmd>, String> {
    let mut guard = worker_slot().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(active) = guard.as_ref() {
        return Ok(active.tx.clone());
    }
    let (tx, rx) = mpsc::sync_channel::<WorkerCmd>(SCRIPT_QUEUE_CAP);
    let epoch = WORKER_EPOCH.fetch_add(1, Ordering::Relaxed);
    let mb = Arc::clone(mailbox());
    LIVE_WORKERS.fetch_add(1, Ordering::Release);
    let handle = std::thread::Builder::new()
        .name("zniff-script-worker".to_string())
        .spawn(move || worker_loop(epoch, rx, mb))
        .map_err(|err| {
            LIVE_WORKERS.fetch_sub(1, Ordering::Release);
            format!("failed to spawn script worker: {err}")
        })?;
    *guard = Some(ActiveWorker {
        tx: tx.clone(),
        epoch,
        join: Some(handle),
    });
    Ok(tx)
}

fn worker_loop(epoch: u64, rx: mpsc::Receiver<WorkerCmd>, mb: Arc<UiScriptMailbox>) {
    let mut vm: Option<WarmLayoutVm> = None;
    let mut transpiled_cache =
        ByteBoundedLru::new(TRANSPILER_CACHE_MAX_BYTES, TRANSPILER_CACHE_MAX_ENTRIES);

    loop {
        let cmd = match rx.recv_timeout(SCRIPT_WORKER_IDLE_TTL) {
            Ok(cmd) => cmd,
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => break,
        };
        match cmd {
            WorkerCmd::Shutdown => break,
            WorkerCmd::ClearCaches => {
                transpiled_cache.clear();
                if let Some(active_vm) = vm.as_ref()
                    && !active_vm.maintain_memory()
                {
                    vm = None;
                }
            }
            WorkerCmd::WakeUi => {
                mb.wake_pending.store(false, Ordering::Release);
                if let Some(req) = mb
                    .pending_active
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                {
                    let report = run_layout_eval(
                        &mut vm,
                        &mut transpiled_cache,
                        Some(&req.script_id),
                        &req.source,
                        req.ctx,
                    );
                    *mb.completed_active
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = Some(ActiveEvalResult {
                        seq: req.seq,
                        script_id: req.script_id,
                        ctx: req.ctx,
                        report,
                    });
                }
                if let Some(req) = mb
                    .pending_draft
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                {
                    let report =
                        run_layout_eval(&mut vm, &mut transpiled_cache, None, &req.source, req.ctx);
                    *mb.completed_draft.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(DraftEvalResult {
                            seq: req.seq,
                            result: report.result,
                        });
                }
            }
            WorkerCmd::EvalSync {
                script_id,
                source,
                ctx,
                reply,
            } => {
                let report = run_layout_eval(
                    &mut vm,
                    &mut transpiled_cache,
                    script_id.as_deref(),
                    &source,
                    ctx,
                );
                let _ = reply.send(report);
            }
            WorkerCmd::EvalAsync {
                script_id,
                source,
                ctx,
                reply,
            } => {
                let report = run_layout_eval(
                    &mut vm,
                    &mut transpiled_cache,
                    script_id.as_deref(),
                    &source,
                    ctx,
                );
                let _ = reply.send(report);
            }
            WorkerCmd::CustomJob(job) => {
                job(&mut vm, &mut transpiled_cache);
            }
        }
    }

    drop(vm);
    let mut guard = worker_slot().lock().unwrap_or_else(|e| e.into_inner());
    if guard.as_ref().is_some_and(|active| active.epoch == epoch) {
        *guard = None;
    }
    LIVE_WORKERS.fetch_sub(1, Ordering::Release);
}

fn wake_ui_worker() {
    let mb = mailbox();
    if mb.wake_pending.swap(true, Ordering::AcqRel) {
        return;
    }
    match ensure_worker_sender() {
        Ok(tx) => {
            if tx.try_send(WorkerCmd::WakeUi).is_err() {
                mb.wake_pending.store(false, Ordering::Release);
            }
        }
        Err(_) => {
            mb.wake_pending.store(false, Ordering::Release);
        }
    }
}

/// Coalesces an active custom layout resize evaluation onto the background worker.
pub(crate) fn request_active_layout_eval(
    seq: u64,
    script_id: String,
    source: String,
    ctx: LayoutContext,
) {
    *mailbox()
        .pending_active
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(ActiveEvalRequest {
        seq,
        script_id,
        source,
        ctx,
    });
    wake_ui_worker();
}

/// Pops the latest completed async active layout evaluation if present.
pub(crate) fn take_active_layout_result() -> Option<ActiveEvalResult> {
    mailbox()
        .completed_active
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
}

/// Coalesces a Settings inline TSX draft validation onto the background worker.
pub(crate) fn request_draft_layout_eval(seq: u64, source: String, ctx: LayoutContext) {
    *mailbox()
        .pending_draft
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(DraftEvalRequest { seq, source, ctx });
    wake_ui_worker();
}

/// Pops the latest completed async draft validation result if present.
pub(crate) fn take_draft_layout_result() -> Option<DraftEvalResult> {
    mailbox()
        .completed_draft
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
}

/// Evaluates a layout script via the lazy background worker, returning the full report.
pub fn eval_layout_report_blocking(
    script_id: Option<&str>,
    source: &str,
    ctx: LayoutContext,
) -> LayoutEvalReport {
    if let Some(id) = script_id.map(str::trim).filter(|id| !id.is_empty())
        && is_circuit_tripped(id)
    {
        return LayoutEvalReport {
            result: Err(format!(
                "circuit breaker tripped after {CIRCUIT_BREAKER_TRIP_COUNT} consecutive timeouts (>{LAYOUT_TIMEOUT_MS} ms)"
            )),
            timed_out: false,
            circuit_tripped: true,
        };
    }

    let tx = match ensure_worker_sender() {
        Ok(tx) => tx,
        Err(err) => {
            return LayoutEvalReport {
                result: Err(err),
                timed_out: false,
                circuit_tripped: false,
            };
        }
    };
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    let cmd = WorkerCmd::EvalSync {
        script_id: script_id.map(str::to_string),
        source: source.to_string(),
        ctx,
        reply: reply_tx,
    };
    if tx.try_send(cmd).is_err() {
        return LayoutEvalReport {
            result: Err("script worker queue is busy".to_string()),
            timed_out: false,
            circuit_tripped: false,
        };
    }
    match reply_rx.recv_timeout(Duration::from_millis(WORKER_SYNC_WAIT_MS)) {
        Ok(report) => report,
        Err(_) => LayoutEvalReport {
            result: Err("script worker timed out waiting for result".to_string()),
            timed_out: true,
            circuit_tripped: false,
        },
    }
}

/// Evaluates a layout script synchronously via the background worker.
pub fn eval_layout_on_worker_blocking(
    script_id: Option<&str>,
    source: &str,
    ctx: LayoutContext,
) -> Result<LayoutSpec, String> {
    eval_layout_report_blocking(script_id, source, ctx).result
}

/// Evaluates a layout script asynchronously on `zniff-script-worker` without blocking Tokio.
pub async fn eval_layout_async(
    script_id: Option<String>,
    source: String,
    ctx: LayoutContext,
) -> Result<LayoutSpec, String> {
    if let Some(id) = script_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        && is_circuit_tripped(id)
    {
        return Err(format!(
            "circuit breaker tripped after {CIRCUIT_BREAKER_TRIP_COUNT} consecutive timeouts (>{LAYOUT_TIMEOUT_MS} ms)"
        ));
    }

    let tx = ensure_worker_sender()?;
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    let cmd = WorkerCmd::EvalAsync {
        script_id,
        source,
        ctx,
        reply: reply_tx,
    };
    if tx.try_send(cmd).is_err() {
        return Err("script worker queue is busy".to_string());
    }
    match tokio::time::timeout(Duration::from_millis(WORKER_SYNC_WAIT_MS), reply_rx).await {
        Ok(Ok(report)) => report.result,
        Ok(Err(_)) => Err("script worker exited before replying".to_string()),
        Err(_) => Err("script worker timed out waiting for result".to_string()),
    }
}

/// Evaluates a fresh, uncached ephemeral QuickJS `Runtime` directly (used only by `perf_bench`).
pub fn eval_layout_cold_ephemeral(source: &str, ctx: LayoutContext) -> Result<LayoutSpec, String> {
    let transpiled = transpile_layout_tsx(source)?;
    let vm = WarmLayoutVm::new()?;
    vm.eval_transpiled(&transpiled, ctx).0
}

/// Runs a packet/hook job asynchronously on the warm worker VM without blocking Tokio.
pub(crate) async fn run_vm_task_async<F, R>(wait_timeout_ms: u64, job: F) -> Result<R, String>
where
    F: FnOnce(
            &mut Option<WarmLayoutVm>,
            &mut ByteBoundedLru<String, Arc<String>>,
        ) -> Result<R, String>
        + Send
        + 'static,
    R: Send + 'static,
{
    let tx = ensure_worker_sender()?;
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    let cmd = WorkerCmd::CustomJob(Box::new(move |vm, cache| {
        let _ = reply_tx.send(job(vm, cache));
    }));
    if tx.try_send(cmd).is_err() {
        return Err("script worker queue is busy".to_string());
    }
    let budget = wait_timeout_ms.saturating_add(150).max(WORKER_SYNC_WAIT_MS);
    match tokio::time::timeout(Duration::from_millis(budget), reply_rx).await {
        Ok(Ok(res)) => res,
        Ok(Err(_)) => Err("script worker exited before replying".to_string()),
        Err(_) => Err("script worker timed out waiting for result".to_string()),
    }
}

/// Runs a packet/hook job synchronously via the warm worker VM with a bounded wait.
pub(crate) fn run_vm_task_blocking<F, R>(wait_timeout_ms: u64, job: F) -> Result<R, String>
where
    F: FnOnce(
            &mut Option<WarmLayoutVm>,
            &mut ByteBoundedLru<String, Arc<String>>,
        ) -> Result<R, String>
        + Send
        + 'static,
    R: Send + 'static,
{
    let tx = ensure_worker_sender()?;
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    let cmd = WorkerCmd::CustomJob(Box::new(move |vm, cache| {
        let _ = reply_tx.send(job(vm, cache));
    }));
    if tx.try_send(cmd).is_err() {
        return Err("script worker queue is busy".to_string());
    }
    let budget = wait_timeout_ms.saturating_add(150).max(WORKER_SYNC_WAIT_MS);
    match reply_rx.recv_timeout(Duration::from_millis(budget)) {
        Ok(res) => res,
        Err(_) => Err("script worker timed out waiting for result".to_string()),
    }
}

#[cfg(test)]
#[path = "script_worker_tests.rs"]
mod tests;
