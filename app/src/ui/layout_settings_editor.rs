//! Validation, persistence actions, and Settings inline editor for custom TSX layouts.

#[allow(unused_imports)]
use super::*;

use std::collections::BTreeMap;

use crate::layout_spec::LayoutContext;

pub(crate) fn validate_draft_layout(state: LayoutUiState, ctx: LayoutContext) {
    state.draft_pending_at.replace(None);
    let seq = state.draft_eval_seq.get().wrapping_add(1);
    state.draft_eval_seq.replace(seq);
    let raw_id = state.draft_id.get_untracked();
    if let Err(err) = validate_layout_id_syntax(&raw_id) {
        state.preview_status.set(Err(err));
        return;
    }
    let source = state.draft_source.get_untracked();
    #[cfg(not(target_arch = "wasm32"))]
    {
        match crate::layout_runtime::eval_layout_tsx(&source, ctx) {
            Ok(spec) => state.preview_status.set(Ok(format_spec_summary(&spec))),
            Err(err) => state.preview_status.set(Err(err)),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        state.busy.set(true);
        spawn_detached(async move {
            let outcome = proxy::eval_web_layout(&source, ctx).await;
            if state.draft_eval_seq.get() == seq {
                match outcome {
                    Ok(spec) => state.preview_status.set(Ok(format_spec_summary(&spec))),
                    Err(err) => state.preview_status.set(Err(err.to_string())),
                }
            }
            state.busy.set(false);
        });
    }
}

pub(crate) fn save_draft_layout(state: LayoutUiState, ctx: LayoutContext, controls: Arc<Controls>) {
    state.draft_pending_at.replace(None);
    let id = match validate_layout_id_syntax(&state.draft_id.get_untracked()) {
        Ok(id) => id,
        Err(err) => {
            state.preview_status.set(Err(err));
            return;
        }
    };
    let source = state.draft_source.get_untracked();
    state.delete_confirm.set(None);

    #[cfg(not(target_arch = "wasm32"))]
    {
        let spec = match crate::layout_runtime::eval_layout_tsx(&source, ctx) {
            Ok(spec) => spec,
            Err(err) => {
                state.preview_status.set(Err(err));
                return;
            }
        };
        let dir = layout_store_dir();
        let entry = match crate::layout_runtime::save_custom_layout(&dir, &id, &source) {
            Ok(entry) => entry,
            Err(err) => {
                state.preview_status.set(Err(err));
                return;
            }
        };
        crate::layout_runtime::store_cached_layout_spec(&dir, &entry.id, ctx, &spec);
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
        state
            .active_source
            .set(Some((entry.id.clone(), entry.source.clone())));
        state.circuit_tripped.set(false);
        state.preview_status.set(Ok(format_spec_summary(&spec)));
        if state.active_spec.with_untracked(|cur| *cur != spec) {
            state.active_spec.set(spec);
        }
        select_custom_layout(state, &entry.id, &controls);
        state.active_error.set(None);
        state.reload_rev.update(|v| *v += 1);
    }
    #[cfg(target_arch = "wasm32")]
    {
        state.busy.set(true);
        spawn_detached(async move {
            match proxy::save_web_layout(&id, &source, ctx).await {
                Ok(entry) => {
                    let meta = CustomLayoutMeta {
                        id: entry.id.clone(),
                        label: entry.label,
                        file_name: entry.file_name,
                        source: entry.source,
                    };
                    state.custom_layouts.update(|list| {
                        list.retain(|item| item.id != meta.id);
                        list.push(meta);
                        list.sort_by(|a, b| a.id.cmp(&b.id));
                    });
                    select_custom_layout(state, &entry.id, &controls);
                    state.active_error.set(None);
                    state
                        .preview_status
                        .set(Ok("✓ 已保存并应用（Save & apply complete）".to_string()));
                    state.reload_rev.update(|v| *v += 1);
                }
                Err(err) => state.preview_status.set(Err(err.to_string())),
            }
            state.busy.set(false);
        });
    }
}

pub(crate) fn trigger_delete_layout(
    state: LayoutUiState,
    layout_mode_sig: Signal<config::LayoutMode>,
    controls: Arc<Controls>,
) {
    let Ok(id) = validate_layout_id_syntax(&state.draft_id.get_untracked()) else {
        state.preview_status.set(Err(t_now(
            "请先输入有效的布局 ID",
            "Enter a valid layout ID first",
        )));
        return;
    };
    let now = clock::now();
    let confirmed = state.delete_confirm.with_untracked(
        |slot| matches!(slot, Some((target, until)) if target == &id && now <= *until),
    );
    if !confirmed {
        let arm_until = now + Duration::from_millis(3500);
        let armed_id = id.clone();
        state.delete_confirm.set(Some((id, arm_until)));
        set_timeout(Duration::from_millis(3500), move || {
            state.delete_confirm.update(|slot| {
                if let Some((cur_id, cur_until)) = slot
                    && *cur_id == armed_id
                    && *cur_until <= clock::now()
                {
                    *slot = None;
                }
            });
        });
        return;
    }
    state.delete_confirm.set(None);

    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = layout_store_dir();
        if let Err(err) = crate::layout_runtime::delete_custom_layout(&dir, &id) {
            state.preview_status.set(Err(err));
            return;
        }
        apply_deleted_layout_state(state, &id, layout_mode_sig, &controls);
    }
    #[cfg(target_arch = "wasm32")]
    {
        state.busy.set(true);
        spawn_detached(async move {
            match proxy::delete_web_layout(&id).await {
                Ok(()) => apply_deleted_layout_state(state, &id, layout_mode_sig, &controls),
                Err(err) => state.preview_status.set(Err(err.to_string())),
            }
            state.busy.set(false);
        });
    }
}

fn apply_deleted_layout_state(
    state: LayoutUiState,
    id: &str,
    layout_mode_sig: Signal<config::LayoutMode>,
    controls: &Controls,
) {
    state
        .custom_layouts
        .update(|list| list.retain(|item| item.id != id));
    let mut all = state.all_pane_sizes.get_untracked();
    if all.remove(id).is_some() {
        state.all_pane_sizes.set(all.clone());
        save_config_or_notify(controls, PrefName::List, |cfg| cfg.layout_pane_sizes = all);
    }
    state.editor_open.set(false);
    if state.active_id.get_untracked() == id {
        select_builtin_layout(
            state,
            layout_mode_sig,
            layout_mode_sig.get_untracked(),
            controls,
        );
    }
    state.reload_rev.update(|v| *v += 1);
}

#[cfg(not(target_arch = "wasm32"))]
fn open_layouts_folder() {
    #[cfg(not(test))]
    {
        let store_dir = layout_store_dir();
        let dir = crate::layout_runtime::layouts_dir(&store_dir);
        let _ = crate::layout_runtime::ensure_layout_types(&store_dir);
        std::thread::spawn(move || {
            #[cfg(target_os = "macos")]
            let _ = std::process::Command::new("open").arg(&dir).spawn();
            #[cfg(target_os = "linux")]
            let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
            #[cfg(target_os = "windows")]
            let _ = std::process::Command::new("explorer").arg(&dir).spawn();
        });
    }
}

/// Custom TSX layout selector and inline script editor inside Settings -> General -> Reading & Layout.
pub(crate) fn settings_custom_layouts_section(
    state: LayoutUiState,
    layout_mode: Signal<config::LayoutMode>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    ensure_custom_layouts_loaded(state);
    let controls_chips = controls.clone();
    let controls_reset = controls.clone();
    let header_actions = view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(
            ui_button(ButtonVariant::Outline, ControlSize::Xs)
                .accessibility_label(tr_opt("新建布局脚本", "New layout script"))
                .on_press(move || open_custom_layout_in_editor(state, None))
                .child(compact_control_text(tr("新建", "New"), TEXT_XS).text_color(fg_primary())),
        )
        .child(
            ui_button(ButtonVariant::Outline, ControlSize::Xs)
                .accessibility_label(tr_opt("重载布局列表", "Reload layouts"))
                .on_press(move || reload_custom_layouts(state))
                .child(
                    compact_control_text(tr("重载", "Reload"), TEXT_XS).text_color(fg_primary()),
                ),
        )
        .child(when(
            move || {
                !state.active_id.with(String::is_empty)
                    && !state.active_pane_overrides.with(BTreeMap::is_empty)
            },
            move || {
                let c = controls_reset.clone();
                ui_button(ButtonVariant::Outline, ControlSize::Xs)
                    .accessibility_label(tr_opt("重置分栏尺寸", "Reset pane sizes"))
                    .on_press(move || reset_pane_size_overrides(state, &c))
                    .child(
                        compact_control_text(tr("重置尺寸", "Reset sizes"), TEXT_XS)
                            .text_color(fg_primary()),
                    )
            },
        ));
    #[cfg(not(target_arch = "wasm32"))]
    let header_actions = header_actions.child(
        ui_button(ButtonVariant::Outline, ControlSize::Xs)
            .accessibility_label(tr_opt("打开布局目录", "Open layouts folder"))
            .on_press(open_layouts_folder)
            .child(
                compact_control_text(tr("打开目录", "Open folder"), TEXT_XS)
                    .text_color(fg_primary()),
            ),
    );

    view()
        .w_full()
        .pt_1()
        .gap_2()
        .flex_col()
        .border_t_1()
        .border_t_color(border_subtle())
        .child(
            view()
                .w_full()
                .px_2p5()
                .gap_2()
                .flex_row()
                .items_center()
                .child(
                    control_text(tr("自定义 TSX 布局", "Custom TSX layouts"))
                        .flex_1()
                        .text_color(fg_secondary()),
                )
                .child(header_actions),
        )
        .child(custom_layout_chips(state, controls_chips))
        .child(when(
            move || state.editor_open.get(),
            move || custom_layout_editor_card(state, layout_mode, controls.clone()),
        ))
}

fn custom_layout_chips(state: LayoutUiState, controls: Arc<Controls>) -> impl IntoElement {
    view()
        .w_full()
        .px_2p5()
        .child(when(
            move || state.custom_layouts.with(Vec::is_empty),
            move || {
                text(tr(
                    "暂无自定义布局脚本（点击“新建”或向 layouts/ 目录添加 .tsx 文件）",
                    "No custom TSX layouts yet (click New or add .tsx files to layouts/)",
                ))
                .text_size(px(TEXT_XS))
                .text_color(fg_muted())
            },
        ))
        .child(view().w_full().flex_row().flex_wrap().gap_1p5().child(each(
            move || state.custom_layouts.get(),
            |item| (item.id.clone(), item.label.clone()),
            move |item, _| {
                let (item_id, label) = item.with_untracked(|it| (it.id.clone(), it.label.clone()));
                let select_id = item_id.clone();
                let edit_id = item_id.clone();
                let active_check = item_id.clone();
                let active_text_id = item_id.clone();
                let border_id = item_id;
                let c = controls.clone();
                view()
                    .h(px(ControlSize::Xs.height()))
                    .pl_2()
                    .pr_1()
                    .gap_1()
                    .rounded(px(RADIUS_SM))
                    .border_1()
                    .border_color(move || {
                        if state.active_id.with(|cur| *cur == border_id) {
                            accent_blue_border()
                        } else {
                            border_control()
                        }
                    })
                    .bg(move || {
                        if state.active_id.with(|cur| *cur == active_check) {
                            accent_blue_soft()
                        } else {
                            bg_control()
                        }
                    })
                    .flex_row()
                    .items_center()
                    .child(
                        button()
                            .accessibility_label(label.clone())
                            .on_press(move || select_custom_layout(state, &select_id, &c))
                            .child(compact_control_text(label.clone(), TEXT_XS).text_color(
                                move || {
                                    if state.active_id.with(|cur| *cur == active_text_id) {
                                        accent_blue_text()
                                    } else {
                                        fg_primary()
                                    }
                                },
                            )),
                    )
                    .child(
                        ui_icon_button(ButtonVariant::Ghost, ControlSize::Xs)
                            .accessibility_label(tr_opt("编辑此布局", "Edit layout"))
                            .on_press(move || {
                                open_custom_layout_in_editor(state, Some(&edit_id));
                            })
                            .child(glyph_px(11.0, || Glyph::RuleRewrite, || color(fg_muted()))),
                    )
            },
        )))
}

fn custom_layout_editor_card(
    state: LayoutUiState,
    layout_mode: Signal<config::LayoutMode>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let controls_save = controls.clone();
    let controls_del = controls;
    let draft_id = state.draft_id;
    let draft_source = state.draft_source;
    view()
        .w_full()
        .p_2p5()
        .gap_2()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border_control())
        .bg(bg_card())
        .flex_col()
        .child(
            view()
                .w_full()
                .gap_2()
                .flex_row()
                .items_center()
                .child(
                    compact_control_text(tr("布局 ID", "Layout ID"), TEXT_XS)
                        .text_color(fg_secondary()),
                )
                .child(
                    ui_input_a11y(input(), tr("布局 ID", "Layout ID"))
                        .value(move || draft_id.get())
                        .placeholder(tr("例如：focus", "e.g. focus"))
                        .placeholder_color(move || color(fg_muted()))
                        .w(px(160.0))
                        .h(px(ControlSize::Xs.height()))
                        .px_2()
                        .rounded(px(RADIUS_SM))
                        .bg(bg_code())
                        .border_1()
                        .border_color(border_control())
                        .text_size(move || scaled_px(TEXT_XS))
                        .text_color(fg_primary())
                        .on_change(move |next: &str| {
                            draft_id.set(next.to_string());
                            state.delete_confirm.set(None);
                        }),
                )
                .child(
                    compact_control_text(
                        move || SharedString::from(format!("{}.tsx", draft_id.get().trim())),
                        TEXT_XS,
                    )
                    .text_color(fg_muted()),
                ),
        )
        .child(
            guard_culled_pointer_events(view())
                .w_full()
                .h(px(210.0))
                .rounded(px(RADIUS_SM))
                .border_1()
                .border_color(border_control())
                .bg(bg_code())
                .overflow_hidden()
                .child(
                    editor()
                        .value(move || SharedString::from(draft_source.get()))
                        .language("javascript")
                        .line_numbers(true)
                        .folding(true)
                        .soft_wrap(false)
                        .font_size(move || scaled_px(12.0))
                        .line_height(move || scaled_px(18.0))
                        .scroll_beyond_last_line(false)
                        .on_change(move |next: &SharedString| {
                            draft_source.set(next.to_string());
                            schedule_debounced_draft_validation(state);
                        })
                        .size_full(),
                ),
        )
        .child(when(
            move || {
                state.preview_status.with(|r| match r {
                    Ok(s) => !s.is_empty(),
                    Err(e) => !e.is_empty(),
                })
            },
            move || {
                text(move || -> SharedString {
                    state
                        .preview_status
                        .with(|r| match r {
                            Ok(s) => s.clone(),
                            Err(e) => e.clone(),
                        })
                        .into()
                })
                .number_of_lines(2)
                .text_overflow(EllipsizeMode::Tail)
                .text_size(px(TEXT_XS))
                .text_color(move || {
                    if state.preview_status.with(Result::is_ok) {
                        status_ok_fg()
                    } else {
                        status_err_fg()
                    }
                })
            },
        ))
        .child(custom_layout_actions_row(
            state,
            layout_mode,
            controls_save,
            controls_del,
        ))
}

fn custom_layout_actions_row(
    state: LayoutUiState,
    layout_mode: Signal<config::LayoutMode>,
    controls_save: Arc<Controls>,
    controls_del: Arc<Controls>,
) -> impl IntoElement {
    let delete_armed = move || {
        let cur = state.draft_id.get().trim().to_string();
        state
            .delete_confirm
            .with(|slot| matches!(slot, Some((id, _)) if *id == cur && !cur.is_empty()))
    };
    let can_delete = move || {
        let cur = state.draft_id.get().trim().to_string();
        !cur.is_empty()
            && state
                .custom_layouts
                .with(|list| list.iter().any(|e| e.id == cur))
    };

    view()
        .w_full()
        .flex_row()
        .items_center()
        .gap_1p5()
        .child(
            ui_button(ButtonVariant::Outline, ControlSize::Xs)
                .disabled(move || state.busy.get())
                .accessibility_label(tr_opt("校验布局脚本", "Validate layout script"))
                .on_press(move || validate_draft_layout(state, current_layout_context()))
                .child(
                    compact_control_text(tr("校验", "Validate"), TEXT_XS).text_color(fg_primary()),
                ),
        )
        .child(
            ui_button(ButtonVariant::Primary, ControlSize::Xs)
                .disabled(move || state.busy.get())
                .accessibility_label(tr_opt("保存并应用布局", "Save and apply layout"))
                .on_press(move || {
                    save_draft_layout(state, current_layout_context(), controls_save.clone())
                })
                .child(
                    compact_control_text(tr("保存并应用", "Save & Apply"), TEXT_XS)
                        .text_color(fg_on_accent()),
                ),
        )
        .child(view().flex_1())
        .child(when(can_delete, move || {
            let c = controls_del.clone();
            ui_button(ButtonVariant::Outline, ControlSize::Xs)
                .border_color(status_err_fg())
                .bg(move || {
                    if delete_armed() {
                        status_err_bg()
                    } else {
                        bg_card()
                    }
                })
                .accessibility_label(tr_opt("删除自定义布局", "Delete custom layout"))
                .on_press(move || trigger_delete_layout(state, layout_mode, c.clone()))
                .child(
                    compact_control_text(
                        move || -> SharedString {
                            if delete_armed() {
                                tr("再次点击确认删除", "Confirm delete")().into()
                            } else {
                                tr("删除", "Delete")().into()
                            }
                        },
                        TEXT_XS,
                    )
                    .text_color(status_err_fg()),
                )
        }))
        .child(
            ui_button(ButtonVariant::Ghost, ControlSize::Xs)
                .accessibility_label(tr_opt("收起布局编辑器", "Close layout editor"))
                .on_press(move || {
                    state.delete_confirm.set(None);
                    state.editor_open.set(false);
                })
                .child(
                    compact_control_text(tr("取消", "Cancel"), TEXT_XS).text_color(fg_secondary()),
                ),
        )
}
