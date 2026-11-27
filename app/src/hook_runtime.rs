//! Sandboxed QuickJS execution runtime for `<data_dir>/hooks/*.tsx` scripts
//! across all 4 lifecycle stages (`onRequest`, `onResponse`, `onPreview`, `onRun`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use rquickjs::{Context, Ctx, Object, Result as JsResult, TypedArray, Value};

pub(crate) use crate::hook_action::{
    HookPreviewInputOwned, HookRequestInputOwned, HookResponseInputOwned, ScriptPreviewAction,
    ScriptRequestAction, ScriptResponseAction, ScriptRunReport,
};
use crate::hook_action::{
    headers_to_js, parse_preview_action, parse_request_action, parse_response_action,
    parse_run_report,
};
use crate::hook_bridge::{HookCallGuard, HookSessionHost};
use crate::hook_spec::{
    DEFAULT_HOOK_TIMEOUT_MS, HookStage, MAX_HOOK_TIMEOUT_MS, MIN_HOOK_TIMEOUT_MS, ScriptHookConfig,
    ScriptHookSummary, ZNIFF_HOOK_DTS, sanitize_hook_id, sanitize_hook_script_file,
    transpile_hook_tsx,
};
use crate::present;
use crate::script_worker::{
    ByteBoundedLru, PACKET_MEM_LIMIT_BYTES, WarmLayoutVm, format_js_err, is_circuit_tripped,
    note_warm_eval, record_hook_eval_outcome, run_vm_task_async, run_vm_task_blocking,
};

const MAX_HOOK_FILE_BYTES: u64 = crate::layout_tsx::MAX_LAYOUT_SCRIPT_BYTES as u64;
const DEFAULT_RUN_TIMEOUT_MS: u64 = 2_000;

/// Returns `<data_dir>/hooks`.
pub fn hooks_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("hooks")
}

/// Ensures `<data_dir>/hooks/zniff-hook.d.ts` exists and stays in sync.
pub fn ensure_hook_types(data_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = hooks_dir(data_dir);
    fs::create_dir_all(&dir)?;
    let dts = dir.join("zniff-hook.d.ts");
    let current = fs::read_to_string(&dts)
        .map(|text| text == ZNIFF_HOOK_DTS)
        .unwrap_or(false);
    if !current {
        fs::write(&dts, ZNIFF_HOOK_DTS)?;
    }
    Ok(dts)
}

/// Lists configured `[[script_hooks]]` merged with discovered `<data_dir>/hooks/*.tsx|*.ts` files.
pub fn list_script_hooks(
    data_dir: &Path,
    configured: &[ScriptHookConfig],
) -> Vec<ScriptHookSummary> {
    let dir = hooks_dir(data_dir);
    let mut disk_files: Vec<String> = fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "zniff-hook.d.ts" || (!name.ends_with(".tsx") && !name.ends_with(".ts")) {
                return None;
            }
            sanitize_hook_script_file(&name).ok()
        })
        .collect();
    disk_files.sort();
    disk_files.dedup();

    let mut out = Vec::new();
    for cfg in configured {
        let has_file = dir.join(&cfg.script).is_file();
        let circuit_key = hook_circuit_key(&dir.join(&cfg.script));
        out.push(ScriptHookSummary {
            id: cfg.id.clone(),
            name: if cfg.name.is_empty() {
                cfg.id.clone()
            } else {
                cfg.name.clone()
            },
            file_name: cfg.script.clone(),
            enabled: cfg.enabled,
            configured: true,
            has_file,
            match_pattern: cfg.match_pattern.clone(),
            stages: cfg.stages.clone(),
            timeout_ms: cfg.timeout_ms,
            circuit_tripped: is_circuit_tripped(&circuit_key),
        });
    }
    for file in disk_files {
        if out
            .iter()
            .any(|item| item.file_name.eq_ignore_ascii_case(&file))
        {
            continue;
        }
        let id = file
            .strip_suffix(".tsx")
            .or_else(|| file.strip_suffix(".ts"))
            .unwrap_or(&file)
            .to_string();
        let path = dir.join(&file);
        out.push(ScriptHookSummary {
            id: id.clone(),
            name: id,
            file_name: file,
            enabled: false,
            configured: false,
            has_file: true,
            match_pattern: String::new(),
            stages: Vec::new(),
            timeout_ms: DEFAULT_RUN_TIMEOUT_MS,
            circuit_tripped: is_circuit_tripped(&hook_circuit_key(&path)),
        });
    }
    out
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedScriptHook {
    pub circuit_key: String,
    pub hook_id: String,
    pub script_path: PathBuf,
    pub vars: BTreeMap<String, String>,
    pub timeout_ms: u64,
}

/// Resolves an `@id` / `@file.tsx` reference against `[[script_hooks]]` and `<data_dir>/hooks/`.
pub(crate) fn resolve_script_hook(
    data_dir: &Path,
    configured: &[ScriptHookConfig],
    id_or_file: &str,
    inline_vars: &BTreeMap<String, String>,
    fallback_timeout_ms: u64,
) -> Result<ResolvedScriptHook, String> {
    let dir = hooks_dir(data_dir);
    let trimmed = id_or_file
        .trim()
        .strip_prefix('@')
        .unwrap_or(id_or_file.trim());
    let clean_stem = sanitize_hook_id(trimmed)?;

    let matched = configured
        .iter()
        .find(|c| c.id.eq_ignore_ascii_case(&clean_stem) || c.script.eq_ignore_ascii_case(trimmed));

    let script_file = if let Some(cfg) = matched {
        cfg.script.clone()
    } else if trimmed.ends_with(".tsx") || trimmed.ends_with(".ts") {
        sanitize_hook_script_file(trimmed)?
    } else {
        let tsx = format!("{clean_stem}.tsx");
        let ts = format!("{clean_stem}.ts");
        if dir.join(&tsx).is_file() || !dir.join(&ts).is_file() {
            tsx
        } else {
            ts
        }
    };

    let mut vars = matched.map(|c| c.vars.clone()).unwrap_or_default();
    for (k, v) in inline_vars {
        vars.insert(k.clone(), v.clone());
    }
    let timeout_ms = if fallback_timeout_ms > 0 {
        fallback_timeout_ms
    } else {
        matched.map_or(DEFAULT_HOOK_TIMEOUT_MS, |c| c.timeout_ms)
    }
    .clamp(MIN_HOOK_TIMEOUT_MS, MAX_HOOK_TIMEOUT_MS);

    let script_path = dir.join(&script_file);
    let circuit_key = hook_circuit_key(&script_path);
    Ok(ResolvedScriptHook {
        circuit_key,
        hook_id: matched.map_or(clean_stem, |c| c.id.clone()),
        script_path,
        vars,
        timeout_ms,
    })
}

pub(crate) fn hook_circuit_key(script_path: &Path) -> String {
    format!("hook:{}", script_path.display())
}

fn load_and_transpile_cached(
    path: &Path,
    cache: &mut ByteBoundedLru<String, Arc<String>>,
) -> Result<Arc<String>, String> {
    let meta = fs::metadata(path)
        .map_err(|err| format!("hook script `{}` unreadable: {err}", path.display()))?;
    if meta.len() > MAX_HOOK_FILE_BYTES {
        return Err(format!(
            "hook script `{}` exceeds size limit ({MAX_HOOK_FILE_BYTES} bytes)",
            path.display()
        ));
    }
    let mtime_nanos = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let key = format!("hook:{}:{}:{mtime_nanos}", path.display(), meta.len());
    if let Some(hit) = cache.get(&key) {
        return Ok(Arc::clone(hit));
    }
    let source = fs::read_to_string(path)
        .map_err(|err| format!("failed to read `{}`: {err}", path.display()))?;
    let js = transpile_hook_tsx(&source)?;
    let arc = Arc::new(js);
    let bytes = key.len().saturating_add(arc.len());
    cache.insert(key, Arc::clone(&arc), bytes);
    Ok(arc)
}

fn exec_hook_on_vm<R>(
    vm_slot: &mut Option<WarmLayoutVm>,
    cache: &mut ByteBoundedLru<String, Arc<String>>,
    host: Option<HookSessionHost>,
    resolved: &ResolvedScriptHook,
    stage: HookStage,
    build_and_extract: impl FnOnce(&Context, &str) -> Result<R, String>,
) -> Result<R, String> {
    if is_circuit_tripped(&resolved.circuit_key) {
        return Err(format!(
            "circuit breaker tripped for script `{}`",
            resolved.hook_id
        ));
    }
    let transpiled = load_and_transpile_cached(&resolved.script_path, cache)?;
    if vm_slot.is_none() {
        *vm_slot = Some(WarmLayoutVm::new()?);
    } else {
        note_warm_eval();
    }
    let vm = vm_slot
        .as_ref()
        .ok_or_else(|| "script VM unavailable".to_string())?;

    let _guard = HookCallGuard::enter(host, stage);
    let (res, timed_out) = vm.with_budget(PACKET_MEM_LIMIT_BYTES, resolved.timeout_ms, |cx| {
        build_and_extract(cx, transpiled.as_str())
    });
    drop(_guard);

    if timed_out || !vm.maintain_memory() {
        *vm_slot = None;
    }
    let tripped = record_hook_eval_outcome(&resolved.circuit_key, timed_out);
    if tripped {
        return Err(format!(
            "circuit breaker tripped for script `{}` (>{} ms)",
            resolved.hook_id, resolved.timeout_ms
        ));
    }
    res
}

fn stage_export_name(stage: HookStage) -> &'static str {
    match stage {
        HookStage::Request => "onRequest",
        HookStage::Response => "onResponse",
        HookStage::Preview => "onPreview",
        HookStage::Run => "onRun",
    }
}

fn call_stage_fn<'js, FBuild, FParse, R>(
    cx: &Ctx<'js>,
    transpiled: &str,
    stage: HookStage,
    vars: &BTreeMap<String, String>,
    build_input: FBuild,
    parse_result: FParse,
) -> Result<R, String>
where
    FBuild: FnOnce(&Ctx<'js>, &Object<'js>) -> JsResult<()>,
    FParse: FnOnce(&Ctx<'js>, Value<'js>) -> Result<R, String>,
{
    let input = Object::new(cx.clone()).map_err(|e| format_js_err(cx, e))?;
    let vars_obj = Object::new(cx.clone()).map_err(|e| format_js_err(cx, e))?;
    for (k, v) in vars {
        vars_obj
            .set(k.as_str(), v.as_str())
            .map_err(|e| format_js_err(cx, e))?;
    }
    input
        .set("vars", vars_obj)
        .map_err(|e| format_js_err(cx, e))?;
    input
        .set("stage", stage.as_str())
        .map_err(|e| format_js_err(cx, e))?;
    build_input(cx, &input).map_err(|e| format_js_err(cx, e))?;

    cx.globals()
        .set("__zniff_hook_input", input)
        .map_err(|e| format_js_err(cx, e))?;

    let fn_name = stage_export_name(stage);
    let wrapped = format!(
        "(() => {{\n\
         \"use strict\";\n\
         globalThis.__zniff_hook_default = undefined;\n\
         globalThis.__zniff_hook_exports = undefined;\n\
         const __in = globalThis.__zniff_hook_input;\n\
         globalThis.__zniff_hook_input = undefined;\n\
         try {{\n\
           {transpiled}\n\
           let __fn = undefined;\n\
           const __exp = globalThis.__zniff_hook_exports;\n\
           const __def = globalThis.__zniff_hook_default;\n\
           if (__exp && typeof __exp[\"{fn_name}\"] === \"function\") __fn = __exp[\"{fn_name}\"];\n\
           else if (typeof {fn_name} === \"function\") __fn = {fn_name};\n\
           else if (__def && typeof __def === \"object\" && typeof __def[\"{fn_name}\"] === \"function\") __fn = __def[\"{fn_name}\"];\n\
           else if (typeof __def === \"function\" && (!__def.name || __def.name === \"{fn_name}\")) __fn = __def;\n\
           if (!__fn) return undefined;\n\
           if (__in && typeof __in === \"object\") {{ if (__in.vars) Object.freeze(__in.vars); Object.freeze(__in); }}\n\
           return __fn(__in);\n\
         }} finally {{\n\
           globalThis.__zniff_hook_default = undefined;\n\
           globalThis.__zniff_hook_exports = undefined;\n\
           globalThis.__zniff_hook_input = undefined;\n\
         }}\n\
         }})()"
    );

    let out_val = cx
        .eval::<Value<'js>, _>(wrapped.as_str())
        .map_err(|err| format_js_err(cx, err))?;
    parse_result(cx, out_val)
}

/// Evaluates `onRequest` on the warm worker without blocking Tokio.
pub(crate) async fn eval_request_hook_async(
    host: HookSessionHost,
    resolved: ResolvedScriptHook,
    input: HookRequestInputOwned,
) -> Result<ScriptRequestAction, String> {
    let wait_ms = resolved.timeout_ms;
    run_vm_task_async(wait_ms, move |vm_slot, cache| {
        exec_hook_on_vm(
            vm_slot,
            cache,
            Some(host),
            &resolved,
            HookStage::Request,
            |cx, js| {
                cx.with(|cx| {
                    call_stage_fn(
                        &cx,
                        js,
                        HookStage::Request,
                        &resolved.vars,
                        |cx, obj| {
                            obj.set("id", input.flow_id as f64)?;
                            obj.set("method", input.method.as_str())?;
                            obj.set("scheme", input.scheme.as_str())?;
                            obj.set("host", input.host.as_str())?;
                            obj.set("url", input.url.as_str())?;
                            obj.set("path", input.path.as_str())?;
                            obj.set("headers", headers_to_js(cx, &input.headers)?)?;
                            obj.set("body", TypedArray::<u8>::new_copy(cx.clone(), &input.body)?)?;
                            Ok(())
                        },
                        parse_request_action,
                    )
                })
            },
        )
    })
    .await
}

/// Evaluates `onResponse` on the warm worker without blocking Tokio.
pub(crate) async fn eval_response_hook_async(
    host: HookSessionHost,
    resolved: ResolvedScriptHook,
    input: HookResponseInputOwned,
) -> Result<ScriptResponseAction, String> {
    let wait_ms = resolved.timeout_ms;
    run_vm_task_async(wait_ms, move |vm_slot, cache| {
        exec_hook_on_vm(
            vm_slot,
            cache,
            Some(host),
            &resolved,
            HookStage::Response,
            |cx, js| {
                cx.with(|cx| {
                    call_stage_fn(
                        &cx,
                        js,
                        HookStage::Response,
                        &resolved.vars,
                        |cx, obj| {
                            obj.set("id", input.flow_id as f64)?;
                            obj.set("method", input.method.as_str())?;
                            obj.set("host", input.host.as_str())?;
                            obj.set("path", input.path.as_str())?;
                            obj.set("url", input.url.as_str())?;
                            obj.set("status", input.status as u32)?;
                            obj.set("headers", headers_to_js(cx, &input.headers)?)?;
                            obj.set("body", TypedArray::<u8>::new_copy(cx.clone(), &input.body)?)?;
                            Ok(())
                        },
                        parse_response_action,
                    )
                })
            },
        )
    })
    .await
}

/// Evaluates `onPreview` (read-only decrypt & optional TSX cards) on the warm worker.
pub(crate) async fn eval_preview_hook_async(
    host: HookSessionHost,
    resolved: ResolvedScriptHook,
    input: HookPreviewInputOwned,
) -> Result<ScriptPreviewAction, String> {
    let wait_ms = resolved.timeout_ms.max(DEFAULT_RUN_TIMEOUT_MS);
    run_vm_task_async(wait_ms, move |vm_slot, cache| {
        exec_hook_on_vm(
            vm_slot,
            cache,
            Some(host),
            &resolved,
            HookStage::Preview,
            |cx, js| {
                cx.with(|cx| {
                    call_stage_fn(
                        &cx,
                        js,
                        HookStage::Preview,
                        &resolved.vars,
                        |cx, obj| {
                            let req_side = Object::new(cx.clone())?;
                            req_side.set("headers", headers_to_js(cx, &input.req_headers)?)?;
                            req_side.set(
                                "body",
                                TypedArray::<u8>::new_copy(cx.clone(), &input.req_body)?,
                            )?;
                            req_side.set(
                                "contentType",
                                present::content_type(&input.req_headers).map(str::to_string),
                            )?;

                            let res_side = Object::new(cx.clone())?;
                            res_side.set("headers", headers_to_js(cx, &input.res_headers)?)?;
                            res_side.set(
                                "body",
                                TypedArray::<u8>::new_copy(cx.clone(), &input.res_body)?,
                            )?;
                            res_side.set(
                                "contentType",
                                present::content_type(&input.res_headers).map(str::to_string),
                            )?;

                            obj.set("id", input.flow_id as f64)?;
                            obj.set(
                                "side",
                                if input.is_response {
                                    "response"
                                } else {
                                    "request"
                                },
                            )?;
                            obj.set("method", input.method.as_str())?;
                            obj.set("host", input.host.as_str())?;
                            obj.set("path", input.path.as_str())?;
                            obj.set("url", input.url.as_str())?;
                            obj.set("status", input.status.map(u16::into) as Option<u32>)?;
                            obj.set("req", req_side)?;
                            obj.set("res", res_side)?;
                            if input.is_response {
                                obj.set("headers", headers_to_js(cx, &input.res_headers)?)?;
                                obj.set(
                                    "body",
                                    TypedArray::<u8>::new_copy(cx.clone(), &input.res_body)?,
                                )?;
                            } else {
                                obj.set("headers", headers_to_js(cx, &input.req_headers)?)?;
                                obj.set(
                                    "body",
                                    TypedArray::<u8>::new_copy(cx.clone(), &input.req_body)?,
                                )?;
                            }
                            Ok(())
                        },
                        |cx, val| parse_preview_action(cx, val, input.is_response),
                    )
                })
            },
        )
    })
    .await
}

/// Evaluates `onRun(ctx)` asynchronously against live captured flows (`zniff.flows.*`).
pub(crate) async fn eval_run_hook_async(
    data_dir: &Path,
    configured: &[ScriptHookConfig],
    host: HookSessionHost,
    id_or_file: &str,
    filter: &str,
    selected_id: Option<u64>,
) -> Result<ScriptRunReport, String> {
    let resolved = resolve_script_hook(data_dir, configured, id_or_file, &BTreeMap::new(), 0)?;
    let wait_ms = resolved.timeout_ms.max(DEFAULT_RUN_TIMEOUT_MS);
    let mut run_resolved = resolved.clone();
    run_resolved.timeout_ms = wait_ms;
    let filter_owned = filter.to_string();
    let started = std::time::Instant::now();
    let hook_id = run_resolved.hook_id.clone();

    run_vm_task_async(wait_ms, move |vm_slot, cache| {
        exec_hook_on_vm(
            vm_slot,
            cache,
            Some(host),
            &run_resolved,
            HookStage::Run,
            |cx, js| {
                let mut report = cx.with(|cx| {
                    call_stage_fn(
                        &cx,
                        js,
                        HookStage::Run,
                        &run_resolved.vars,
                        |_, obj| {
                            obj.set("filter", filter_owned.as_str())?;
                            obj.set("selectedId", selected_id.map(|id| id as f64))?;
                            Ok(())
                        },
                        |cx, val| parse_run_report(cx, val, hook_id, 0),
                    )
                })?;
                report.duration_ms = started.elapsed().as_millis() as u64;
                Ok(report)
            },
        )
    })
    .await
}

/// Synchronous wrapper around `eval_run_hook_async` for desktop UI command/action threads.
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
pub(crate) fn eval_run_hook_blocking(
    data_dir: &Path,
    configured: &[ScriptHookConfig],
    host: HookSessionHost,
    id_or_file: &str,
    filter: &str,
    selected_id: Option<u64>,
) -> Result<ScriptRunReport, String> {
    let resolved = resolve_script_hook(data_dir, configured, id_or_file, &BTreeMap::new(), 0)?;
    let wait_ms = resolved.timeout_ms.max(DEFAULT_RUN_TIMEOUT_MS);
    let mut run_resolved = resolved.clone();
    run_resolved.timeout_ms = wait_ms;
    let filter_owned = filter.to_string();
    let started = std::time::Instant::now();
    let hook_id = run_resolved.hook_id.clone();

    run_vm_task_blocking(wait_ms, move |vm_slot, cache| {
        exec_hook_on_vm(
            vm_slot,
            cache,
            Some(host),
            &run_resolved,
            HookStage::Run,
            |cx, js| {
                let mut report = cx.with(|cx| {
                    call_stage_fn(
                        &cx,
                        js,
                        HookStage::Run,
                        &run_resolved.vars,
                        |_, obj| {
                            obj.set("filter", filter_owned.as_str())?;
                            obj.set("selectedId", selected_id.map(|id| id as f64))?;
                            Ok(())
                        },
                        |cx, val| parse_run_report(cx, val, hook_id, 0),
                    )
                })?;
                report.duration_ms = started.elapsed().as_millis() as u64;
                Ok(report)
            },
        )
    })
}

#[cfg(test)]
#[path = "hook_runtime_tests.rs"]
mod tests;
