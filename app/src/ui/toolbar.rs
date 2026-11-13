#[allow(unused_imports)]
use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn toolbar(
    status: Signal<StatusMsg>,
    running: Signal<bool>,
    ca_status: Signal<ca::CaStatus>,
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
    let filter_controls = import_controls.clone();
    let deselect_controls = import_controls.clone();
    #[cfg(not(target_arch = "wasm32"))]
    let update_controls = import_controls.clone();
    const TOOLBAR_H: f32 = 42.0;
    let bar = view()
        .window_drag(true)
        .flex_row()
        .items_center()
        .gap_2()
        .pl(move || {
            #[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
            {
                let left = nsh_core::element::desktop::title_bar().left.0;
                if left > 0.0 {
                    return px(left + 6.0);
                }
            }
            px(LIST_INSET)
        })
        .pr_3()
        .h(px(TOOLBAR_H))
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
                ),
        )
        .child(view().w(px(1.0)).h(px(16.0)).bg(border_strong()))
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
            let active_name = move || {
                toolbar_active_profile_name(
                    &profiles.saved.get(),
                    &profiles.name.get(),
                    &profiles.filter.get(),
                    &profiles.bypass.get(),
                    &profiles.rules.get(),
                    &profiles.upstream.get(),
                )
            };
            let condition_info = move || {
                toolbar_filter_conditions(
                    &filter.get(),
                    &profiles.bypass.get(),
                    &profiles.rules.get(),
                    &profiles.upstream.get(),
                    profiles.rules_on.get(),
                    ui_lang(langs),
                )
            };
            view()
                .flex_1()
                .flex_row()
                .items_center()
                .gap_2()
                .px_1()
                .min_w(px(0.0))
                .child(
                    view()
                        .w(px(7.0))
                        .h(px(7.0))
                        .rounded_full()
                        .flex_shrink_0()
                        .bg(move || {
                            if startup_error().is_some() {
                                status_warn_fg()
                            } else if running.get() {
                                status_ok_fg()
                            } else {
                                fg_muted()
                            }
                        }),
                )
                .child(when(
                    move || startup_error().is_none() && active_name().is_some(),
                    move || {
                        let deselect_controls = deselect_controls.clone();
                        view()
                            .window_drag(false)
                            .h(px(22.0))
                            .max_w(px(184.0))
                            .min_w(px(0.0))
                            .rounded(px(5.0))
                            .overflow_hidden()
                            .flex_row()
                            .items_center()
                            .border_1()
                            .border_color(accent_blue_border())
                            .bg(accent_blue_soft())
                            .child(
                                view()
                                    .h_full()
                                    .max_w(px(162.0))
                                    .min_w(px(0.0))
                                    .pl_2()
                                    .pr_1p5()
                                    .flex_row()
                                    .items_center()
                                    .cursor(Cursor::Pointer)
                                    .hover(|style| style.bg(accent_blue_soft_hover()))
                                    .active(|style| style.opacity(0.8))
                                    .tooltip(label_tip(
                                        "打开过滤与规则配置",
                                        "Open filters & rules config",
                                    ))
                                    .on_press(move || {
                                        profiles.panel_open.set(true);
                                    })
                                    .child(
                                        text(move || active_name().unwrap_or_default())
                                            .max_w(px(148.0))
                                            .min_w(px(0.0))
                                            .number_of_lines(1)
                                            .text_overflow(EllipsizeMode::Tail)
                                            .text_size(px(11.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(accent_blue_text()),
                                    ),
                            )
                            .child(
                                view()
                                    .w(px(1.0))
                                    .h(px(12.0))
                                    .flex_shrink_0()
                                    .bg(accent_blue_border()),
                            )
                            .child(
                                view()
                                    .w(px(20.0))
                                    .h(px(20.0))
                                    .flex_shrink_0()
                                    .flex_row()
                                    .items_center()
                                    .justify_center()
                                    .cursor(Cursor::Pointer)
                                    .hover(|style| style.bg(accent_blue_soft_hover()))
                                    .active(|style| style.opacity(0.75))
                                    .label(tr("取消选择配置", "Deselect setup"))
                                    .tooltip(label_tip(
                                        "取消选择当前配置",
                                        "Deselect current setup",
                                    ))
                                    .on_press(move || {
                                        deselect_active_profile(profiles, &deselect_controls);
                                    })
                                    .child(glyph_px(
                                        10.0,
                                        || Glyph::Close,
                                        || color(accent_blue_text()),
                                    )),
                            )
                    },
                ))
                .child(
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
                                .text_color(fg_secondary())
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
                    }),
                )
        });
    #[cfg(not(target_arch = "wasm32"))]
    let bar = bar.child(toolbar_update_pill(update_state, update_controls));
    bar.child(toolbar_summary_button(
        rows,
        flow_total,
        filter,
        selected,
        compose,
        seq_builder,
    ))
    .child(saved_filter_menu(profiles, filter_controls))
    .child(toolbar_text_button(tr("清空", "Clear"), {
        let clear = Rc::clone(&clear);
        move || clear()
    }))
    .child(toolbar_menu(
        ca_status,
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
        .px_2p5()
        .gap_1p5()
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
    ca_status: Signal<ca::CaStatus>,
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
    let open = signal(false);
    menu()
        .open(move || Some(open.get()))
        .on_open_change(move |event: &nsh_base::MenuEvent| {
            if let nsh_base::MenuEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(8.0))
        .trigger(more_trigger(ca_status, proxy_on, update_state, open))
        .child(more_menu(
            open,
            ca_status,
            proxy_on,
            install,
            remove,
            toggle_proxy,
            export_har,
            copy_env,
            clear_unpinned,
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
        .px_2p5()
        .gap_1p5()
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
        .px_2p5()
        .gap_1p5()
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
