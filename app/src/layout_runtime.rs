//! Sandboxed QuickJS evaluator and `<data_dir>/layouts/*.tsx` persistence.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rquickjs::{CaughtError, Context, Runtime};
use serde::{Deserialize, Serialize};

use crate::config::LayoutMode;
use crate::layout_spec::{LayoutContext, LayoutSpec};
use crate::layout_tsx::{ZNIFF_LAYOUT_DTS, transpile_layout_tsx};

/// Maximum QuickJS heap memory allowed for evaluating a layout script (8 MiB).
pub const LAYOUT_MEM_LIMIT_BYTES: usize = 8 * 1024 * 1024;

/// Maximum QuickJS call stack allowed for a layout script (256 KiB).
pub const LAYOUT_STACK_LIMIT_BYTES: usize = 256 * 1024;

/// Maximum wall-clock duration allowed per layout script evaluation (50 ms).
pub const LAYOUT_TIMEOUT_MS: u64 = 50;

/// Maximum file size read from disk for a single layout script (128 KiB).
const MAX_LAYOUT_FILE_BYTES: u64 = crate::layout_tsx::MAX_LAYOUT_SCRIPT_BYTES as u64;

/// Persisted custom TSX/TS layout metadata discovered in `<data_dir>/layouts/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomLayoutEntry {
    pub id: String,
    pub label: String,
    pub file_name: String,
    pub source: String,
}

const SANDBOX_PRELUDE: &str = r#"
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
function __zniff_h(tag, props, ...children) {
  if (tag === __zniff_fragment) {
    const flat = [];
    __zniff_push_child(flat, children);
    return flat;
  }
  const merged = Object.assign({}, props || {});
  if (children.length > 0) merged.children = children;
  if (typeof tag === "function") return tag(merged);
  throw new Error("Unsupported TSX tag `" + String(tag) + "`: expected Layout, Row, Col, or Pane");
}
(() => {
  const deny = (label) => function () {
    throw new TypeError(label + " is disabled inside zniff layout scripts");
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
  for (const proto of [
    Object.prototype, Array.prototype, Function.prototype, String.prototype,
    Number.prototype, Boolean.prototype, RegExp.prototype, Map.prototype, Set.prototype,
  ]) {
    Object.freeze(proto);
  }
})();
"#;

/// Evaluates a user TSX/TS layout script in an isolated, time- and memory-bounded QuickJS sandbox.
pub fn eval_layout_tsx(source: &str, ctx: LayoutContext) -> Result<LayoutSpec, String> {
    let transpiled = transpile_layout_tsx(source)?;
    let ctx_json = serde_json::to_string(&ctx).map_err(|err| err.to_string())?;

    let rt = Runtime::new().map_err(|err| format!("failed to init QuickJS runtime: {err}"))?;
    rt.set_memory_limit(LAYOUT_MEM_LIMIT_BYTES);
    rt.set_max_stack_size(LAYOUT_STACK_LIMIT_BYTES);
    let started = Instant::now();
    let timeout = Duration::from_millis(LAYOUT_TIMEOUT_MS);
    rt.set_interrupt_handler(Some(Box::new(move || started.elapsed() >= timeout)));

    let context =
        Context::full(&rt).map_err(|err| format!("failed to create QuickJS context: {err}"))?;
    let json = context.with(|cx| -> Result<String, String> {
        cx.eval::<(), _>(SANDBOX_PRELUDE)
            .map_err(|err| format_js_error(&cx, err))?;
        cx.eval::<(), _>(transpiled.as_str())
            .map_err(|err| format_js_error(&cx, err))?;
        let invoke = format!(
            "(() => {{\n\
             const __exp = globalThis.__zniff_layout_export;\n\
             if (__exp === undefined || __exp === null) throw new Error('layout script exported nothing');\n\
             const __ctx = Object.freeze({ctx_json});\n\
             const __res = typeof __exp === 'function' ? __exp(__ctx) : __exp;\n\
             if (!__res || typeof __res !== 'object') throw new Error('layout script must return a LayoutSpec object');\n\
             return JSON.stringify(__res);\n\
             }})()"
        );
        cx.eval::<String, _>(invoke.as_str())
            .map_err(|err| format_js_error(&cx, err))
    })?;

    LayoutSpec::parse_json(&json).map_err(|err| err.to_string())
}

fn format_js_error(cx: &rquickjs::Ctx<'_>, err: rquickjs::Error) -> String {
    CaughtError::from_error(cx, err).to_string()
}

/// Returns `<data_dir>/layouts`.
pub fn layouts_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("layouts")
}

/// Validates and normalizes a custom layout script ID (`[a-zA-Z0-9_-]`, `1..=48`).
pub fn sanitize_layout_id(raw: &str) -> Result<String, String> {
    let trimmed = raw
        .trim()
        .strip_suffix(".tsx")
        .or_else(|| raw.trim().strip_suffix(".ts"))
        .unwrap_or_else(|| raw.trim());
    if trimmed.is_empty() || trimmed.len() > 48 {
        return Err("layout ID must be 1–48 characters".to_string());
    }
    if trimmed.contains("..")
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("layout ID may only use letters, digits, `-`, and `_`".to_string());
    }
    let lower = trimmed.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "split" | "top_bottom" | "default" | "zniff-layout"
    ) {
        return Err(format!("`{trimmed}` is a reserved layout ID"));
    }
    Ok(trimmed.to_string())
}

/// Ensures `<data_dir>/layouts/zniff-layout.d.ts` exists and stays up to date.
pub fn ensure_layout_types(data_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = layouts_dir(data_dir);
    fs::create_dir_all(&dir)?;
    let dts_path = dir.join("zniff-layout.d.ts");
    let up_to_date = fs::read_to_string(&dts_path)
        .map(|existing| existing == ZNIFF_LAYOUT_DTS)
        .unwrap_or(false);
    if !up_to_date {
        fs::write(&dts_path, ZNIFF_LAYOUT_DTS)?;
    }
    Ok(dts_path)
}

/// Lists all valid `*.tsx` / `*.ts` custom layout files under `<data_dir>/layouts/`.
pub fn list_custom_layouts(data_dir: &Path) -> Vec<CustomLayoutEntry> {
    let dir = layouts_dir(data_dir);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > MAX_LAYOUT_FILE_BYTES {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if file_name.ends_with(".d.ts")
            || (!file_name.ends_with(".tsx") && !file_name.ends_with(".ts"))
        {
            continue;
        }
        let Ok(id) = sanitize_layout_id(file_name) else {
            continue;
        };
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        let label = extract_layout_label(&source).unwrap_or_else(|| id.clone());
        out.push(CustomLayoutEntry {
            id,
            label,
            file_name: file_name.to_string(),
            source,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Loads one custom layout script by sanitized ID.
pub fn load_custom_layout(data_dir: &Path, raw_id: &str) -> Result<CustomLayoutEntry, String> {
    let id = sanitize_layout_id(raw_id)?;
    let dir = layouts_dir(data_dir);
    for ext in ["tsx", "ts"] {
        let file_name = format!("{id}.{ext}");
        let path = dir.join(&file_name);
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(format!("refusing unsafe symlink or non-file `{file_name}`"));
        }
        if meta.len() > MAX_LAYOUT_FILE_BYTES {
            return Err(format!("layout `{file_name}` exceeds size limit"));
        }
        let source = fs::read_to_string(&path).map_err(|err| err.to_string())?;
        let label = extract_layout_label(&source).unwrap_or_else(|| id.clone());
        return Ok(CustomLayoutEntry {
            id,
            label,
            file_name,
            source,
        });
    }
    Err(format!("custom layout `{id}` not found"))
}

/// Saves a custom TSX layout script atomically as `<data_dir>/layouts/<id>.tsx`.
pub fn save_custom_layout(
    data_dir: &Path,
    raw_id: &str,
    source: &str,
) -> Result<CustomLayoutEntry, String> {
    let id = sanitize_layout_id(raw_id)?;
    if source.len() as u64 > MAX_LAYOUT_FILE_BYTES {
        return Err("layout script exceeds 128 KiB limit".to_string());
    }
    ensure_layout_types(data_dir).map_err(|err| err.to_string())?;
    let dir = layouts_dir(data_dir);
    let file_name = format!("{id}.tsx");
    let final_path = dir.join(&file_name);
    let tmp_path = dir.join(format!(".{id}.tsx.tmp"));
    fs::write(&tmp_path, source).map_err(|err| err.to_string())?;
    if let Err(err) = fs::rename(&tmp_path, &final_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err.to_string());
    }
    // Remove legacy `<id>.ts` if user migrated to `<id>.tsx`.
    let legacy_ts = dir.join(format!("{id}.ts"));
    if legacy_ts.exists() {
        let _ = fs::remove_file(legacy_ts);
    }
    let label = extract_layout_label(source).unwrap_or_else(|| id.clone());
    Ok(CustomLayoutEntry {
        id,
        label,
        file_name,
        source: source.to_string(),
    })
}

/// Deletes `<id>.tsx` and `<id>.ts` under `<data_dir>/layouts/`.
pub fn delete_custom_layout(data_dir: &Path, raw_id: &str) -> Result<(), String> {
    let id = sanitize_layout_id(raw_id)?;
    let dir = layouts_dir(data_dir);
    let mut removed = false;
    for ext in ["tsx", "ts"] {
        let path = dir.join(format!("{id}.{ext}"));
        if path.exists() {
            fs::remove_file(&path).map_err(|err| err.to_string())?;
            removed = true;
        }
    }
    if removed {
        Ok(())
    } else {
        Err(format!("custom layout `{id}` not found"))
    }
}

/// Starter TSX script template used when creating a new custom layout.
pub fn default_custom_layout_template(id: &str) -> String {
    let display = id
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let title = if display.is_empty() {
        "Custom Focus".to_string()
    } else {
        display
    };
    format!(
        r#"import {{ Layout, Row, Col, Pane }} from "zniff/layout";
import type {{ LayoutContext }} from "zniff/layout";

export default ({{ width, target }}: LayoutContext) => (
  <Layout name="{title}" footer="bottom">
    {{width < 920 || target === "web" ? (
      <Col>
        <Pane name="flow_list" size="46%" />
        <Pane name="detail" size="1fr" />
      </Col>
    ) : (
      <Row>
        <Pane name="sidebar" size={{220}} />
        <Col>
          <Pane name="flow_list" size="1fr" />
          <Pane name="summary" size={{156}} />
        </Col>
        <Pane name="detail" size="48%" />
      </Row>
    )}}
  </Layout>
);
"#
    )
}

/// Evaluates the active custom layout (or built-in fallback), returning any load/eval error.
pub fn resolve_active_layout(
    data_dir: &Path,
    active_id: &str,
    fallback_mode: LayoutMode,
    ctx: LayoutContext,
) -> (LayoutSpec, Option<String>) {
    let trimmed = active_id.trim();
    if trimmed.is_empty() {
        return (LayoutSpec::for_layout_mode(fallback_mode), None);
    }
    let entry = match load_custom_layout(data_dir, trimmed) {
        Ok(entry) => entry,
        Err(err) => return (LayoutSpec::for_layout_mode(fallback_mode), Some(err)),
    };
    match eval_layout_tsx(&entry.source, ctx) {
        Ok(spec) => (spec, None),
        Err(err) => (LayoutSpec::for_layout_mode(fallback_mode), Some(err)),
    }
}

fn extract_layout_label(source: &str) -> Option<String> {
    if let Some(tag_pos) = source.find("<Layout") {
        let rest = &source[tag_pos..source.len().min(tag_pos + 240)];
        if let Some(label) = extract_quoted_after_key(rest, "name=") {
            return Some(label);
        }
    }
    extract_quoted_after_key(source, "name:")
}

fn extract_quoted_after_key(haystack: &str, key: &str) -> Option<String> {
    let pos = haystack.find(key)?;
    let rest = haystack[pos + key.len()..].trim_start();
    let quote = rest.as_bytes().first().copied()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let inner = &rest[1..];
    let end = inner.find(quote as char)?;
    let trimmed = inner[..end].trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_spec::{FooterDock, LayoutNode, LayoutSize, LayoutTarget, ViewModule};

    #[test]
    fn evaluates_tsx_responsive_breakpoints_and_conditional_sidebar() {
        let script = r#"
import { Layout, Row, Col, Pane } from "zniff/layout";
import type { LayoutContext } from "zniff/layout";

export default ({ width, target }: LayoutContext) => (
  <Layout name="Responsive Focus" footer={target === "web" ? "hidden" : "top"}>
    {width < 900 ? (
      <Col>
        <Pane name="flow_list" size="45%" />
        <Pane name="detail" size="1fr" />
      </Col>
    ) : (
      <Row>
        <Pane name="detail" size="1fr" />
        <Col>
          <Pane name="flow_list" size="1fr" />
          <Pane name="summary" size={160} />
        </Col>
        {width >= 1100 && <Pane name="sidebar" size={220} />}
      </Row>
    )}
  </Layout>
);
"#;
        let wide_desktop = eval_layout_tsx(
            script,
            LayoutContext {
                width: 1280.0,
                height: 800.0,
                target: LayoutTarget::Desktop,
            },
        )
        .expect("wide desktop eval");
        assert_eq!(wide_desktop.name, "Responsive Focus");
        assert_eq!(wide_desktop.footer, FooterDock::Top);
        assert!(wide_desktop.has_standalone_summary());
        assert!(wide_desktop.root.contains_module(ViewModule::Sidebar));

        let mid_web = eval_layout_tsx(
            script,
            LayoutContext {
                width: 980.0,
                height: 700.0,
                target: LayoutTarget::Web,
            },
        )
        .expect("mid web eval");
        assert_eq!(mid_web.footer, FooterDock::Hidden);
        assert!(!mid_web.root.contains_module(ViewModule::Sidebar));

        let narrow = eval_layout_tsx(
            script,
            LayoutContext {
                width: 720.0,
                height: 640.0,
                target: LayoutTarget::Desktop,
            },
        )
        .expect("narrow eval");
        assert!(matches!(narrow.root, LayoutNode::Col { .. }));
        assert_eq!(
            narrow.root.modules(),
            vec![ViewModule::FlowList, ViewModule::Detail]
        );
        if let LayoutNode::Col { children, .. } = &narrow.root {
            assert_eq!(children[0].size(), LayoutSize::Percent(45.0));
        }
    }

    #[test]
    fn evaluates_pure_ts_helpers() {
        let script = r#"
import { row, col, pane } from "zniff/layout";
export default ({ width }: LayoutContext) => ({
  name: "HelperTS",
  footer: "bottom",
  root: row(
    col(pane("flow_list", "1fr"), pane("summary", 140)),
    pane("detail", width >= 1200 ? "55%" : "45%"),
  ),
});
"#;
        let spec = eval_layout_tsx(
            script,
            LayoutContext {
                width: 1300.0,
                height: 800.0,
                target: LayoutTarget::Desktop,
            },
        )
        .expect("pure ts helper eval");
        assert_eq!(spec.name, "HelperTS");
        assert!(spec.has_standalone_summary());
    }

    #[test]
    fn sandbox_blocks_infinite_loops_eval_function_and_bad_imports() {
        let loop_err = eval_layout_tsx(
            "export default () => { while (true) {} };",
            LayoutContext {
                width: 1200.0,
                height: 760.0,
                target: LayoutTarget::Desktop,
            },
        )
        .expect_err("timeout interrupts infinite loop");
        assert!(!loop_err.is_empty());

        let eval_err = eval_layout_tsx(
            r#"export default () => eval("({ root: { type: 'row', children: [{ type: 'pane', name: 'flow_list' }] } })");"#,
            LayoutContext {
                width: 1200.0,
                height: 760.0,
                target: LayoutTarget::Desktop,
            },
        )
        .expect_err("eval blocked");
        assert!(!eval_err.is_empty());

        let fn_err = eval_layout_tsx(
            r#"export default () => new Function("return {}")();"#,
            LayoutContext {
                width: 1200.0,
                height: 760.0,
                target: LayoutTarget::Desktop,
            },
        )
        .expect_err("Function constructor blocked");
        assert!(fn_err.contains("disabled"));
    }

    #[test]
    fn custom_layout_disk_crud_and_resolve_work_in_isolated_temp_dir() {
        let dir = std::env::temp_dir().join(format!(
            "zniff-layouts-test-{}-{}",
            std::process::id(),
            crate::clock::now().elapsed().unwrap_or_default().as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");

        assert!(sanitize_layout_id("../evil").is_err());
        assert!(sanitize_layout_id("split").is_err());
        assert!(sanitize_layout_id("zniff-layout").is_err());
        assert_eq!(sanitize_layout_id("my-focus.tsx").unwrap(), "my-focus");

        let tpl = default_custom_layout_template("my-focus");
        let saved = save_custom_layout(&dir, "my-focus", &tpl).expect("save custom layout");
        assert_eq!(saved.id, "my-focus");
        assert_eq!(saved.label, "My Focus");
        assert!(layouts_dir(&dir).join("zniff-layout.d.ts").exists());

        let list = list_custom_layouts(&dir);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "my-focus");

        let (resolved, err) = resolve_active_layout(
            &dir,
            "my-focus",
            LayoutMode::Split,
            LayoutContext {
                width: 1280.0,
                height: 800.0,
                target: LayoutTarget::Desktop,
            },
        );
        assert_eq!(err, None);
        assert_eq!(resolved.name, "My Focus");
        assert!(resolved.has_standalone_summary());

        let (fallback, missing_err) = resolve_active_layout(
            &dir,
            "missing-id",
            LayoutMode::TopBottom,
            LayoutContext {
                width: 1280.0,
                height: 800.0,
                target: LayoutTarget::Desktop,
            },
        );
        assert!(fallback.is_builtin_top_bottom());
        assert!(missing_err.is_some());

        delete_custom_layout(&dir, "my-focus").expect("delete");
        assert!(list_custom_layouts(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
