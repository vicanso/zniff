//! Reactive state, validation, and Settings editor for custom TSX workspace layouts.

#[allow(unused_imports)]
use super::*;

use std::collections::BTreeMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

use crate::layout_spec::{FooterDock, LayoutContext, LayoutSpec, LayoutTarget, ViewModule};

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
    pub(crate) active_error: Signal<Option<String>>,
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
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) layouts_loaded: Signal<bool>,
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

export default ({ width }: LayoutContext) => (
  <Layout name="Custom Focus" footer="bottom">
    {width < 920 ? (
      <Col>
        <Pane name="flow_list" size="46%" />
        <Pane name="detail" size="1fr" />
      </Col>
    ) : (
      <Row>
        <Pane name="sidebar" size={220} />
        <Pane name="flow_list" size="1fr" />
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

pub(crate) fn init_layout_ui_state(
    active_layout: &str,
    pane_sizes: &BTreeMap<String, BTreeMap<String, u32>>,
    layout_mode: config::LayoutMode,
) -> LayoutUiState {
    let clean_active = config::sanitize_layout_id_pref(active_layout);
    let clean_sizes = config::sanitize_layout_pane_sizes(pane_sizes.clone());
    let active_overrides = if clean_active.is_empty() {
        BTreeMap::new()
    } else {
        clean_sizes.get(&clean_active).cloned().unwrap_or_default()
    };
    #[cfg(not(target_arch = "wasm32"))]
    let (entries, template, initial_spec, initial_err) = {
        let dir = native_store::layout_store_dir();
        let _ = crate::layout_runtime::ensure_layout_types(&dir);
        let entries: Vec<CustomLayoutMeta> = crate::layout_runtime::list_custom_layouts(&dir)
            .into_iter()
            .map(|e| CustomLayoutMeta {
                id: e.id,
                label: e.label,
                file_name: e.file_name,
                source: e.source,
            })
            .collect();
        let template = crate::layout_runtime::default_custom_layout_template("focus");
        let (spec, err) = crate::layout_runtime::resolve_active_layout(
            &dir,
            &clean_active,
            layout_mode,
            LayoutContext {
                width: config::WINDOW_WIDTH_DEFAULT as f32,
                height: config::WINDOW_HEIGHT_DEFAULT as f32,
                target: LayoutTarget::Desktop,
            },
        );
        (entries, template, spec, err)
    };
    #[cfg(target_arch = "wasm32")]
    let (entries, template, initial_spec, initial_err) = (
        Vec::new(),
        fallback_layout_template(),
        LayoutSpec::for_layout_mode(layout_mode),
        None,
    );

    let state = LayoutUiState {
        active_id: signal(clean_active),
        active_spec: signal(initial_spec),
        active_error: signal(initial_err),
        error_dismissed: signal(false),
        custom_layouts: signal(entries),
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
        layouts_loaded: signal(!cfg!(target_arch = "wasm32")),
    };

    #[cfg(target_arch = "wasm32")]
    reload_custom_layouts(state);

    state
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
    state.active_id.set(String::new());
    state.active_error.set(None);
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
    all.insert(active_id.clone(), updated_map);
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
                    let entries = payload
                        .layouts
                        .into_iter()
                        .map(|e| CustomLayoutMeta {
                            id: e.id,
                            label: e.label,
                            file_name: e.file_name,
                            source: e.source,
                        })
                        .collect();
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

pub(crate) fn evaluate_active_layout(
    state: LayoutUiState,
    layout_mode: config::LayoutMode,
    ctx: LayoutContext,
) {
    let active_id = state.active_id.get_untracked();
    if active_id.is_empty() {
        let spec = LayoutSpec::for_layout_mode(layout_mode);
        if state.active_spec.with_untracked(|cur| *cur != spec) {
            state.active_spec.set(spec);
        }
        if state.active_error.with_untracked(Option::is_some) {
            state.active_error.set(None);
            state.error_dismissed.set(false);
        }
        return;
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = native_store::layout_store_dir();
        let (next_spec, next_err) =
            crate::layout_runtime::resolve_active_layout(&dir, &active_id, layout_mode, ctx);
        if state.active_spec.with_untracked(|cur| *cur != next_spec) {
            state.active_spec.set(next_spec);
        }
        if state.active_error.with_untracked(|cur| *cur != next_err) {
            state.active_error.set(next_err);
            state.error_dismissed.set(false);
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
        let expected_id = active_id.clone();
        spawn_detached(async move {
            let outcome = proxy::eval_web_layout(&source, ctx).await;
            if state.active_id.get_untracked() != expected_id {
                return;
            }
            match outcome {
                Ok(spec) => {
                    if state.active_spec.with_untracked(|cur| *cur != spec) {
                        state.active_spec.set(spec);
                    }
                    state.active_error.set(None);
                }
                Err(err) => {
                    let fallback = LayoutSpec::for_layout_mode(layout_mode);
                    if state.active_spec.with_untracked(|cur| *cur != fallback) {
                        state.active_spec.set(fallback);
                    }
                    let msg = Some(err.to_string());
                    if state.active_error.with_untracked(|cur| *cur != msg) {
                        state.active_error.set(msg);
                        state.error_dismissed.set(false);
                    }
                }
            }
        });
    }
}

/// Dismissible workspace banner shown when the active custom TSX layout fails to load or evaluate.
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
                        let prefix = tr(
                            "自定义布局出错，已临时回退到默认布局：",
                            "Custom layout error, using built-in fallback: ",
                        )();
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
