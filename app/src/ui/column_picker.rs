#[allow(unused_imports)]
use super::*;

pub(crate) fn column_menu(
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let open = signal(false);
    menu()
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(4.0))
        .margin(px(8.0))
        .popup_style(ui_popup_style)
        .popup_starting_style(ui_popup_starting_style)
        .on_open_change(move |event: &nsh_base::MenuEvent| {
            if let nsh_base::MenuEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .trigger(column_picker_trigger(open))
        .child(column_picker_content(columns, controls))
}

pub(crate) fn column_picker_trigger(open: Signal<bool>) -> impl IntoElement {
    view()
        .role(AccessibilityRole::Button)
        .w(px(CONTROL_H))
        .h(px(CONTROL_H))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(move || {
            if open.get() {
                accent_blue_border()
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .bg(move || {
            if open.get() {
                accent_blue_soft()
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .hover(|style| {
            style.bg(move || {
                if open.get() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .label(tr("列", "Columns"))
        .tooltip(label_tip("列", "Columns"))
        .child(glyph(
            || Glyph::Columns,
            move || {
                if open.get() {
                    color(accent_blue_text())
                } else {
                    color(fg_secondary())
                }
            },
        ))
}

pub(crate) fn column_picker_panel(
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    view()
        .rounded(px(RADIUS_LG))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_lg()
        .overflow_hidden()
        .child(column_picker_content(columns, controls))
}

fn column_picker_section_header(title: impl IntoReactive<SharedString>) -> impl IntoElement {
    view()
        .px_2p5()
        .pt_1p5()
        .pb_0p5()
        .child(ui_section_header(title))
}

fn column_picker_content(
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let fixed_controls = controls.clone();
    let scroll_controls = controls.clone();
    let reset_controls = controls.clone();
    view()
        .w(px(268.0))
        .max_w(move || px((window().width.0 - 16.0).max(160.0)))
        .max_h(move || px((window().height.0 - 24.0).max(180.0)))
        .overflow_hidden()
        .child(
            scroll()
                .w_full()
                .max_h(move || px((window().height.0 - 26.0).max(178.0)))
                .child(
                    view()
                        .w_full()
                        .p_1()
                        .gap_0p5()
                        .child(column_picker_section_header(tr("固定列", "Fixed columns")))
                        .children((0..3).map(move |slot| {
                            column_slot_row(columns, fixed_controls.clone(), slot)
                        }))
                        .child(menu_rule())
                        .child(column_picker_section_header(tr(
                            "可滚动列",
                            "Scrollable columns",
                        )))
                        .children((3..ListColumn::ORDERED.len()).map(move |slot| {
                            column_slot_row(columns, scroll_controls.clone(), slot)
                        }))
                        .child(when(
                            move || columns.get().has_custom_order(),
                            move || column_reset_order(columns, reset_controls.clone()),
                        ))
                        .child(menu_rule())
                        .child(column_picker_section_header(tr("路径展示", "Path display")))
                        .child(column_path_display_choice(
                            columns,
                            controls.clone(),
                            config::PathDisplay::Auto,
                            "自动 (无域名过滤时含域名)",
                            "Auto (Host + Path when unfiltered)",
                        ))
                        .child(column_path_display_choice(
                            columns,
                            controls.clone(),
                            config::PathDisplay::PathOnly,
                            "仅展示 Path",
                            "Path only",
                        ))
                        .child(column_path_display_choice(
                            columns,
                            controls.clone(),
                            config::PathDisplay::HostAndPath,
                            "域名 + Path",
                            "Host + Path",
                        ))
                        .child(column_path_display_choice(
                            columns,
                            controls.clone(),
                            config::PathDisplay::FullUrl,
                            "完整 URL",
                            "Full URL",
                        ))
                        .child(menu_rule())
                        .child(column_picker_section_header(tr("文本省略", "Truncation")))
                        .child(column_ellipsize_choice(
                            columns,
                            controls.clone(),
                            config::ListEllipsize::Head,
                            "开头省略",
                            "Ellipsize head",
                        ))
                        .child(column_ellipsize_choice(
                            columns,
                            controls.clone(),
                            config::ListEllipsize::Middle,
                            "中间省略",
                            "Ellipsize middle",
                        ))
                        .child(column_ellipsize_choice(
                            columns,
                            controls,
                            config::ListEllipsize::Tail,
                            "结尾省略",
                            "Ellipsize tail",
                        )),
                ),
        )
}

pub(crate) fn column_slot_row(
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
    slot: usize,
) -> impl IntoElement {
    let toggle_controls = controls.clone();
    let up_controls = controls.clone();
    let down_controls = controls;
    let is_on = move || {
        let cols = columns.get();
        cols.on(cols.order[slot])
    };
    let locked = move || is_on() && columns.get().visible_count() <= 1;
    let a11y_langs = ui_langs();
    let row_label = move || -> SharedString {
        let cols = columns.get();
        let (zh, en) = cols.order[slot].labels();
        match ui_lang(a11y_langs) {
            Lang::Zh => {
                let state = if !cols.on(cols.order[slot]) {
                    "隐藏"
                } else if cols.visible_count() <= 1 {
                    "锁定"
                } else {
                    "显示"
                };
                format!("{zh} ({state})").into()
            }
            Lang::En => {
                let state = if !cols.on(cols.order[slot]) {
                    "hidden"
                } else if cols.visible_count() <= 1 {
                    "locked"
                } else {
                    "visible"
                };
                format!("{en} ({state})").into()
            }
        }
    };
    view()
        .h(px(30.0))
        .pr_0p5()
        .gap_0p5()
        .flex_row()
        .items_center()
        .child(
            view()
                .role(AccessibilityRole::Button)
                .label(row_label)
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .pl_2p5()
                .pr_1p5()
                .gap_2()
                .rounded(px(RADIUS_MD))
                .flex_row()
                .items_center()
                .opacity(move || if locked() { 0.55_f32 } else { 1.0_f32 })
                .hover(|style| {
                    style.bg(move || {
                        if locked() {
                            AdaptiveColor::TRANSPARENT
                        } else {
                            bg_control_hover()
                        }
                    })
                })
                .active(|style| {
                    style.bg(move || {
                        if locked() {
                            AdaptiveColor::TRANSPARENT
                        } else {
                            bg_control_active()
                        }
                    })
                })
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .disabled(locked)
                .on_press(move || {
                    let current = columns.get();
                    let next = current.toggle(current.order[slot]);
                    if next == current {
                        return;
                    }
                    columns.set(next);
                    if let Err(err) = save_config(|config| next.write(config)) {
                        set_status(
                            &toggle_controls,
                            StatusMsg::PrefUnsaved {
                                name: PrefName::List,
                                err: err.to_string(),
                            },
                        );
                    }
                })
                .child(
                    view()
                        .w(px(10.0))
                        .h(px(10.0))
                        .flex_shrink_0()
                        .rounded(px(RADIUS_SM))
                        .border_1()
                        .border_color(move || if is_on() { accent_blue() } else { fg_muted() })
                        .bg(move || if is_on() { accent_blue() } else { bg_card() }),
                )
                .child(
                    control_text(move || {
                        let (zh, en) = columns.get().order[slot].labels();
                        tr(zh, en)()
                    })
                    .flex_1()
                    .min_w(px(0.0))
                    .number_of_lines(1)
                    .text_color(move || {
                        if is_on() {
                            accent_blue_text()
                        } else {
                            fg_secondary()
                        }
                    }),
                ),
        )
        .child(column_move_button(columns, up_controls, slot, -1))
        .child(column_move_button(columns, down_controls, slot, 1))
}

pub(crate) fn column_move_button(
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
    slot: usize,
    delta: isize,
) -> impl IntoElement {
    let can_move = ListColumns::can_move_slot(slot, delta);
    let icon = if delta < 0 {
        Glyph::ChevronUp
    } else {
        Glyph::Chevron
    };
    let (zh, en) = if delta < 0 {
        ("上移", "Move up")
    } else {
        ("下移", "Move down")
    };
    let btn = view()
        .role(AccessibilityRole::Button)
        .w(px(ControlSize::Xs.height()))
        .h(px(ControlSize::Xs.height()))
        .flex_shrink_0()
        .rounded(px(RADIUS_SM))
        .flex_row()
        .items_center()
        .justify_center()
        .child(glyph_px(
            12.0,
            move || icon,
            move || color(if can_move { fg_secondary() } else { fg_muted() }),
        ));
    if can_move {
        btn.hover(|style| style.bg(bg_control_hover()))
            .active(|style| style.bg(bg_control_active()))
            .transition_colors()
            .duration(MOTION_INSTANT_MS)
            .ease_out()
            .focus_visible(apply_focus_ring)
            .label(tr(zh, en))
            .tooltip(label_tip(zh, en))
            .on_press(move || {
                let current = columns.get();
                let column = current.order[slot];
                let next = current.move_by(column, delta);
                if next == current {
                    return;
                }
                columns.set(next);
                if let Err(err) = save_config(|config| next.write(config)) {
                    set_status(
                        &controls,
                        StatusMsg::PrefUnsaved {
                            name: PrefName::List,
                            err: err.to_string(),
                        },
                    );
                }
            })
    } else {
        btn.opacity(0.28)
    }
}

pub(crate) fn column_reset_order(
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    view()
        .role(AccessibilityRole::Button)
        .h(px(CONTROL_H))
        .px_2p5()
        .gap_2()
        .rounded(px(RADIUS_MD))
        .flex_row()
        .items_center()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .on_press(move || {
            let current = columns.get();
            let next = current.reset_order();
            if next == current {
                return;
            }
            columns.set(next);
            if let Err(err) = save_config(|config| next.write(config)) {
                set_status(
                    &controls,
                    StatusMsg::PrefUnsaved {
                        name: PrefName::List,
                        err: err.to_string(),
                    },
                );
            }
        })
        .child(view().w(px(10.0)).h(px(10.0)).flex_shrink_0())
        .child(
            control_text(tr("恢复默认列序", "Reset column order"))
                .text_size(px(TEXT_SM))
                .text_color(accent_blue_text()),
        )
}

pub(crate) fn column_path_display_choice(
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
    path_display: config::PathDisplay,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    let marked = columns;
    marked_radio_stay(
        move || marked.get().path_display == path_display,
        tr(zh, en),
        move || {
            let next = columns.get().with_path_display(path_display);
            if next == columns.get() {
                return;
            }
            columns.set(next);
            if let Err(err) = save_config(|config| next.write(config)) {
                set_status(
                    &controls,
                    StatusMsg::PrefUnsaved {
                        name: PrefName::List,
                        err: err.to_string(),
                    },
                );
            }
        },
    )
}

pub(crate) fn column_ellipsize_choice(
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
    ellipsize: config::ListEllipsize,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    let marked = columns;
    marked_radio_stay(
        move || marked.get().ellipsize == ellipsize,
        tr(zh, en),
        move || {
            let next = columns.get().with_ellipsize(ellipsize);
            if next == columns.get() {
                return;
            }
            columns.set(next);
            if let Err(err) = save_config(|config| next.write(config)) {
                set_status(
                    &controls,
                    StatusMsg::PrefUnsaved {
                        name: PrefName::List,
                        err: err.to_string(),
                    },
                );
            }
        },
    )
}

pub(crate) fn edit_action_label(
    lang: Lang,
    target_id: Option<u64>,
    paused: Option<(u64, BreakSide)>,
) -> &'static str {
    match (target_id, paused) {
        (Some(id), Some((pid, BreakSide::Response))) if id == pid => {
            t(lang, "编辑响应", "Edit response")
        }
        (Some(id), Some((pid, BreakSide::Request))) if id == pid => {
            t(lang, "编辑请求", "Edit request")
        }
        _ => t(lang, "编辑重发", "Edit & resend"),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn row_context_menu(
    picked: Signal<Option<u64>>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
    compose: Compose,
    controls: Arc<Controls>,
    filter: Signal<String>,
    shown: Shown,
    baseline: Signal<Option<u64>>,
    profiles: ProfileSignals,
    menu_open: Signal<bool>,
) -> impl IntoElement {
    let rules_text = profiles.rules;
    let copy_url = controls.clone();
    let break_controls = controls.clone();
    let copy_curl = controls.clone();
    let copy_ai = controls.clone();
    let focus_controls = controls.clone();
    let mute_controls = controls.clone();
    let pin_controls = controls.clone();
    let pin_mark = controls.clone();
    let pin_label = controls.clone();
    let base_controls = controls.clone();
    let local_check = controls.clone();
    let map_label_check = controls.clone();
    let map_local_controls = controls.clone();
    let quick_rule_controls = controls.clone();
    let repeat_controls = controls.clone();
    let edit_controls = controls.clone();
    let replay_controls = controls.clone();
    let is_marker_check = controls.clone();
    let is_marker = move || {
        picked.get().is_some_and(|id| {
            with_active_session(&is_marker_check, |s| {
                s.get(id).is_some_and(session::Flow::is_marker)
            })
        })
    };
    let marker_filter_controls = controls.clone();
    let marker_copy_controls = controls.clone();
    let marker_copy_status = controls.clone();
    let marker_del_check = controls.clone();
    let marker_del_controls = controls.clone();
    let star_check = controls.clone();
    let note_check = controls.clone();
    let live_check = controls.clone();
    let can_local = move || {
        let controls = &local_check;
        picked.get().is_some_and(|id| {
            with_active_session(controls, |session| {
                session
                    .get(id)
                    .is_some_and(|flow| !flow.is_marker() && flow.relay_source.is_none())
            })
        })
    };
    let can_mutate = move || !is_active_tab_read_only(&star_check);
    let can_note = move || !is_active_tab_read_only(&note_check);
    let can_del_marker = move || !is_active_tab_read_only(&marker_del_check);
    let can_local_break = can_local.clone();
    let can_local_map = can_local.clone();
    let can_quick_rule = can_local.clone();
    let can_live = move || !is_active_tab_read_only(&live_check) && can_local();
    let can_replay = can_live.clone();
    let can_repeat = can_live.clone();
    let can_edit = can_live;
    let can_break = can_local_break;
    let can_map = can_local_map;
    view()
        .w(px(196.0))
        .max_w(move || px((window().width.0 - 16.0).max(160.0)))
        .max_h(move || px((window().height.0 - 24.0).max(160.0)))
        .rounded(px(RADIUS_LG))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_lg()
        .overflow_hidden()
        .child(
            scroll()
                .w_full()
                .max_h(move || px((window().height.0 - 26.0).max(158.0)))
                .child(
                    when(is_marker, move || {
                        let marker_filter_controls = marker_filter_controls.clone();
                        let marker_copy_controls = marker_copy_controls.clone();
                        let marker_copy_status = marker_copy_status.clone();
                        let can_del_marker = can_del_marker.clone();
                        let marker_del_controls = marker_del_controls.clone();
                        view()
                            .w_full()
                            .p_1()
                            .gap_0p5()
                            .child(action_item(
                                tr("只看此标记段", "Filter this section"),
                                closing(menu_open, move || {
                                    if let Some(id) = picked.get()
                                        && let Some(label) =
                                            with_active_session(&marker_filter_controls, |s| {
                                                s.get(id).map(|f| f.path.clone())
                                            })
                                    {
                                        filter.set(marker_filter_token(&label));
                                    }
                                }),
                            ))
                            .child(action_item(
                                tr("复制标记名称", "Copy marker label"),
                                closing(menu_open, move || {
                                    if let Some(id) = picked.get()
                                        && let Some(label) =
                                            with_active_session(&marker_copy_controls, |s| {
                                                s.get(id).map(|f| f.path.clone())
                                            })
                                    {
                                        set_clipboard(label);
                                        set_status(&marker_copy_status, StatusMsg::CopiedMarker);
                                    }
                                }),
                            ))
                            .child(menu_rule())
                            .child(action_item_enabled(
                                tr("删除标记", "Delete marker"),
                                can_del_marker,
                                closing(menu_open, move || {
                                    if guard_read_only_tab(&marker_del_controls) {
                                        return;
                                    }
                                    if let Some(id) = picked.get() {
                                        marker_del_controls.hub.remove_marker(id);
                                        if selected.get() == Some(id) {
                                            selected.set(None);
                                        }
                                    }
                                }),
                            ))
                    })
                    .otherwise(move || {
                        let copy_url = copy_url.clone();
                        let break_controls = break_controls.clone();
                        let copy_curl = copy_curl.clone();
                        let copy_ai = copy_ai.clone();
                        let focus_controls = focus_controls.clone();
                        let mute_controls = mute_controls.clone();
                        let pin_controls = pin_controls.clone();
                        let pin_mark = pin_mark.clone();
                        let pin_label = pin_label.clone();
                        let base_controls = base_controls.clone();
                        let map_label_check = map_label_check.clone();
                        let map_local_controls = map_local_controls.clone();
                        let quick_rule_controls = quick_rule_controls.clone();
                        let repeat_controls = repeat_controls.clone();
                        let edit_controls = edit_controls.clone();
                        let replay_controls = replay_controls.clone();
                        let can_mutate = can_mutate.clone();
                        let can_note = can_note.clone();
                        let can_replay = can_replay.clone();
                        let can_repeat = can_repeat.clone();
                        let can_edit = can_edit.clone();
                        let can_break = can_break.clone();
                        let can_map = can_map.clone();
                        let can_quick_rule = can_quick_rule.clone();
                        view()
                            .w_full()
                            .p_1()
                            .gap_0p5()
                            .child(action_item(
                                tr("复制 URL", "Copy URL"),
                                closing(menu_open, move || {
                                    copy_part(&copy_url, picked.get(), CopyPart::Url);
                                }),
                            ))
                            .child(action_item(
                                tr("复制 cURL", "Copy cURL"),
                                closing(menu_open, move || {
                                    copy_part(&copy_curl, picked.get(), CopyPart::Curl);
                                }),
                            ))
                            .child(action_item(
                                tr("复制给 AI", "Copy for AI"),
                                closing(menu_open, move || {
                                    copy_snippet(&copy_ai, picked.get(), Snippet::AiContext);
                                }),
                            ))
                            .child(menu_rule())
                            .child(action_item(
                                tr("只看", "Focus"),
                                closing(menu_open, move || {
                                    focus_host(&focus_controls, picked.get(), filter);
                                }),
                            ))
                            .child(action_item(
                                tr("屏蔽", "Mute"),
                                closing(menu_open, move || {
                                    mute_host(&mute_controls, picked.get(), filter);
                                }),
                            ))
                            .child(menu_rule())
                            .child(marked_action_enabled(
                                move || flow_pinned(&pin_mark, picked.get()),
                                can_mutate,
                                {
                                    let langs = ui_langs();
                                    move || -> SharedString {
                                        if flow_pinned(&pin_label, picked.get()) {
                                            t(ui_lang(langs), "取消星标", "Unstar").into()
                                        } else {
                                            t(ui_lang(langs), "星标", "Star").into()
                                        }
                                    }
                                },
                                closing(menu_open, move || toggle_pin(&pin_controls, picked.get())),
                            ))
                            .child(marked_action(
                                move || baseline.get().is_some_and(|id| picked.get() == Some(id)),
                                {
                                    let langs = ui_langs();
                                    let picked_label = picked;
                                    let baseline_label = baseline;
                                    move || -> SharedString {
                                        if baseline_label
                                            .get()
                                            .is_some_and(|id| picked_label.get() == Some(id))
                                        {
                                            t(ui_lang(langs), "取消基准", "Clear baseline").into()
                                        } else {
                                            t(ui_lang(langs), "基准", "Baseline").into()
                                        }
                                    }
                                },
                                {
                                    let langs = ui_langs();
                                    closing(menu_open, move || {
                                        set_baseline(
                                            &base_controls,
                                            picked.get(),
                                            baseline,
                                            &shown,
                                            ui_lang(langs),
                                        );
                                    })
                                },
                            ))
                            .child(action_item_enabled(
                                tr("备注…", "Note…"),
                                can_note,
                                closing(menu_open, move || {
                                    shown.repeating.set(false);
                                    shown.editing_comment.set(true);
                                }),
                            ))
                            .child(menu_rule())
                            .child(action_item_enabled(
                                tr("重放", "Replay"),
                                can_replay,
                                closing(menu_open, move || {
                                    replay_flow(
                                        &replay_controls,
                                        picked.get(),
                                        ReplayOverride::default(),
                                    );
                                }),
                            ))
                            .child(action_item_enabled(
                                tr("批量重放…", "Repeat…"),
                                can_repeat,
                                closing(menu_open, move || {
                                    if refuse_relay(&repeat_controls, picked.get()) {
                                        return;
                                    }
                                    shown.editing_comment.set(false);
                                    shown.repeating.set(true);
                                }),
                            ))
                            .child(action_item_enabled(
                                {
                                    let langs = ui_langs();
                                    move || -> SharedString {
                                        edit_action_label(
                                            ui_lang(langs),
                                            picked.get(),
                                            paused.get(),
                                        )
                                        .into()
                                    }
                                },
                                can_edit,
                                closing(menu_open, move || {
                                    let target = picked.get();
                                    if target.is_some() {
                                        selected.set(target);
                                    }
                                    begin_compose(&edit_controls, target, compose);
                                }),
                            ))
                            .child(action_item_enabled(
                                tr("填入断点", "Add break"),
                                can_break,
                                closing(menu_open, move || {
                                    fill_break_rule(&break_controls, picked.get(), rules_text);
                                }),
                            ))
                            .child(action_item_enabled(
                                {
                                    let langs = ui_langs();
                                    move || -> SharedString {
                                        #[cfg(not(target_arch = "wasm32"))]
                                        let saves_file = picked.get().is_some_and(|id| {
                                            with_active_session(&map_label_check, |session| {
                                                session.get(id).is_some_and(|flow| {
                                                    !flow.response_body.is_empty()
                                                })
                                            })
                                        });
                                        #[cfg(target_arch = "wasm32")]
                                        let saves_file = {
                                            let _ = (&picked, &map_label_check);
                                            false
                                        };
                                        if saves_file {
                                            t(ui_lang(langs), "本地映射…", "Map local…").into()
                                        } else {
                                            t(ui_lang(langs), "本地映射", "Map local").into()
                                        }
                                    }
                                },
                                can_map,
                                closing(menu_open, move || {
                                    map_local_from_flow(
                                        map_local_controls.clone(),
                                        picked.get(),
                                        rules_text,
                                    );
                                }),
                            ))
                            .child(flow_quick_rule_section(can_quick_rule, move |action| {
                                menu_open.set(false);
                                open_quick_rule_for_flow(
                                    &quick_rule_controls,
                                    profiles,
                                    picked.get(),
                                    action,
                                );
                            }))
                    }),
                ),
        )
}

pub(crate) fn closing(open: Signal<bool>, run: impl Fn() + 'static) -> impl Fn() + 'static {
    move || {
        open.set(false);
        run();
    }
}

pub(crate) fn quick_kind(kind: u8) -> crate::rules::QuickRule {
    use crate::rules::QuickRule;
    match kind % 16 {
        0 => QuickRule::Replace,
        1 => QuickRule::Remote,
        2 => QuickRule::Local,
        3 => QuickRule::Delay,
        4 => QuickRule::Break,
        5 => QuickRule::BreakRes,
        6 => QuickRule::Header,
        7 => QuickRule::Query,
        8 => QuickRule::Cors,
        9 => QuickRule::Throttle,
        10 => QuickRule::Cert,
        11 => QuickRule::Json,
        12 => QuickRule::NoCache,
        13 => QuickRule::Block,
        14 => QuickRule::Resolve,
        _ => QuickRule::Hook,
    }
}

pub(crate) fn fill_break_rule(controls: &Controls, id: Option<u64>, rules_text: Signal<String>) {
    if refuse_relay(controls, id) {
        return;
    }
    let Some(flow) = selected_flow(controls, id) else {
        return;
    };
    let Some(next) = crate::rules::append_break_line(&rules_text.get(), &flow.host, &flow.path)
    else {
        return;
    };
    let target = format!("{}{}", flow.host, flow.path);
    rules_text.set(next.clone());
    controls.hub.set_rules(&next);
    if let Err(err) = save_config(|config| config.rules = next) {
        set_status(
            controls,
            StatusMsg::PrefUnsaved {
                name: PrefName::Rules,
                err: err.to_string(),
            },
        );
        return;
    }
    set_status(controls, StatusMsg::BreakRuleAdded(target));
}

pub(crate) fn map_local_from_flow(
    controls: Arc<Controls>,
    id: Option<u64>,
    rules_text: Signal<String>,
) {
    if refuse_relay(&controls, id) {
        return;
    }
    let Some(flow) = selected_flow(&controls, id) else {
        return;
    };
    let host = flow.host.clone();
    let path = flow.path.clone();
    let status = flow.status;
    #[cfg(target_arch = "wasm32")]
    let status_only = true;
    #[cfg(not(target_arch = "wasm32"))]
    let status_only = flow.response_body.is_empty();
    if status_only {
        let Some(next) = crate::rules::append_map_local_line(
            &rules_text.get(),
            &host,
            &path,
            None,
            Some(status.unwrap_or(200)),
        ) else {
            return;
        };
        rules_text.set(next.clone());
        controls.hub.set_rules(&next);
        if let Err(err) = save_config(|config| config.rules = next) {
            set_status(
                &controls,
                StatusMsg::PrefUnsaved {
                    name: PrefName::Rules,
                    err: err.to_string(),
                },
            );
            return;
        }
        set_status(&controls, StatusMsg::MappedLocal(format!("{host}{path}")));
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let ext = present::content_type(&flow.response_headers)
            .and_then(present::image_extension)
            .unwrap_or_else(|| body_extension(&flow.response_headers, &flow.response_body));
        let dialog = save_dialog_with_default_dir()
            .file_name(format!("{}.{}", flow.id, ext))
            .filter(nsh_core::file_dialog::FileFilter::new(
                ext.to_ascii_uppercase(),
                [ext],
            ));
        let body = flow.response_body;
        spawn_detached(async move {
            match pick_save_path_after_paint(dialog).await {
                DialogOutcome::Picked(save_path) => {
                    if let Err(err) = write_export_file(&save_path, &body) {
                        set_status(&controls, StatusMsg::SaveFailed(err.to_string()));
                        return;
                    }
                    let file_text = save_path.display().to_string();
                    let Some(next) = crate::rules::append_map_local_line(
                        &rules_text.get(),
                        &host,
                        &path,
                        Some(&file_text),
                        status,
                    ) else {
                        return;
                    };
                    rules_text.set(next.clone());
                    controls.hub.set_rules(&next);
                    if let Err(err) = save_config(|config| config.rules = next) {
                        set_status(
                            &controls,
                            StatusMsg::PrefUnsaved {
                                name: PrefName::Rules,
                                err: err.to_string(),
                            },
                        );
                        return;
                    }
                    set_clipboard(file_text.clone());
                    set_status(&controls, StatusMsg::MappedLocal(file_text));
                }
                DialogOutcome::Cancelled => set_status(&controls, StatusMsg::SaveCancelled),
                DialogOutcome::Busy => {}
                DialogOutcome::Failed(err) => set_status(&controls, StatusMsg::SaveFailed(err)),
            }
        });
    }
}

pub(crate) fn flow_pinned(controls: &Controls, id: Option<u64>) -> bool {
    id.is_some_and(|id| {
        with_active_session(controls, |session| {
            session.get(id).is_some_and(|flow| flow.pinned)
        })
    })
}
