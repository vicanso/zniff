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

/// Title-bar toggle button for the left Domains & Apps sidebar in Split layout.
pub(crate) fn toolbar_sidebar_toggle(
    layout_mode: Signal<config::LayoutMode>,
    sidebar_collapsed: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    when(
        move || layout_mode.get() == config::LayoutMode::Split,
        move || {
            let controls = controls.clone();
            view()
                .window_drag(false)
                .mr_1()
                .flex_shrink_0()
                .tooltip(
                    tooltip().child(ui_tooltip_surface(
                        text(move || {
                            if sidebar_collapsed.get() {
                                tr("展开侧边栏", "Expand sidebar")()
                            } else {
                                tr("收起侧边栏", "Collapse sidebar")()
                            }
                        })
                        .text_size(px(TEXT_SM))
                        .text_color(fg_primary()),
                    )),
                )
                .child(
                    ui_icon_button(ButtonVariant::Ghost, ControlSize::Sm)
                        .selected(move || !sidebar_collapsed.get())
                        .selected_style(|s| s.bg(bg_control()))
                        .accessibility_label(move || {
                            Some(SharedString::from(if sidebar_collapsed.get() {
                                tr("展开域名与应用栏", "Expand Domains & Apps sidebar")()
                            } else {
                                tr("收起域名与应用栏", "Collapse Domains & Apps sidebar")()
                            }))
                        })
                        .on_press(move || {
                            if let Some(layout_ref) = use_context::<SidebarLayoutRef>() {
                                layout_ref.animate();
                            }
                            let next = !sidebar_collapsed.get();
                            sidebar_collapsed.set(next);
                            if let Err(err) = save_config(|config| config.sidebar_collapsed = next)
                            {
                                set_status(
                                    &controls,
                                    StatusMsg::PrefUnsaved {
                                        name: PrefName::List,
                                        err: err.to_string(),
                                    },
                                );
                            }
                        })
                        .child(glyph_px(
                            ControlSize::Sm.icon_size(),
                            move || {
                                if sidebar_collapsed.get() {
                                    Glyph::Sidebar
                                } else {
                                    Glyph::SidebarOpen
                                }
                            },
                            move || {
                                color(if sidebar_collapsed.get() {
                                    fg_secondary()
                                } else {
                                    fg_primary()
                                })
                            },
                        )),
                )
        },
    )
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
    let clear_unpinned = Rc::new(clear_unpinned);
    let remove = Rc::new(remove);
    let preset_controls = import_controls.clone();
    let filter_controls = import_controls.clone();
    let rules_controls = import_controls.clone();
    let network_controls = import_controls.clone();
    let layout_controls = import_controls.clone();
    let sidebar_controls = import_controls.clone();
    let shared_rule_draft = QuickRuleDraftSignals {
        kind: signal(0_u8),
        host: signal(String::new()),
        path: signal(String::new()),
        op: signal(String::new()),
        from: signal(String::new()),
        to: signal(String::new()),
        modes: QuickRuleModeSignals::new(),
        editing_line: signal(None),
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
        .child(toolbar_sidebar_toggle(
            profiles.layout_mode,
            profiles.sidebar_collapsed,
            sidebar_controls,
        ))
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
                        .rounded(px(RADIUS_MD))
                        .bg(accent_blue())
                        .items_center()
                        .justify_center()
                        .child(
                            mono("Z")
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::BOLD)
                                .text_color(fg_on_accent()),
                        ),
                )
                .child(
                    text("zniff")
                        .text_size(px(TEXT_MD))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg_primary()),
                )
                .child(
                    view()
                        .w(px(6.0))
                        .h(px(6.0))
                        .ml_0p5()
                        .rounded_full()
                        .flex_shrink_0()
                        .bg(move || {
                            let has_err = status.with(|s| {
                                matches!(
                                    s,
                                    StatusMsg::BindFailed { .. }
                                        | StatusMsg::CaUnavailable(_)
                                        | StatusMsg::StopRestoreFailed(_)
                                        | StatusMsg::ProxyApplyFailed { .. }
                                        | StatusMsg::ProxyRestoreFailed(_)
                                )
                            });
                            if has_err {
                                status_warn_fg()
                            } else if running.get() {
                                status_ok_fg()
                            } else {
                                fg_muted()
                            }
                        })
                        .style_with(move |s| {
                            let has_err = status.with(|msg| {
                                matches!(
                                    msg,
                                    StatusMsg::BindFailed { .. }
                                        | StatusMsg::CaUnavailable(_)
                                        | StatusMsg::StopRestoreFailed(_)
                                        | StatusMsg::ProxyApplyFailed { .. }
                                        | StatusMsg::ProxyRestoreFailed(_)
                                )
                            });
                            if running.get() && !has_err && !reduce_motion() {
                                s.animation(ui_pulse_animation(MOTION_PULSE_MS))
                            } else {
                                s
                            }
                        }),
                ),
        )
        .child(
            view()
                .w(px(1.0))
                .h(px(16.0))
                .mx_0p5()
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
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
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
                            move || startup_error().is_none() && has_primary_filter(),
                            move || {
                                let is_active_pill = move || profiles.filter_enabled.get();
                                view()
                                    .window_drag(false)
                                    .max_w(px(240.0))
                                    .min_w(px(0.0))
                                    .flex_shrink(1.0)
                                    .tooltip(
                                        tooltip().child(ui_tooltip_surface(
                                            text(move || -> SharedString {
                                                format_filter_pill_tooltip(
                                                    &profiles.filter.get(),
                                                    &filter.get(),
                                                    profiles.filter_enabled.get(),
                                                    ui_lang(langs),
                                                )
                                                .into()
                                            })
                                            .text_size(px(TEXT_SM))
                                            .text_color(fg_primary()),
                                        )),
                                    )
                                    .child(
                                        nsh_base::toggle()
                                            .pressed(is_active_pill)
                                            .h(px(CONTROL_H))
                                            .max_w(px(240.0))
                                            .min_w(px(0.0))
                                            .px_2()
                                            .gap_1()
                                            .rounded(px(RADIUS_MD))
                                            .overflow_hidden()
                                            .flex_shrink(1.0)
                                            .flex_row()
                                            .items_center()
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
                                            .active(|style| style.bg(bg_control_active()))
                                            .transition_colors()
                                            .duration(MOTION_INSTANT_MS)
                                            .ease_out()
                                            .focus_visible(apply_focus_ring)
                                            .accessibility_label(tr_opt(
                                                "切换过滤生效状态",
                                                "Toggle filter active state",
                                            ))
                                            .on_change(move |_: &bool| {
                                                profiles.filter_enabled.update(|on| *on = !*on);
                                            })
                                            .child(glyph_px(
                                                ControlSize::Xs.icon_size(),
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
                                                    let prefix =
                                                        t(ui_lang(langs), "过滤: ", "Filter: ");
                                                    let expr = profiles.filter.get();
                                                    SharedString::from(format!(
                                                        "{prefix}{}",
                                                        expr.trim()
                                                    ))
                                                })
                                                .min_w(px(0.0))
                                                .flex_shrink(1.0)
                                                .number_of_lines(1)
                                                .text_overflow(EllipsizeMode::Tail)
                                                .text_size(px(TEXT_SM))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(move || {
                                                    if is_active_pill() {
                                                        accent_blue_text()
                                                    } else {
                                                        fg_muted()
                                                    }
                                                }),
                                            ),
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
                                            .text_size(px(TEXT_SM))
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
                                    .text_size(px(TEXT_SM))
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
                        )),
                )
                .child(when(
                    move || startup_error().is_none(),
                    move || {
                        view().min_w(px(0.0)).flex_shrink(0.0).child(
                            toolbar_network_preset_popover(profiles, network_controls.clone()),
                        )
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
        shown_phase,
        compose,
        seq_builder,
    ))
    .child(saved_filter_menu(
        profiles,
        filter_controls,
        shared_rule_draft,
    ))
    .child(rules_menu_popover(
        profiles,
        rules_controls,
        shared_rule_draft,
    ))
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
    .child(
        view()
            .w(px(1.0))
            .h(px(16.0))
            .mx_0p5()
            .flex_shrink_0()
            .bg(border_divider()),
    )
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
    bind_popover_dismiss(open);
    popover()
        .open(move || Some(open.get()))
        .popup_style(ui_popup_style)
        .popup_starting_style(ui_popup_starting_style)
        .popup_ending_style(ui_popup_ending_style)
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
    let total_str = present::format_thousands(total as u64);
    if filtering {
        format!(
            "{} / {total_str}",
            present::format_thousands(filtered as u64)
        )
    } else {
        total_str
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
    let mut menu = view().w(px(164.0)).p_1().gap_0p5().flex_col();
    for (mode, icon, zh, en) in modes {
        let mode_controls = controls.clone();
        menu = menu.child(
            button()
                .selected(move || layout_mode.get() == mode)
                .w_full()
                .h(px(ControlSize::Sm.height()))
                .px_2()
                .rounded(px(RADIUS_MD))
                .flex_row()
                .items_center()
                .gap_2()
                .bg(move || {
                    if layout_mode.get() == mode {
                        accent_blue_soft()
                    } else {
                        AdaptiveColor::TRANSPARENT
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
                .active(|style| style.bg(bg_control_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .accessibility_label(tr_opt(zh, en))
                .on_press(move |_: &()| {
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
                    ControlSize::Sm.icon_size(),
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
                        .text_size(px(TEXT_SM))
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
    bind_popover_dismiss(open);
    popover()
        .open(move || Some(open.get()))
        .popup_style(ui_popup_style)
        .popup_starting_style(ui_popup_starting_style)
        .popup_ending_style(ui_popup_ending_style)
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
                .role(AccessibilityRole::Button)
                .h(px(CONTROL_H))
                .px_2()
                .gap_1p5()
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
                .label(tr("分栏布局", "Split layout"))
                .tooltip(label_tip_unless(
                    open,
                    "切换分栏布局",
                    "Switch split layout",
                ))
                .child(glyph_px(
                    ControlSize::Sm.icon_size(),
                    current_icon,
                    move || {
                        if open.get() {
                            color(accent_blue_text())
                        } else {
                            color(fg_secondary())
                        }
                    },
                ))
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
    shown_phase: Signal<Phase>,
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
    no_drag(
        ui_toggle_button(ControlSize::Sm, is_summary_active)
            .on_press(move || {
                reset_compose(compose);
                seq_builder.resume_ids.set(Vec::new());
                close_sequence_diagram(seq_builder);
                selected.set(None);
                shown_phase.set(Phase::Empty);
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
                    .rounded(px(RADIUS_SM))
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .bg(move || {
                        if is_summary_active() {
                            accent_blue_soft_hover()
                        } else {
                            bg_control()
                        }
                    })
                    .child(
                        mono(move || SharedString::from(flow_count_now(rows, flow_total, filter)))
                            .tabular_nums()
                            .number_of_lines(1)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || {
                                if is_summary_active() {
                                    accent_blue_text()
                                } else {
                                    fg_secondary()
                                }
                            }),
                    ),
            ),
    )
    .tooltip(label_tip("查看会话统计概览", "View session summary"))
}

pub(crate) fn more_trigger(
    ca_status: Signal<ca::CaStatus>,
    proxy_on: Signal<bool>,
    update_state: Signal<UpdateState>,
    open: Signal<bool>,
) -> impl IntoElement {
    let _ = (ca_status, proxy_on);
    let dot_update = update_state;
    let is_tinted = move || open.get();
    view()
        .window_drag(false)
        .role(AccessibilityRole::Button)
        .h(px(CONTROL_H))
        .px_2()
        .gap_1()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(move || {
            if is_tinted() {
                accent_blue_border()
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .bg(move || {
            if is_tinted() {
                accent_blue_soft()
            } else {
                AdaptiveColor::TRANSPARENT
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
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .child(when(
            move || {
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = dot_update.get().nudge.is_some();
                    false
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    dot_update.get().nudge.is_some()
                }
            },
            move || {
                view()
                    .w(px(6.0))
                    .h(px(6.0))
                    .rounded_full()
                    .bg(accent_blue())
            },
        ))
        .child(control_text(tr("更多", "More")).text_color(move || {
            if is_tinted() {
                accent_blue_text()
            } else {
                fg_primary()
            }
        }))
        .child(glyph_px(
            ControlSize::Xs.icon_size(),
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
