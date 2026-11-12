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
        .on_open_change(move |event: &nsh_base::MenuEvent| {
            if let nsh_base::MenuEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .trigger(column_picker_trigger(open))
        .child(column_picker_panel(columns, controls))
}

pub(crate) fn column_picker_trigger(open: Signal<bool>) -> impl IntoElement {
    view()
        .w(px(30.0))
        .h(px(30.0))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(move || {
            if open.get() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if open.get() {
                accent_blue_soft()
            } else {
                bg_control()
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
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
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
    let fixed_controls = controls.clone();
    let scroll_controls = controls.clone();
    let reset_controls = controls.clone();
    view()
        .w(px(204.0))
        .max_w(move || px((window().width.0 - 16.0).max(160.0)))
        .max_h(move || px((window().height.0 - 24.0).max(180.0)))
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
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
                        .children((0..3).map(move |slot| {
                            column_slot_row(columns, fixed_controls.clone(), slot)
                        }))
                        .child(menu_rule())
                        .children((3..ListColumn::ORDERED.len()).map(move |slot| {
                            column_slot_row(columns, scroll_controls.clone(), slot)
                        }))
                        .child(when(
                            move || columns.get().has_custom_order(),
                            move || column_reset_order(columns, reset_controls.clone()),
                        ))
                        .child(menu_rule())
                        .child(column_full_url_choice(
                            columns,
                            controls.clone(),
                            false,
                            "仅展示 Path",
                            "Path only",
                        ))
                        .child(column_full_url_choice(
                            columns,
                            controls.clone(),
                            true,
                            "完整 URL",
                            "Full URL",
                        ))
                        .child(menu_rule())
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
    view()
        .h(px(30.0))
        .pr_0p5()
        .gap_0p5()
        .flex_row()
        .items_center()
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .pl_2p5()
                .pr_1p5()
                .gap_2()
                .rounded(px(6.0))
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
                .active(|style| style.opacity(move || if locked() { 0.55_f32 } else { 0.75_f32 }))
                .cursor(Cursor::Pointer)
                .disabled(locked)
                .disabled_style(|style| style.cursor(Cursor::Default))
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
                        .w(px(8.5))
                        .h(px(8.5))
                        .flex_shrink_0()
                        .rounded(px(2.5))
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
        .w(px(22.0))
        .h(px(22.0))
        .flex_shrink_0()
        .rounded(px(5.0))
        .flex_row()
        .items_center()
        .justify_center()
        .child(glyph_px(
            11.5,
            move || icon,
            move || color(if can_move { fg_secondary() } else { fg_muted() }),
        ));
    if can_move {
        btn.hover(|style| style.bg(bg_control_hover()))
            .active(|style| style.opacity(0.75))
            .cursor(Cursor::Pointer)
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
        btn.opacity(0.28).cursor(Cursor::Default)
    }
}

pub(crate) fn column_reset_order(
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    view()
        .h(px(28.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.opacity(0.75))
        .cursor(Cursor::Pointer)
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
        .child(view().w(px(8.5)).h(px(8.5)).flex_shrink_0())
        .child(
            control_text(tr("恢复默认列序", "Reset column order"))
                .text_size(px(11.5))
                .text_color(accent_blue_text()),
        )
}

pub(crate) fn column_full_url_choice(
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
    full_url: bool,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    let marked = columns;
    marked_radio_stay(
        move || marked.get().full_url == full_url,
        tr(zh, en),
        move || {
            let next = columns.get().with_full_url(full_url);
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
    rules_text: Signal<String>,
    menu_open: Signal<bool>,
) -> impl IntoElement {
    let copy_url = controls.clone();
    let break_controls = controls.clone();
    let copy_curl = controls.clone();
    let focus_controls = controls.clone();
    let mute_controls = controls.clone();
    let pin_controls = controls.clone();
    let pin_mark = controls.clone();
    let pin_label = controls.clone();
    let base_controls = controls.clone();
    let local_check = controls.clone();
    let map_label_check = controls.clone();
    let map_local_controls = controls.clone();
    let repeat_controls = controls.clone();
    let edit_controls = controls.clone();
    let replay_controls = controls;
    let can_local = move || {
        let controls = &local_check;
        picked.get().is_some_and(|id| {
            controls.hub.with_session(|session| {
                session
                    .get(id)
                    .is_some_and(|flow| flow.relay_source.is_none())
            })
        })
    };
    let can_replay = can_local.clone();
    let can_repeat = can_local.clone();
    let can_edit = can_local.clone();
    let can_break = can_local.clone();
    let can_map = can_local;
    view()
        .w(px(196.0))
        .max_w(move || px((window().width.0 - 16.0).max(160.0)))
        .max_h(move || px((window().height.0 - 24.0).max(160.0)))
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
        .overflow_hidden()
        .child(
            scroll()
                .w_full()
                .max_h(move || px((window().height.0 - 26.0).max(158.0)))
                .child(
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
                            "cURL",
                            closing(menu_open, move || {
                                copy_part(&copy_curl, picked.get(), CopyPart::Curl);
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
                        .child(marked_action(
                            move || flow_pinned(&pin_mark, picked.get()),
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
                        .child(action_item(
                            tr("备注…", "Note…"),
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
                                    edit_action_label(ui_lang(langs), picked.get(), paused.get())
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
                                        map_label_check.hub.with_session(|session| {
                                            session
                                                .get(id)
                                                .is_some_and(|flow| !flow.response_body.is_empty())
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
                        )),
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
    match kind % 14 {
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
        _ => QuickRule::Block,
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
    }
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
            match pick_save_path(dialog).await {
                Ok(Some(save_path)) => {
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
                Ok(None) => set_status(&controls, StatusMsg::SaveCancelled),
                Err(err) => set_status(&controls, StatusMsg::SaveFailed(err)),
            }
        });
    }
}

pub(crate) fn flow_pinned(controls: &Controls, id: Option<u64>) -> bool {
    id.is_some_and(|id| {
        controls
            .hub
            .with_session(|session| session.get(id).is_some_and(|flow| flow.pinned))
    })
}
