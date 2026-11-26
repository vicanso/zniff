//! Reactive state, lazy persistence loading, debouncing, and circuit-breaker UI for custom TSX layouts.

#[allow(unused_imports)]
use super::*;

use std::collections::BTreeMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

use crate::layout_spec::{FooterDock, LayoutContext, LayoutSpec, LayoutTarget, ViewModule};

const LAYOUT_RESIZE_DEBOUNCE_MS: u64 = 120;
const LAYOUT_DRAFT_DEBOUNCE_MS: u64 = 180;

/// Metadata and script source for one discovered custom `.tsx`/`.ts` layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CustomLayoutMeta {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) file_name: String,
    pub(crate) source: String,
}

/// Shared UI signals coordinating built-in vs. custom TSX layouts across toolbar, shell, and Settings.
#[derive(Clone, Copy)]
pub(crate) struct LayoutUiState {
    pub(crate) active_id: Signal<String>,
    pub(crate) active_spec: Signal<LayoutSpec>,
    pub(crate) active_source: Signal<Option<(String, String)>>,
    pub(crate) active_error: Signal<Option<String>>,
    pub(crate) circuit_tripped: Signal<bool>,
    pub(crate) error_dismissed: Signal<bool>,
    pub(crate) custom_layouts: Signal<Vec<CustomLayoutMeta>>,
    pub(crate) all_pane_sizes: Signal<BTreeMap<String, BTreeMap<String, u32>>>,
    pub(crate) active_pane_overrides: Signal<BTreeMap<String, u32>>,
    pub(crate) reload_rev: Signal<u64>,
    pub(crate) editor_open: Signal<bool>,
    pub(crate) draft_id: Signal<String>,
    pub(crate) draft_source: Signal<String>,
    pub(crate) preview_status: Signal<Result<String, String>>,
    pub(crate) busy: Signal<bool>,
    pub(crate) delete_confirm: Signal<Option<(String, SystemTime)>>,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) default_template: Signal<String>,
    pub(crate) layouts_loaded: Signal<bool>,
    pub(crate) resize_pending: Stored<Option<(LayoutContext, SystemTime)>>,
    pub(crate) draft_pending_at: Stored<Option<SystemTime>>,
    pub(crate) active_eval_seq: Stored<u64>,
    pub(crate) draft_eval_seq: Stored<u64>,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native_store::layout_store_dir;

#[cfg(not(target_arch = "wasm32"))]
mod native_store {
    use super::*;

    #[cfg(test)]
    thread_local! {
        static TEST_LAYOUT_DATA_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    #[cfg(test)]
    pub(crate) fn set_test_layout_data_dir(dir: Option<PathBuf>) {
        TEST_LAYOUT_DATA_DIR.with(|slot| *slot.borrow_mut() = dir);
    }

    pub(crate) fn layout_store_dir() -> PathBuf {
        #[cfg(test)]
        {
            TEST_LAYOUT_DATA_DIR.with(|slot| {
                slot.borrow().clone().unwrap_or_else(|| {
                    std::env::temp_dir().join(format!("zniff-test-layouts-{}", std::process::id()))
                })
            })
        }
        #[cfg(not(test))]
        {
            ca::data_dir()
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), test))]
pub(crate) use native_store::set_test_layout_data_dir;

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn fallback_layout_template() -> String {
    r#"import { Layout, Row, Col, Pane } from "zniff/layout";
import type { LayoutContext } from "zniff/layout";

export default ({ width, target }: LayoutContext) => (
  <Layout name="Custom Focus" footer="bottom">
    {width < 920 || target === "web" ? (
      <Col>
        <Pane name="flow_list" size="46%" />
        <Pane name="detail" size="1fr" />
      </Col>
    ) : (
      <Row>
        <Pane name="sidebar" size={220} />
        <Col>
          <Pane name="flow_list" size="1fr" />
          <Pane name="summary" size={156} />
        </Col>
        <Pane name="detail" size="48%" />
      </Row>
    )}
  </Layout>
);
"#
    .to_string()
}

pub(crate) fn current_layout_context() -> LayoutContext {
    let bounds = window();
    LayoutContext {
        width: bounds.width.0.max(320.0),
        height: bounds.height.0.max(240.0),
        target: if cfg!(target_arch = "wasm32") {
            LayoutTarget::Web
        } else {
            LayoutTarget::Desktop
        },
    }
}

pub(crate) fn validate_layout_id_syntax(raw: &str) -> Result<String, String> {
    let trimmed = raw
        .trim()
        .strip_suffix(".tsx")
        .or_else(|| raw.trim().strip_suffix(".ts"))
        .unwrap_or_else(|| raw.trim());
    if trimmed.is_empty() || trimmed.len() > 48 {
        return Err(t_now(
            "布局 ID 需为 1–48 个字符",
            "Layout ID must be 1–48 characters",
        ));
    }
    if trimmed.contains("..")
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(t_now(
            "布局 ID 仅支持字母、数字、`-` 与 `_`",
            "Layout ID may only use letters, digits, `-`, and `_`",
        ));
    }
    let clean = config::sanitize_layout_id_pref(trimmed);
    if clean.is_empty() {
        return Err(t_now(
            "不能使用保留布局名称（split / top_bottom / default）",
            "Cannot use a reserved layout ID (split / top_bottom / default)",
        ));
    }
    Ok(clean)
}

pub(crate) fn t_now(zh: &'static str, en: &'static str) -> String {
    t(ui_lang(ui_langs()), zh, en).to_string()
}

fn footer_label(dock: FooterDock) -> (&'static str, &'static str) {
    match dock {
        FooterDock::Bottom => ("底栏：底部", "footer: bottom"),
        FooterDock::Top => ("底栏：顶部", "footer: top"),
        FooterDock::Hidden => ("底栏：隐藏", "footer: hidden"),
    }
}

pub(crate) fn format_spec_summary(spec: &LayoutSpec) -> String {
    let lang = ui_lang(ui_langs());
    let mut modules = Vec::new();
    for m in [
        ViewModule::Sidebar,
        ViewModule::FlowList,
        ViewModule::Detail,
        ViewModule::Summary,
    ] {
        if spec.root.contains_module(m) {
            modules.push(m.id());
        }
    }
    let (f_zh, f_en) = footer_label(spec.footer);
    let title = if spec.name.trim().is_empty() {
        "Custom"
    } else {
        spec.name.as_str()
    };
    format!(
        "✓ {title} · {} · {}",
        t(lang, f_zh, f_en),
        modules.join(", ")
    )
}

#[cfg(test)]
pub(crate) fn init_layout_ui_state(
    active_layout: &str,
    pane_sizes: &BTreeMap<String, BTreeMap<String, u32>>,
    layout_mode: config::LayoutMode,
) -> LayoutUiState {
    init_layout_ui_state_cached(active_layout, pane_sizes, layout_mode, None)
}

/// Initializes layout UI state lazily:
/// - Built-in layout (`active_layout == ""`) performs zero directory scans and zero QuickJS work.
/// - Custom layout checks `runtime_cache` and `.active-cache.json` first for zero-VM Frame 1 mount.
pub(crate) fn init_layout_ui_state_cached(
    active_layout: &str,
    pane_sizes: &BTreeMap<String, BTreeMap<String, u32>>,
    layout_mode: config::LayoutMode,
    runtime_cache: Option<&(String, LayoutSpec)>,
) -> LayoutUiState {
    let clean_active = config::sanitize_layout_id_pref(active_layout);
    let clean_sizes = config::sanitize_layout_pane_sizes(pane_sizes.clone());
    let active_overrides = if clean_active.is_empty() {
        BTreeMap::new()
    } else {
        clean_sizes.get(&clean_active).cloned().unwrap_or_default()
    };

    #[cfg(not(target_arch = "wasm32"))]
    let (template, initial_spec, initial_source, initial_err, initial_tripped) = {
        let template = crate::layout_runtime::default_custom_layout_template("focus");
        if clean_active.is_empty() {
            (
                template,
                LayoutSpec::for_layout_mode(layout_mode),
                None,
                None,
                false,
            )
        } else if let Some((cached_id, cached_spec)) = runtime_cache
            && cached_id == &clean_active
            && cached_spec.validate().is_ok()
        {
            (template, cached_spec.clone(), None, None, false)
        } else {
            let dir = native_store::layout_store_dir();
            let resolved = crate::layout_runtime::resolve_active_layout_with_cache(
                &dir,
                &clean_active,
                layout_mode,
                LayoutContext {
                    width: config::WINDOW_WIDTH_DEFAULT as f32,
                    height: config::WINDOW_HEIGHT_DEFAULT as f32,
                    target: LayoutTarget::Desktop,
                },
            );
            let source = resolved.source.map(|src| (clean_active.clone(), src));
            (
                template,
                resolved.spec,
                source,
                resolved.error,
                resolved.circuit_tripped,
            )
        }
    };
    #[cfg(target_arch = "wasm32")]
    let (template, initial_spec, initial_source, initial_err, initial_tripped) = {
        let cached = runtime_cache
            .filter(|(id, spec)| {
                !clean_active.is_empty() && id == &clean_active && spec.validate().is_ok()
            })
            .map(|(_, spec)| spec.clone());
        (
            fallback_layout_template(),
            cached.unwrap_or_else(|| LayoutSpec::for_layout_mode(layout_mode)),
            None,
            None,
            false,
        )
    };

    let state = LayoutUiState {
        active_id: signal(clean_active.clone()),
        active_spec: signal(initial_spec),
        active_source: signal(initial_source),
        active_error: signal(initial_err),
        circuit_tripped: signal(initial_tripped),
        error_dismissed: signal(false),
        custom_layouts: signal(Vec::new()),
        all_pane_sizes: signal(clean_sizes),
        active_pane_overrides: signal(active_overrides),
        reload_rev: signal(0),
        editor_open: signal(false),
        draft_id: signal("focus".to_string()),
        draft_source: signal(template.clone()),
        preview_status: signal(Ok(String::new())),
        busy: signal(false),
        delete_confirm: signal(None),
        default_template: signal(template),
        layouts_loaded: signal(false),
        resize_pending: stored(None),
        draft_pending_at: stored(None),
        active_eval_seq: stored(0),
        draft_eval_seq: stored(0),
    };

    #[cfg(target_arch = "wasm32")]
    if !clean_active.is_empty() {
        reload_custom_layouts(state);
    }

    state
}

/// Loads `<data_dir>/layouts/` (or `/api/layouts` on wasm32) once on demand when the user opens
/// the layout switcher popover or the Settings custom-layout panel.
pub(crate) fn ensure_custom_layouts_loaded(state: LayoutUiState) {
    if state.layouts_loaded.get_untracked() || state.busy.get_untracked() {
        return;
    }
    reload_custom_layouts(state);
}

fn sync_overrides_for_active(state: LayoutUiState, id: &str) {
    let next = if id.is_empty() {
        BTreeMap::new()
    } else {
        state
            .all_pane_sizes
            .with_untracked(|all| all.get(id).cloned().unwrap_or_default())
    };
    state.active_pane_overrides.set(next);
}

pub(crate) fn select_builtin_layout(
    state: LayoutUiState,
    layout_mode_sig: Signal<config::LayoutMode>,
    mode: config::LayoutMode,
    controls: &Controls,
) {
    layout_mode_sig.set(mode);
    state.resize_pending.replace(None);
    state
        .active_eval_seq
        .replace(state.active_eval_seq.get().wrapping_add(1));
    state.active_id.set(String::new());
    state.active_source.set(None);
    state.active_error.set(None);
    state.circuit_tripped.set(false);
    state.error_dismissed.set(false);
    state.delete_confirm.set(None);
    sync_overrides_for_active(state, "");
    let spec = LayoutSpec::for_layout_mode(mode);
    if state.active_spec.with_untracked(|cur| *cur != spec) {
        state.active_spec.set(spec);
    }
    save_config_or_notify(controls, PrefName::List, |cfg| {
        cfg.layout_mode = mode;
        cfg.active_layout.clear();
    });
}

pub(crate) fn select_custom_layout(state: LayoutUiState, id: &str, controls: &Controls) {
    let clean = config::sanitize_layout_id_pref(id);
    if clean.is_empty() {
        return;
    }
    state.resize_pending.replace(None);
    state.active_id.set(clean.clone());
    state.error_dismissed.set(false);
    state.delete_confirm.set(None);
    sync_overrides_for_active(state, &clean);
    save_config_or_notify(controls, PrefName::List, |cfg| {
        cfg.active_layout = clean;
    });
}

pub(crate) fn save_pane_size_override(
    state: LayoutUiState,
    path: String,
    px_size: u32,
    controls: &Controls,
) {
    let active_id = state.active_id.get_untracked();
    if active_id.is_empty() {
        return;
    }
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed.len() > 64 {
        return;
    }
    let clamped = config::clamp_layout_pane_size(px_size);
    let mut updated_map = state.active_pane_overrides.get_untracked();
    updated_map.insert(trimmed.to_string(), clamped);
    state.active_pane_overrides.set(updated_map.clone());

    let mut all = state.all_pane_sizes.get_untracked();
    all.insert(active_id, updated_map);
    state.all_pane_sizes.set(all.clone());

    save_config_or_notify(controls, PrefName::List, |cfg| {
        cfg.layout_pane_sizes = all;
    });
}

pub(crate) fn reset_pane_size_overrides(state: LayoutUiState, controls: &Controls) {
    let active_id = state.active_id.get_untracked();
    if active_id.is_empty() {
        return;
    }
    state.active_pane_overrides.set(BTreeMap::new());
    let mut all = state.all_pane_sizes.get_untracked();
    all.remove(&active_id);
    state.all_pane_sizes.set(all.clone());
    save_config_or_notify(controls, PrefName::List, |cfg| {
        cfg.layout_pane_sizes = all;
    });
}

fn next_available_layout_id(entries: &[CustomLayoutMeta]) -> String {
    for candidate in ["focus", "focus_2", "focus_3", "custom"] {
        if !entries.iter().any(|e| e.id == candidate) {
            return candidate.to_string();
        }
    }
    format!("custom_{}", entries.len() + 1)
}

pub(crate) fn open_custom_layout_in_editor(state: LayoutUiState, target_id: Option<&str>) {
    ensure_custom_layouts_loaded(state);
    state.delete_confirm.set(None);
    state.editor_open.set(true);
    let list = state.custom_layouts.get_untracked();
    if let Some(id) = target_id
        && let Some(entry) = list.iter().find(|item| item.id == id)
    {
        state.draft_id.set(entry.id.clone());
        state.draft_source.set(entry.source.clone());
        validate_draft_layout(state, current_layout_context());
        return;
    }
    let fresh_id = next_available_layout_id(&list);
    #[cfg(not(target_arch = "wasm32"))]
    let tmpl = crate::layout_runtime::default_custom_layout_template(&fresh_id);
    #[cfg(target_arch = "wasm32")]
    let tmpl = state.default_template.get_untracked();
    state.draft_id.set(fresh_id);
    state.draft_source.set(tmpl);
    validate_draft_layout(state, current_layout_context());
}

pub(crate) fn reload_custom_layouts(state: LayoutUiState) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = native_store::layout_store_dir();
        let _ = crate::layout_runtime::ensure_layout_types(&dir);
        let list: Vec<CustomLayoutMeta> = crate::layout_runtime::list_custom_layouts(&dir)
            .into_iter()
            .map(|e| CustomLayoutMeta {
                id: e.id,
                label: e.label,
                file_name: e.file_name,
                source: e.source,
            })
            .collect();
        let active_id = state.active_id.get_untracked();
        if let Some(active) = list.iter().find(|e| e.id == active_id) {
            state
                .active_source
                .set(Some((active.id.clone(), active.source.clone())));
        }
        state.custom_layouts.set(list);
        state.layouts_loaded.set(true);
        state.reload_rev.update(|v| *v += 1);
    }
    #[cfg(target_arch = "wasm32")]
    {
        state.busy.set(true);
        spawn_detached(async move {
            match proxy::fetch_web_layouts().await {
                Ok(payload) => {
                    let entries: Vec<CustomLayoutMeta> = payload
                        .layouts
                        .into_iter()
                        .map(|e| CustomLayoutMeta {
                            id: e.id,
                            label: e.label,
                            file_name: e.file_name,
                            source: e.source,
                        })
                        .collect();
                    let active_id = state.active_id.get_untracked();
                    if let Some(active) = entries.iter().find(|e| e.id == active_id) {
                        state
                            .active_source
                            .set(Some((active.id.clone(), active.source.clone())));
                    }
                    state.custom_layouts.set(entries);
                    if !payload.default_template.trim().is_empty() {
                        state.default_template.set(payload.default_template);
                    }
                    state.layouts_loaded.set(true);
                    state.reload_rev.update(|v| *v += 1);
                }
                Err(err) => {
                    state.preview_status.set(Err(err.to_string()));
                }
            }
            state.busy.set(false);
        });
    }
}

/// Schedules a coalesced 120 ms debounced layout re-evaluation during window resize.
pub(crate) fn schedule_debounced_layout_resize(state: LayoutUiState, ctx: LayoutContext) {
    if state.active_id.with_untracked(String::is_empty) {
        state.resize_pending.replace(None);
        return;
    }
    state.resize_pending.replace(Some((ctx, clock::now())));
}

/// Schedules a coalesced 180 ms debounced draft script validation on editor keystrokes.
pub(crate) fn schedule_debounced_draft_validation(state: LayoutUiState) {
    state.draft_pending_at.replace(Some(clock::now()));
}

#[cfg(not(target_arch = "wasm32"))]
fn cached_active_source_for(state: LayoutUiState, active_id: &str) -> Result<String, String> {
    if let Some((cached_id, src)) = state.active_source.get_untracked()
        && cached_id == active_id
    {
        return Ok(src);
    }
    if let Some(src) = state.custom_layouts.with_untracked(|list| {
        list.iter()
            .find(|e| e.id == active_id)
            .map(|e| e.source.clone())
    }) {
        state
            .active_source
            .set(Some((active_id.to_string(), src.clone())));
        return Ok(src);
    }
    let dir = native_store::layout_store_dir();
    let entry = crate::layout_runtime::load_custom_layout(&dir, active_id)?;
    state
        .active_source
        .set(Some((entry.id.clone(), entry.source.clone())));
    Ok(entry.source)
}

#[cfg(not(target_arch = "wasm32"))]
fn apply_active_layout_outcome(
    state: LayoutUiState,
    layout_mode: config::LayoutMode,
    result: Result<LayoutSpec, String>,
    circuit_tripped: bool,
) {
    match result {
        Ok(spec) => {
            if state.active_spec.with_untracked(|cur| *cur != spec) {
                state.active_spec.set(spec);
            }
            if state.active_error.with_untracked(Option::is_some) {
                state.active_error.set(None);
                state.error_dismissed.set(false);
            }
            if state.circuit_tripped.get_untracked() {
                state.circuit_tripped.set(false);
            }
        }
        Err(err) => {
            let fallback = LayoutSpec::for_layout_mode(layout_mode);
            if state.active_spec.with_untracked(|cur| *cur != fallback) {
                state.active_spec.set(fallback);
            }
            let msg = Some(err);
            if state.active_error.with_untracked(|cur| *cur != msg) {
                state.active_error.set(msg);
                state.error_dismissed.set(false);
            }
            if state.circuit_tripped.get_untracked() != circuit_tripped {
                state.circuit_tripped.set(circuit_tripped);
            }
        }
    }
}

/// Advances debounced layout/editor timers and drains async worker results on the 16 ms UI tick.
pub(crate) fn tick_layout_async_tasks(
    state: LayoutUiState,
    layout_mode: config::LayoutMode,
    now: SystemTime,
) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(done) = crate::script_worker::take_active_layout_result()
            && done.seq == state.active_eval_seq.get()
            && done.script_id == state.active_id.get_untracked()
        {
            if let Ok(ref spec) = done.report.result {
                let dir = native_store::layout_store_dir();
                crate::layout_runtime::store_cached_layout_spec(
                    &dir,
                    &done.script_id,
                    done.ctx,
                    spec,
                );
            }
            apply_active_layout_outcome(
                state,
                layout_mode,
                done.report.result,
                done.report.circuit_tripped,
            );
        }

        if let Some(done) = crate::script_worker::take_draft_layout_result()
            && done.seq == state.draft_eval_seq.get()
        {
            match done.result {
                Ok(spec) => state.preview_status.set(Ok(format_spec_summary(&spec))),
                Err(err) => state.preview_status.set(Err(err)),
            }
        }

        if let Some((ctx, queued_at)) = state.resize_pending.get()
            && now.duration_since(queued_at).unwrap_or_default()
                >= Duration::from_millis(LAYOUT_RESIZE_DEBOUNCE_MS)
        {
            state.resize_pending.replace(None);
            let active_id = state.active_id.get_untracked();
            if !active_id.is_empty() {
                match cached_active_source_for(state, &active_id) {
                    Ok(source) => {
                        let seq = state.active_eval_seq.get().wrapping_add(1);
                        state.active_eval_seq.replace(seq);
                        crate::script_worker::request_active_layout_eval(
                            seq, active_id, source, ctx,
                        );
                    }
                    Err(err) => apply_active_layout_outcome(state, layout_mode, Err(err), false),
                }
            }
        }

        if let Some(queued_at) = state.draft_pending_at.get()
            && now.duration_since(queued_at).unwrap_or_default()
                >= Duration::from_millis(LAYOUT_DRAFT_DEBOUNCE_MS)
        {
            state.draft_pending_at.replace(None);
            let raw_id = state.draft_id.get_untracked();
            if let Err(err) = validate_layout_id_syntax(&raw_id) {
                state.preview_status.set(Err(err));
            } else {
                let seq = state.draft_eval_seq.get().wrapping_add(1);
                state.draft_eval_seq.replace(seq);
                crate::script_worker::request_draft_layout_eval(
                    seq,
                    state.draft_source.get_untracked(),
                    current_layout_context(),
                );
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        if let Some((ctx, queued_at)) = state.resize_pending.get()
            && now.duration_since(queued_at).unwrap_or_default()
                >= Duration::from_millis(LAYOUT_RESIZE_DEBOUNCE_MS)
        {
            state.resize_pending.replace(None);
            evaluate_active_layout(state, layout_mode, ctx);
        }
        if let Some(queued_at) = state.draft_pending_at.get()
            && now.duration_since(queued_at).unwrap_or_default()
                >= Duration::from_millis(LAYOUT_DRAFT_DEBOUNCE_MS)
        {
            state.draft_pending_at.replace(None);
            validate_draft_layout(state, current_layout_context());
        }
    }
}

pub(crate) fn evaluate_active_layout(
    state: LayoutUiState,
    layout_mode: config::LayoutMode,
    ctx: LayoutContext,
) {
    state.resize_pending.replace(None);
    let seq = state.active_eval_seq.get().wrapping_add(1);
    state.active_eval_seq.replace(seq);

    let active_id = state.active_id.get_untracked();
    if active_id.is_empty() {
        let spec = LayoutSpec::for_layout_mode(layout_mode);
        if state.active_spec.with_untracked(|cur| *cur != spec) {
            state.active_spec.set(spec);
        }
        if state.active_source.with_untracked(Option::is_some) {
            state.active_source.set(None);
        }
        if state.active_error.with_untracked(Option::is_some) {
            state.active_error.set(None);
            state.error_dismissed.set(false);
        }
        if state.circuit_tripped.get_untracked() {
            state.circuit_tripped.set(false);
        }
        return;
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = native_store::layout_store_dir();
        let resolved = crate::layout_runtime::resolve_active_layout_with_cache(
            &dir,
            &active_id,
            layout_mode,
            ctx,
        );
        if let Some(src) = resolved.source {
            state.active_source.set(Some((active_id, src)));
        }
        match resolved.error {
            None => apply_active_layout_outcome(state, layout_mode, Ok(resolved.spec), false),
            Some(err) => {
                apply_active_layout_outcome(state, layout_mode, Err(err), resolved.circuit_tripped)
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        if !state.layouts_loaded.get_untracked() {
            return;
        }
        let maybe_source = state.custom_layouts.with_untracked(|list| {
            list.iter()
                .find(|e| e.id == active_id)
                .map(|e| e.source.clone())
        });
        let Some(source) = maybe_source else {
            let spec = LayoutSpec::for_layout_mode(layout_mode);
            if state.active_spec.with_untracked(|cur| *cur != spec) {
                state.active_spec.set(spec);
            }
            let err = Some(format!("custom layout `{active_id}` not found"));
            if state.active_error.with_untracked(|cur| *cur != err) {
                state.active_error.set(err);
                state.error_dismissed.set(false);
            }
            return;
        };
        let expected_id = active_id;
        spawn_detached(async move {
            let outcome = proxy::eval_web_layout(&source, ctx).await;
            if state.active_eval_seq.get() != seq || state.active_id.get_untracked() != expected_id
            {
                return;
            }
            match outcome {
                Ok(spec) => {
                    if state.active_spec.with_untracked(|cur| *cur != spec) {
                        state.active_spec.set(spec);
                    }
                    state.active_error.set(None);
                    state.circuit_tripped.set(false);
                }
                Err(err) => {
                    let fallback = LayoutSpec::for_layout_mode(layout_mode);
                    if state.active_spec.with_untracked(|cur| *cur != fallback) {
                        state.active_spec.set(fallback);
                    }
                    let msg_text = err.to_string();
                    state
                        .circuit_tripped
                        .set(msg_text.contains("circuit breaker"));
                    let msg = Some(msg_text);
                    if state.active_error.with_untracked(|cur| *cur != msg) {
                        state.active_error.set(msg);
                        state.error_dismissed.set(false);
                    }
                }
            }
        });
    }
}

/// Dismissible workspace banner shown when the active custom TSX layout fails or trips the circuit breaker.
pub(crate) fn layout_error_banner(
    state: LayoutUiState,
    profiles: ProfileSignals,
    controls: Arc<Controls>,
) -> impl IntoElement {
    when(
        move || state.active_error.with(Option::is_some) && !state.error_dismissed.get(),
        move || {
            let controls_reset = controls.clone();
            view()
                .w_full()
                .min_h(px(32.0))
                .px_3()
                .py_1()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_2()
                .bg(status_err_bg())
                .border_b_1()
                .border_b_color(status_err_fg())
                .child(glyph_px(13.0, || Glyph::Alert, || color(status_err_fg())))
                .child(
                    text(move || -> SharedString {
                        let msg = state.active_error.with(|e| e.clone().unwrap_or_default());
                        let prefix = if state.circuit_tripped.get() {
                            tr(
                                "自定义布局连续超时已自动熔断，已回退默认布局：",
                                "Custom layout auto-paused after repeated timeouts; using built-in fallback: ",
                            )()
                        } else {
                            tr(
                                "自定义布局出错，已临时回退到默认布局：",
                                "Custom layout error, using built-in fallback: ",
                            )()
                        };
                        format!("{prefix}{msg}").into()
                    })
                    .flex_1()
                    .min_w(px(0.0))
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail)
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(status_err_fg()),
                )
                .child(when(
                    move || state.circuit_tripped.get(),
                    move || {
                        ui_button(ButtonVariant::Outline, ControlSize::Xs)
                            .accessibility_label(tr_opt("重试启用", "Retry script"))
                            .on_press(move || {
                                let cur = state.active_id.get_untracked();
                                #[cfg(not(target_arch = "wasm32"))]
                                if !cur.is_empty() {
                                    crate::script_worker::reset_script_circuit(&cur);
                                }
                                let _ = &cur;
                                state.circuit_tripped.set(false);
                                state.error_dismissed.set(false);
                                evaluate_active_layout(
                                    state,
                                    profiles.layout_mode.get_untracked(),
                                    current_layout_context(),
                                );
                            })
                            .child(
                                compact_control_text(tr("重试启用", "Retry script"), TEXT_XS)
                                    .text_color(fg_primary()),
                            )
                    },
                ))
                .child(
                    ui_button(ButtonVariant::Outline, ControlSize::Xs)
                        .accessibility_label(tr_opt("编辑布局", "Edit layout"))
                        .on_press(move || {
                            let cur = state.active_id.get_untracked();
                            let target = (!cur.is_empty()).then_some(cur);
                            open_custom_layout_in_editor(state, target.as_deref());
                            profiles.settings_tab.set(SettingsTab::General);
                            profiles.settings_open.set(true);
                        })
                        .child(
                            compact_control_text(tr("编辑布局", "Edit layout"), TEXT_XS)
                                .text_color(fg_primary()),
                        ),
                )
                .child(
                    ui_button(ButtonVariant::Outline, ControlSize::Xs)
                        .accessibility_label(tr_opt("切回默认", "Use built-in"))
                        .on_press(move || {
                            select_builtin_layout(
                                state,
                                profiles.layout_mode,
                                profiles.layout_mode.get_untracked(),
                                &controls_reset,
                            );
                        })
                        .child(
                            compact_control_text(tr("切回默认", "Use built-in"), TEXT_XS)
                                .text_color(fg_primary()),
                        ),
                )
                .child(
                    ui_icon_button(ButtonVariant::Ghost, ControlSize::Xs)
                        .accessibility_label(tr_opt("关闭布局错误提示", "Dismiss layout error"))
                        .on_press(move || state.error_dismissed.set(true))
                        .child(glyph_px(11.0, || Glyph::Close, || color(status_err_fg()))),
                )
        },
    )
}
