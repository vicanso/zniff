#[allow(unused_imports)]
use super::*;

pub(crate) const TOOLBAR_H: f32 = 42.0;
pub(crate) const TOOLBAR_PAD_END: f32 = 12.0;
pub(crate) const TITLE_BAR_CONTROLS_GAP: f32 = 6.0;

/// Reserves horizontal toolbar padding for native window controls (macOS traffic
/// lights, GTK `WindowControls`, Windows caption buttons) so edge controls remain clickable.
pub(crate) fn toolbar_side_padding(title_left: f32, title_right: f32) -> (f32, f32) {
    let left = if title_left > 0.0 {
        (title_left + TITLE_BAR_CONTROLS_GAP).max(LIST_INSET)
    } else {
        LIST_INSET
    };
    let right = if title_right > 0.0 {
        (title_right + TITLE_BAR_CONTROLS_GAP).max(TOOLBAR_PAD_END)
    } else {
        TOOLBAR_PAD_END
    };
    (left, right)
}

pub(crate) fn toolbar_bar_height(title_top: f32) -> f32 {
    if title_top > TOOLBAR_H {
        title_top
    } else {
        TOOLBAR_H
    }
}

/// Formats the hover tooltip for the toolbar primary filter pill, preserving the
/// full untruncated filter expression (and any active secondary filter) plus the
/// click-to-toggle hint.
pub(crate) fn format_filter_pill_tooltip(
    primary_filter: &str,
    effective_filter: &str,
    filter_enabled: bool,
    lang: Lang,
) -> String {
    let primary = primary_filter.trim();
    let effective = effective_filter.trim();
    let secondary = if filter_enabled {
        if !effective.is_empty() && effective != primary {
            effective
                .strip_prefix(primary)
                .map(str::trim)
                .filter(|rest| !rest.is_empty())
                .unwrap_or(effective)
        } else {
            ""
        }
    } else {
        effective
    };
    let hint = if filter_enabled {
        t(lang, "点击暂停主过滤", "Click to pause primary filter")
    } else {
        t(
            lang,
            "主过滤已暂停 · 点击启用",
            "Primary filter paused · Click to enable",
        )
    };
    let mut lines = Vec::with_capacity(3);
    if !primary.is_empty() {
        lines.push(primary.to_string());
    }
    if !secondary.is_empty() {
        let prefix = t(lang, "二次过滤: ", "Secondary filter: ");
        lines.push(format!("{prefix}{secondary}"));
    }
    lines.push(hint.to_string());
    lines.join("\n")
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn toolbar(
    status: Signal<StatusMsg>,
    running: Signal<bool>,
    ca_ui: CaSignals,
    use_system_proxy: Signal<bool>,
    install: impl Fn() + 'static,
    remove: impl Fn() + 'static,
    toggle_proxy: impl Fn() + 'static,
    clear: impl Fn() + 'static,
    clear_unpinned: impl Fn() + 'static,
    export_har: impl Fn() + 'static,
    copy_env: impl Fn() + 'static,
    toggle: impl Fn() + 'static,
    import_controls: Arc<Controls>,
    rows: Signal<Vec<Arc<RowLine>>>,
    flow_total: Signal<usize>,
    filter: Signal<String>,
    pretty: Signal<bool>,
    shown_phase: Signal<Phase>,
    share_id: Signal<String>,
    profiles: ProfileSignals,
    compose: Compose,
    seq_builder: SeqBuilder,
    selected: Signal<Option<u64>>,
    update_state: Signal<UpdateState>,
) -> impl IntoElement {
    let install = Rc::new(install);
    let clear = Rc::new(clear);
    let clear_unpinned = Rc::new(clear_unpinned);
    let remove = Rc::new(remove);
    let preset_controls = import_controls.clone();
    let rules_controls = import_controls.clone();
    let network_controls = import_controls.clone();
    let layout_controls = import_controls.clone();
    let shared_rule_draft = QuickRuleDraftSignals {
        kind: signal(0_u8),
        host: signal(String::new()),
        path: signal(String::new()),
        op: signal(String::new()),
        from: signal(String::new()),
        to: signal(String::new()),
        modes: QuickRuleModeSignals::new(),
    };
    #[cfg(not(target_arch = "wasm32"))]
    let update_controls = import_controls.clone();
    let bar = view()
        .window_drag(true)
        .flex_row()
        .items_center()
        .gap_1()
        .pl(move || {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let edges = nsh_core::element::desktop::title_bar();
                px(toolbar_side_padding(edges.left.0, edges.right.0).0)
            }
            #[cfg(target_arch = "wasm32")]
            {
                px(LIST_INSET)
            }
        })
        .pr(move || {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let edges = nsh_core::element::desktop::title_bar();
                px(toolbar_side_padding(edges.left.0, edges.right.0).1)
            }
            #[cfg(target_arch = "wasm32")]
            {
                px(TOOLBAR_PAD_END)
            }
        })
        .h(move || {
            #[cfg(not(target_arch = "wasm32"))]
            {
                px(toolbar_bar_height(
                    nsh_core::element::desktop::title_bar().top.0,
                ))
            }
            #[cfg(target_arch = "wasm32")]
            {
                px(TOOLBAR_H)
            }
        })
        .bg(bg_chrome())
        .border_b_1()
        .border_b_color(border_strong())
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .pr_1()
                .child(
                    view()
                        .w(px(20.0))
                        .h(px(20.0))
                        .rounded(px(5.5))
                        .bg(accent_blue())
                        .items_center()
                        .justify_center()
                        .child(
                            mono("Z")
                                .text_size(px(12.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(white()),
                        ),
                )
                .child(
                    text("zniff")
                        .text_size(px(14.5))
                        .font_weight(FontWeight::BOLD)
                        .text_color(fg_primary()),
                )
                .child(
                    view()
                        .w(px(7.0))
                        .h(px(7.0))
                        .ml_0p5()
                        .rounded_full()
                        .flex_shrink_0()
                        .bg(move || {
                            let has_err = matches!(
                                status.get(),
                                StatusMsg::BindFailed { .. }
                                    | StatusMsg::CaUnavailable(_)
                                    | StatusMsg::StopRestoreFailed(_)
                                    | StatusMsg::ProxyApplyFailed { .. }
                                    | StatusMsg::ProxyRestoreFailed(_)
                            );
                            if has_err {
                                status_warn_fg()
                            } else if running.get() {
                                status_ok_fg()
                            } else {
                                fg_muted()
                            }
                        }),
                ),
        )
        .child(
            view()
                .w(px(1.5))
                .h(px(18.0))
                .rounded_full()
                .flex_shrink_0()
                .bg(border_divider()),
        )
        .child({
            let langs = ui_langs();
            let startup_error = move || match status.get() {
                msg @ (StatusMsg::BindFailed { .. }
                | StatusMsg::CaUnavailable(_)
                | StatusMsg::StopRestoreFailed(_)
                | StatusMsg::ProxyApplyFailed { .. }
                | StatusMsg::ProxyRestoreFailed(_)) => Some(msg.text(ui_lang(langs))),
                _ => None,
            };
            let condition_info = move || {
                let effective_rules =
                    effective_rules_text(&profiles.rules.get(), &profiles.rule_groups.get());
                toolbar_status_summary(
                    &profiles.filter.get(),
                    &filter.get(),
                    profiles.filter_enabled.get(),
                    &profiles.bypass.get(),
                    &effective_rules,
                    &profiles.upstream.get(),
                    profiles.rules_on.get(),
                    ui_lang(langs),
                )
            };
            let has_primary_filter = move || !profiles.filter.get().trim().is_empty();
            view()
                .flex_1()
                .flex_row()
                .items_center()
                .gap_1p5()
                .px_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .child(when(
                    move || startup_error().is_none(),
                    move || {
                        view()
                            .min_w(px(0.0))
                            .flex_shrink(1.0)
                            .child(preset_pill_popover(
                                profiles,
                                preset_controls.clone(),
                                shared_rule_draft,
                            ))
                    },
                ))
                .child(when(
                    move || startup_error().is_none(),
                    move || {
                        view().min_w(px(0.0)).flex_shrink(0.0).child(
                            toolbar_network_preset_popover(profiles, network_controls.clone()),
                        )
                    },
                ))
                .child(when(
                    move || startup_error().is_none() && has_primary_filter(),
                    move || {
                        let is_active_pill = move || profiles.filter_enabled.get();
                        view()
                            .window_drag(false)
                            .h(px(24.0))
                            .max_w(px(240.0))
                            .min_w(px(0.0))
                            .px_2()
                            .gap_1()
                            .rounded(px(6.0))
                            .overflow_hidden()
                            .flex_shrink(1.0)
                            .flex_row()
                            .items_center()
                            .cursor(Cursor::Pointer)
                            .border_1()
                            .border_color(move || {
                                if is_active_pill() {
                                    accent_blue_border()
                                } else {
                                    border_control()
                                }
                            })
                            .bg(move || {
                                if is_active_pill() {
                                    accent_blue_soft()
                                } else {
                                    bg_control()
                                }
                            })
                            .hover(|style| {
                                style.bg(move || {
                                    if is_active_pill() {
                                        accent_blue_soft_hover()
                                    } else {
                                        bg_control_hover()
                                    }
                                })
                            })
                            .active(|style| style.opacity(0.8))
                            .label(tr("切换过滤生效状态", "Toggle filter active state"))
                            .tooltip(
                                tooltip().child(
                                    view()
                                        .max_w(px(420.0))
                                        .px_2p5()
                                        .py_1p5()
                                        .rounded(px(6.0))
                                        .bg(bg_card())
                                        .border_1()
                                        .border_color(border_strong())
                                        .child(
                                            text(move || -> SharedString {
                                                format_filter_pill_tooltip(
                                                    &profiles.filter.get(),
                                                    &filter.get(),
                                                    profiles.filter_enabled.get(),
                                                    ui_lang(langs),
                                                )
                                                .into()
                                            })
                                            .text_size(px(11.5))
                                            .text_color(fg_primary()),
                                        ),
                                ),
                            )
                            .on_press(move || {
                                profiles.filter_enabled.update(|on| *on = !*on);
                            })
                            .child(glyph_px(
                                10.5,
                                move || {
                                    if profiles.filter_enabled.get() {
                                        Glyph::Check
                                    } else {
                                        Glyph::History
                                    }
                                },
                                move || {
                                    if is_active_pill() {
                                        color(accent_blue_text())
                                    } else {
                                        color(fg_muted())
                                    }
                                },
                            ))
                            .child(
                                mono(move || {
                                    let prefix = t(ui_lang(langs), "过滤: ", "Filter: ");
                                    let expr = profiles.filter.get();
                                    SharedString::from(format!("{prefix}{}", expr.trim()))
                                })
                                .min_w(px(0.0))
                                .flex_shrink(1.0)
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Tail)
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || {
                                    if is_active_pill() {
                                        accent_blue_text()
                                    } else {
                                        fg_muted()
                                    }
                                }),
                            )
                    },
                ))
                .child(when(
                    move || startup_error().is_some() || !has_primary_filter(),
                    move || {
                        when(
                            move || startup_error().is_none() && condition_info().1,
                            move || {
                                mono(move || condition_info().0)
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .number_of_lines(1)
                                    .text_overflow(EllipsizeMode::Tail)
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(move || {
                                        if condition_info().2 {
                                            fg_muted()
                                        } else {
                                            fg_secondary()
                                        }
                                    })
                            },
                        )
                        .otherwise(move || {
                            text(move || {
                                if let Some(err) = startup_error() {
                                    return err;
                                }
                                condition_info().0
                            })
                            .flex_1()
                            .min_w(px(0.0))
                            .number_of_lines(1)
                            .text_overflow(EllipsizeMode::Tail)
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || {
                                if startup_error().is_some() {
                                    status_warn_fg()
                                } else {
                                    fg_muted()
                                }
                            })
                        })
                    },
                ))
        });
    #[cfg(not(target_arch = "wasm32"))]
    let bar = bar.child(toolbar_update_pill(update_state, update_controls));
    bar.child(toolbar_layout_switcher(
        profiles.layout_mode,
        layout_controls,
    ))
    .child(toolbar_summary_button(
        rows,
        flow_total,
        filter,
        selected,
        compose,
        seq_builder,
    ))
    .child(saved_filter_menu(profiles))
    .child(rules_menu_popover(
        profiles,
        rules_controls,
        shared_rule_draft,
    ))
    .child(toolbar_text_button(tr("清空", "Clear"), {
        let clear = Rc::clone(&clear);
        move || clear()
    }))
    .child(toolbar_menu(
        ca_ui,
        use_system_proxy,
        install,
        remove,
        toggle_proxy,
        export_har,
        copy_env,
        {
            let clear_unpinned = Rc::clone(&clear_unpinned);
            move || clear_unpinned()
        },
        import_controls,
        rows,
        filter,
        pretty,
        shown_phase,
        share_id,
        profiles,
        compose,
        seq_builder,
        selected,
        update_state,
    ))
    .child(primary_button(
        {
            let langs = ui_langs();
            move || -> SharedString {
                if running.get() {
                    t(ui_lang(langs), "停止", "Stop").into()
                } else {
                    t(ui_lang(langs), "开始", "Start").into()
                }
            }
        },
        move || running.get(),
        toggle,
    ))
}

pub(crate) fn toolbar_text_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .window_drag(false)
        .h(px(CONTROL_H))
        .px_2()
        .gap_1()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(border_control())
        .bg(bg_control())
        .hover(|style| style.bg(bg_control_hover()))
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.8))
        .focus_visible(apply_focus_ring)
        .tooltip(label_tip(
            "清空会话 (Cmd/Ctrl+K)",
            "Clear session (Cmd/Ctrl+K)",
        ))
        .on_press(on_press)
        .child(glyph_px(13.5, || Glyph::Trash, || color(fg_secondary())))
        .child(control_text(label).text_color(fg_primary()))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn toolbar_menu(
    ca_ui: CaSignals,
    proxy_on: Signal<bool>,
    install: Rc<impl Fn() + 'static>,
    remove: Rc<impl Fn() + 'static>,
    toggle_proxy: impl Fn() + 'static,
    export_har: impl Fn() + 'static,
    copy_env: impl Fn() + 'static,
    clear_unpinned: impl Fn() + 'static,
    import_controls: Arc<Controls>,
    rows: Signal<Vec<Arc<RowLine>>>,
    filter: Signal<String>,
    pretty: Signal<bool>,
    shown_phase: Signal<Phase>,
    share_id: Signal<String>,
    profiles: ProfileSignals,
    compose: Compose,
    seq_builder: SeqBuilder,
    selected: Signal<Option<u64>>,
    update_state: Signal<UpdateState>,
) -> impl IntoElement {
    let _ = (install, remove, toggle_proxy, pretty);
    let open = signal(false);
    popover()
        .open(move || Some(open.get()))
        .on_open_change(move |event: &nsh_base::PopoverEvent| {
            if let nsh_base::PopoverEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(8.0))
        .trigger(more_trigger(ca_ui.status, proxy_on, update_state, open))
        .child(more_menu(
            open,
            export_har,
            copy_env,
            clear_unpinned,
            import_controls,
            rows,
            filter,
            shown_phase,
            share_id,
            profiles,
            compose,
            seq_builder,
            selected,
            update_state,
        ))
}

/// Session size for the toolbar Summary button. A filter shows
/// `matched / total`; an empty filter shows the total alone.
pub(crate) fn flow_count_text(filtered: usize, total: usize, filtering: bool) -> String {
    if filtering {
        format!("{filtered} / {total}")
    } else {
        total.to_string()
    }
}

pub(crate) fn flow_count_now(
    rows: Signal<Vec<Arc<RowLine>>>,
    flow_total: Signal<usize>,
    filter: Signal<String>,
) -> String {
    flow_count_text(
        rows.with(Vec::len),
        flow_total.get(),
        !filter.get().is_empty(),
    )
}

pub(crate) fn toolbar_layout_switcher(
    layout_mode: Signal<config::LayoutMode>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let open = signal(false);
    let modes = [
        (
            config::LayoutMode::Split,
            Glyph::LayoutSplit,
            "左右分栏",
            "Left / Right split",
        ),
        (
            config::LayoutMode::TopBottom,
            Glyph::LayoutTopBottom,
            "上下分栏",
            "Top / Bottom split",
        ),
    ];
    let current_icon = move || match layout_mode.get() {
        config::LayoutMode::Split => Glyph::LayoutSplit,
        config::LayoutMode::TopBottom => Glyph::LayoutTopBottom,
    };
    let mut menu = view()
        .w(px(164.0))
        .p_1()
        .gap_0p5()
        .rounded(px(8.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md();
    for (mode, icon, zh, en) in modes {
        let mode_controls = controls.clone();
        menu = menu.child(
            view()
                .w_full()
                .h(px(28.0))
                .px_2()
                .rounded(px(5.0))
                .flex_row()
                .items_center()
                .gap_2()
                .cursor(Cursor::Pointer)
                .bg(move || {
                    if layout_mode.get() == mode {
                        accent_blue_soft()
                    } else {
                        bg_card()
                    }
                })
                .hover(|style| {
                    style.bg(move || {
                        if layout_mode.get() == mode {
                            accent_blue_soft_hover()
                        } else {
                            bg_control_hover()
                        }
                    })
                })
                .active(|style| style.opacity(0.8))
                .on_press(move || {
                    open.set(false);
                    if layout_mode.get() == mode {
                        return;
                    }
                    layout_mode.set(mode);
                    if let Err(err) = save_config(|config| config.layout_mode = mode) {
                        set_status(
                            &mode_controls,
                            StatusMsg::PrefUnsaved {
                                name: PrefName::List,
                                err: err.to_string(),
                            },
                        );
                    }
                })
                .child(glyph_px(
                    13.0,
                    move || icon,
                    move || {
                        if layout_mode.get() == mode {
                            color(accent_blue_text())
                        } else {
                            color(fg_muted())
                        }
                    },
                ))
                .child(
                    text(tr(zh, en))
                        .flex_1()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if layout_mode.get() == mode {
                                accent_blue_text()
                            } else {
                                fg_primary()
                            }
                        }),
                ),
        );
    }
    popover()
        .open(move || Some(open.get()))
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(8.0))
        .on_open_change(move |event: &nsh_base::PopoverEvent| {
            if let nsh_base::PopoverEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .trigger(
            view()
                .window_drag(false)
                .h(px(28.0))
                .px_2()
                .gap_1p5()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
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
                .label(tr("分栏布局", "Split layout"))
                .tooltip(label_tip("切换分栏布局", "Switch split layout"))
                .child(glyph_px(13.0, current_icon, move || {
                    if open.get() {
                        color(accent_blue_text())
                    } else {
                        color(fg_secondary())
                    }
                }))
                .child(when(
                    move || window().width.0 >= 860.0,
                    move || {
                        control_text(tr("分栏", "Layout")).text_color(move || {
                            if open.get() {
                                accent_blue_text()
                            } else {
                                fg_primary()
                            }
                        })
                    },
                )),
        )
        .child(menu)
}

pub(crate) fn toolbar_summary_button(
    rows: Signal<Vec<Arc<RowLine>>>,
    flow_total: Signal<usize>,
    filter: Signal<String>,
    selected: Signal<Option<u64>>,
    compose: Compose,
    seq_builder: SeqBuilder,
) -> impl IntoElement {
    let is_summary_active = move || {
        selected.get().is_none()
            && !compose.composing.get()
            && !seq_builder.open.get()
            && !seq_builder.redact_open.get()
            && !rows.with(Vec::is_empty)
    };
    view()
        .window_drag(false)
        .h(px(CONTROL_H))
        .px_2()
        .gap_1()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(move || {
            if is_summary_active() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if is_summary_active() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if is_summary_active() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .focus_visible(apply_focus_ring)
        .cursor(Cursor::Pointer)
        .tooltip(label_tip("查看会话统计概览", "View session summary"))
        .on_press(move || {
            reset_compose(compose);
            close_sequence_diagram(seq_builder);
            selected.set(None);
        })
        .child(control_text(tr("统计", "Summary")).text_color(move || {
            if is_summary_active() {
                accent_blue_text()
            } else {
                fg_primary()
            }
        }))
        .child(
            view()
                .h(px(16.0))
                .px_1p5()
                .rounded(px(4.0))
                .flex_row()
                .items_center()
                .justify_center()
                .bg(move || {
                    if is_summary_active() {
                        accent_blue_soft_hover()
                    } else {
                        bg_card()
                    }
                })
                .child(
                    mono(move || SharedString::from(flow_count_now(rows, flow_total, filter)))
                        .tabular_nums()
                        .number_of_lines(1)
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || {
                            if is_summary_active() {
                                accent_blue_text()
                            } else {
                                fg_secondary()
                            }
                        }),
                ),
        )
}

pub(crate) fn more_trigger(
    ca_status: Signal<ca::CaStatus>,
    proxy_on: Signal<bool>,
    update_state: Signal<UpdateState>,
    open: Signal<bool>,
) -> impl IntoElement {
    let dot_status = ca_status;
    let dot_proxy = proxy_on;
    let dot_update = update_state;
    let is_tinted = move || open.get() || proxy_on.get();
    view()
        .window_drag(false)
        .h(px(CONTROL_H))
        .px_2()
        .gap_1()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(move || {
            if is_tinted() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if is_tinted() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if is_tinted() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.75))
        .focus_visible(apply_focus_ring)
        .child(when(
            move || {
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = (
                        dot_proxy.get(),
                        dot_status.get(),
                        dot_update.get().nudge.is_some(),
                    );
                    false
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    dot_proxy.get()
                        || dot_status.get() != ca::CaStatus::Trusted
                        || dot_update.get().nudge.is_some()
                }
            },
            move || {
                view().w(px(6.0)).h(px(6.0)).rounded_full().bg(move || {
                    if dot_status.get() != ca::CaStatus::Trusted {
                        status_warn_fg()
                    } else {
                        accent_blue()
                    }
                })
            },
        ))
        .child(control_text(tr("更多", "More")).text_color(move || {
            if is_tinted() {
                accent_blue_text()
            } else {
                fg_primary()
            }
        }))
        .child(glyph(
            || Glyph::Chevron,
            move || {
                color(if is_tinted() {
                    accent_blue_text()
                } else {
                    fg_secondary()
                })
            },
        ))
}
