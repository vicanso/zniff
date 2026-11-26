//! Sandboxed QuickJS layout evaluator adapter and `<data_dir>/layouts/*.tsx` persistence.
//!
//! Execution delegates to the lazy background worker in [`crate::script_worker`]
//! and pairs with a small `<data_dir>/layouts/.active-cache.json` snapshot so
//! unchanged custom layouts restore on Frame 1 with zero QuickJS startup cost.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::config::LayoutMode;
use crate::layout_spec::{LayoutContext, LayoutSpec, LayoutTarget, layout_context_bucket};
use crate::layout_tsx::ZNIFF_LAYOUT_DTS;
use crate::script_worker::{
    LayoutEvalReport, eval_layout_cold_ephemeral, eval_layout_on_worker_blocking,
    eval_layout_report_blocking, reset_script_circuit,
};

/// Maximum file size read from disk for a single layout script (128 KiB).
const MAX_LAYOUT_FILE_BYTES: u64 = crate::layout_tsx::MAX_LAYOUT_SCRIPT_BYTES as u64;
const ACTIVE_CACHE_FILE: &str = ".active-cache.json";

/// Persisted custom TSX/TS layout metadata discovered in `<data_dir>/layouts/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomLayoutEntry {
    pub id: String,
    pub label: String,
    pub file_name: String,
    pub source: String,
}

/// Resolution result for an active layout, carrying source, error, and circuit-breaker status.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveLayoutResolution {
    pub spec: LayoutSpec,
    pub source: Option<String>,
    pub error: Option<String>,
    pub circuit_tripped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActiveLayoutDiskCache {
    id: String,
    file_len: u64,
    mtime_nanos: u128,
    width_bucket: i32,
    height_bucket: i32,
    target: LayoutTarget,
    spec: LayoutSpec,
}

/// Evaluates an anonymous TSX/TS layout script on the lazy QuickJS worker.
pub fn eval_layout_tsx(source: &str, ctx: LayoutContext) -> Result<LayoutSpec, String> {
    eval_layout_on_worker_blocking(None, source, ctx)
}

/// Evaluates a named layout script on the lazy QuickJS worker with 3-strike circuit protection.
pub fn eval_named_layout_tsx(id: &str, source: &str, ctx: LayoutContext) -> LayoutEvalReport {
    eval_layout_report_blocking(Some(id), source, ctx)
}

/// Evaluates an uncached, single-use ephemeral QuickJS `Runtime` (used only by `perf_bench`).
pub fn eval_layout_tsx_cold_ephemeral(
    source: &str,
    ctx: LayoutContext,
) -> Result<LayoutSpec, String> {
    eval_layout_cold_ephemeral(source, ctx)
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

/// Loads one custom layout script by sanitized ID without scanning the whole directory.
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

fn file_mtime_nanos(meta: &fs::Metadata) -> u128 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn active_layout_meta(data_dir: &Path, clean_id: &str) -> Option<(u64, u128)> {
    let dir = layouts_dir(data_dir);
    for ext in ["tsx", "ts"] {
        let path = dir.join(format!("{clean_id}.{ext}"));
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.file_type().is_symlink() && meta.is_file() && meta.len() <= MAX_LAYOUT_FILE_BYTES {
            return Some((meta.len(), file_mtime_nanos(&meta)));
        }
        return None;
    }
    None
}

/// Loads the cached `LayoutSpec` snapshot when source metadata and context bucket match.
pub fn try_load_cached_layout_spec(
    data_dir: &Path,
    raw_id: &str,
    ctx: LayoutContext,
) -> Option<LayoutSpec> {
    let id = sanitize_layout_id(raw_id).ok()?;
    let (file_len, mtime_nanos) = active_layout_meta(data_dir, &id)?;
    let cache_path = layouts_dir(data_dir).join(ACTIVE_CACHE_FILE);
    let raw = fs::read_to_string(cache_path).ok()?;
    let cache: ActiveLayoutDiskCache = serde_json::from_str(&raw).ok()?;
    let (wb, hb) = layout_context_bucket(ctx);
    if cache.id == id
        && cache.file_len == file_len
        && cache.mtime_nanos == mtime_nanos
        && cache.width_bucket == wb
        && cache.height_bucket == hb
        && cache.target == ctx.target
        && cache.spec.validate().is_ok()
    {
        Some(cache.spec)
    } else {
        None
    }
}

/// Persists the evaluated `LayoutSpec` snapshot for zero-VM Frame 1 cold boot.
pub fn store_cached_layout_spec(
    data_dir: &Path,
    raw_id: &str,
    ctx: LayoutContext,
    spec: &LayoutSpec,
) {
    let Ok(id) = sanitize_layout_id(raw_id) else {
        return;
    };
    let Some((file_len, mtime_nanos)) = active_layout_meta(data_dir, &id) else {
        return;
    };
    let (width_bucket, height_bucket) = layout_context_bucket(ctx);
    let payload = ActiveLayoutDiskCache {
        id,
        file_len,
        mtime_nanos,
        width_bucket,
        height_bucket,
        target: ctx.target,
        spec: spec.clone(),
    };
    if let Ok(json) = serde_json::to_string(&payload) {
        let dir = layouts_dir(data_dir);
        let tmp = dir.join(".active-cache.json.tmp");
        let dst = dir.join(ACTIVE_CACHE_FILE);
        if fs::write(&tmp, json).is_ok() && fs::rename(&tmp, &dst).is_err() {
            let _ = fs::remove_file(&tmp);
        }
    }
}

fn clear_cached_layout_spec(data_dir: &Path, id: Option<&str>) {
    let cache_path = layouts_dir(data_dir).join(ACTIVE_CACHE_FILE);
    if let Some(target_id) = id
        && let Ok(raw) = fs::read_to_string(&cache_path)
        && let Ok(cache) = serde_json::from_str::<ActiveLayoutDiskCache>(&raw)
        && cache.id != target_id
    {
        return;
    }
    let _ = fs::remove_file(cache_path);
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
    let legacy_ts = dir.join(format!("{id}.ts"));
    if legacy_ts.exists() {
        let _ = fs::remove_file(legacy_ts);
    }
    reset_script_circuit(&id);
    clear_cached_layout_spec(data_dir, Some(&id));
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
    reset_script_circuit(&id);
    clear_cached_layout_spec(data_dir, Some(&id));
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

/// Resolves the active custom layout (or built-in fallback), using `.active-cache.json`
/// when available to avoid starting QuickJS on cold boot.
pub fn resolve_active_layout_with_cache(
    data_dir: &Path,
    active_id: &str,
    fallback_mode: LayoutMode,
    ctx: LayoutContext,
) -> ActiveLayoutResolution {
    let trimmed = active_id.trim();
    if trimmed.is_empty() {
        return ActiveLayoutResolution {
            spec: LayoutSpec::for_layout_mode(fallback_mode),
            source: None,
            error: None,
            circuit_tripped: false,
        };
    }
    let entry = match load_custom_layout(data_dir, trimmed) {
        Ok(entry) => entry,
        Err(err) => {
            return ActiveLayoutResolution {
                spec: LayoutSpec::for_layout_mode(fallback_mode),
                source: None,
                error: Some(err),
                circuit_tripped: false,
            };
        }
    };
    if let Some(cached) = try_load_cached_layout_spec(data_dir, &entry.id, ctx) {
        return ActiveLayoutResolution {
            spec: cached,
            source: Some(entry.source),
            error: None,
            circuit_tripped: false,
        };
    }
    let report = eval_named_layout_tsx(&entry.id, &entry.source, ctx);
    match report.result {
        Ok(spec) => {
            store_cached_layout_spec(data_dir, &entry.id, ctx, &spec);
            ActiveLayoutResolution {
                spec,
                source: Some(entry.source),
                error: None,
                circuit_tripped: false,
            }
        }
        Err(err) => ActiveLayoutResolution {
            spec: LayoutSpec::for_layout_mode(fallback_mode),
            source: Some(entry.source),
            error: Some(err),
            circuit_tripped: report.circuit_tripped,
        },
    }
}

/// Evaluates the active custom layout (or built-in fallback), returning any load/eval error.
#[cfg(test)]
pub fn resolve_active_layout(
    data_dir: &Path,
    active_id: &str,
    fallback_mode: LayoutMode,
    ctx: LayoutContext,
) -> (LayoutSpec, Option<String>) {
    let resolved = resolve_active_layout_with_cache(data_dir, active_id, fallback_mode, ctx);
    (resolved.spec, resolved.error)
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
    use crate::layout_spec::{FooterDock, LayoutNode, LayoutSize, ViewModule};

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

        let ctx = LayoutContext {
            width: 1280.0,
            height: 800.0,
            target: LayoutTarget::Desktop,
        };
        let (resolved, err) = resolve_active_layout(&dir, "my-focus", LayoutMode::Split, ctx);
        assert_eq!(err, None);
        assert_eq!(resolved.name, "My Focus");
        assert!(resolved.has_standalone_summary());
        assert!(try_load_cached_layout_spec(&dir, "my-focus", ctx).is_some());

        // Built-in empty ID returns immediately without writing layouts/ in a clean dir.
        let clean_dir = dir.join("clean-sub");
        fs::create_dir_all(&clean_dir).unwrap();
        let (builtin, builtin_err) = resolve_active_layout(&clean_dir, "", LayoutMode::Split, ctx);
        assert!(builtin.is_builtin_split());
        assert_eq!(builtin_err, None);
        assert!(!layouts_dir(&clean_dir).exists());

        let (fallback, missing_err) =
            resolve_active_layout(&dir, "missing-id", LayoutMode::TopBottom, ctx);
        assert!(fallback.is_builtin_top_bottom());
        assert!(missing_err.is_some());

        delete_custom_layout(&dir, "my-focus").expect("delete");
        assert!(list_custom_layouts(&dir).is_empty());
        assert!(try_load_cached_layout_spec(&dir, "my-focus", ctx).is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
