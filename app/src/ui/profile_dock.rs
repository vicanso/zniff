#[allow(unused_imports)]
use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn profile_dock(
    profiles: ProfileSignals,
    draft_host: Signal<String>,
    draft_path: Signal<String>,
    draft_op: Signal<String>,
    draft_from: Signal<String>,
    draft_to: Signal<String>,
    draft_modes: QuickRuleModeSignals,
    draft_kind: Signal<u8>,
    import_open: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let rule_name = profiles.name;
    let filter = profiles.filter;
    let bypass_text = profiles.bypass;
    let rules_text = profiles.rules;
    let upstream_text = profiles.upstream;
    let rules_on = profiles.rules_on;
    let breaks_on = profiles.breaks_on;
    let show_tunnels = profiles.show_tunnels;
    let saved = profiles.saved;
    let history = profiles.history;
    let preset_controls = controls.clone();
    let history_controls = controls.clone();
    let blank_controls = controls.clone();
    let export_controls = controls.clone();
    let import_controls = controls.clone();
    let import_panel_controls = controls.clone();
    let save_controls = controls;
    let dropdown_open = signal(false);
    let history_open = signal(false);
    let import_text = signal(String::new());
    let matched_name = move || {
        saved.with(|items| {
            matched_saved_profile(
                items,
                &filter.get(),
                &bypass_text.get(),
                &rules_text.get(),
                &upstream_text.get(),
            )
        })
    };
    // The dropdown follows the name field. Adding a rule changes the live
    // text, so a content match would drop the current setup. Seed the name
    // once from that match, then leave it until 新建配置.
    let shown_name = move || {
        let typed = rule_name.get();
        let typed = typed.trim();
        if !typed.is_empty() {
            return Some(typed.to_string());
        }
        matched_name()
    };
    let adopted = signal(false);
    effect(move || {
        if adopted.get() {
            return;
        }
        let typed = rule_name.get();
        if !typed.trim().is_empty() {
            adopted.set(true);
            return;
        }
        if let Some(name) = matched_name() {
            rule_name.set(name);
            adopted.set(true);
        }
    });
    let deselect_rev = profiles.deselect_rev;
    let seen_deselect_rev = signal(deselect_rev.get_untracked());
    effect(move || {
        let rev = deselect_rev.get();
        if rev == seen_deselect_rev.get_untracked() {
            return;
        }
        seen_deselect_rev.set(rev);
        adopted.set(true);
        dropdown_open.set(false);
        history_open.set(false);
        clear_rule_draft(
            draft_host,
            draft_path,
            draft_op,
            draft_from,
            draft_to,
            draft_modes,
            draft_kind,
        );
    });

    view()
        .w_full()
        .flex_shrink_0()
        .px_3()
        .py_2p5()
        .gap_2()
        .border_t_1()
        .border_t_color(border_subtle())
        .child(when(
            move || import_open.get(),
            move || {
                let confirm_controls = import_panel_controls.clone();
                let ready = move || !import_text.get().trim().is_empty();
                view()
                    .w_full()
                    .p_2p5()
                    .gap_2()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(border_subtle())
                    .bg(bg_canvas())
                    .child(
                        view()
                            .w_full()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                text(tr(
                                    "粘贴 TOML 配置导入（支持 [[profiles]] 或单套配置）",
                                    "Paste TOML setup to import (supports [[profiles]] or single setup)",
                                ))
                                .flex_1()
                                .min_w(px(0.0))
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Tail)
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(fg_muted()),
                            )
                            .child(
                                view()
                                    .flex_shrink_0()
                                    .flex_row()
                                    .items_center()
                                    .gap_1p5()
                                    .child(when(
                                        move || !import_text.get().is_empty(),
                                        move || {
                                            view()
                                                .h(px(24.0))
                                                .px_2()
                                                .rounded(px(6.0))
                                                .border_1()
                                                .border_color(border_control())
                                                .bg(bg_control())
                                                .hover(|style| style.bg(bg_control_hover()))
                                                .active(|style| style.opacity(0.8))
                                                .flex_row()
                                                .items_center()
                                                .cursor(Cursor::Pointer)
                                                .on_press(move || import_text.set(String::new()))
                                                .child(
                                                    text(tr("清空", "Clear"))
                                                        .text_size(px(11.0))
                                                        .text_color(fg_secondary()),
                                                )
                                        },
                                    ))
                                    .child(
                                        view()
                                            .h(px(24.0))
                                            .px_2p5()
                                            .rounded(px(6.0))
                                            .border_1()
                                            .border_color(border_control())
                                            .bg(bg_control())
                                            .hover(|style| style.bg(bg_control_hover()))
                                            .active(|style| style.opacity(0.8))
                                            .flex_row()
                                            .items_center()
                                            .cursor(Cursor::Pointer)
                                            .on_press(move || {
                                                import_text.set(String::new());
                                                import_open.set(false);
                                            })
                                            .child(
                                                text(tr("取消", "Cancel"))
                                                    .text_size(px(11.0))
                                                    .text_color(fg_secondary()),
                                            ),
                                    )
                                    .child(
                                        view()
                                            .h(px(24.0))
                                            .px_2p5()
                                            .rounded(px(6.0))
                                            .border_1()
                                            .border_color(move || {
                                                if ready() {
                                                    accent_blue()
                                                } else {
                                                    border_control()
                                                }
                                            })
                                            .bg(move || {
                                                if ready() {
                                                    accent_blue()
                                                } else {
                                                    bg_control()
                                                }
                                            })
                                            .hover(|style| {
                                                style.bg(move || {
                                                    if ready() {
                                                        accent_blue_hover()
                                                    } else {
                                                        bg_control()
                                                    }
                                                })
                                            })
                                            .active(|style| {
                                                style.opacity(move || {
                                                    if ready() { 0.85_f32 } else { 1.0_f32 }
                                                })
                                            })
                                            .flex_row()
                                            .items_center()
                                            .cursor(Cursor::Pointer)
                                            .disabled(move || !ready())
                                            .disabled_style(|style| style.cursor(Cursor::Default))
                                            .on_press(move || {
                                                if !ready() {
                                                    return;
                                                }
                                                if import_profiles_text(
                                                    &confirm_controls,
                                                    &import_text.get(),
                                                    profiles,
                                                ) {
                                                    clear_rule_draft(
                                                        draft_host,
                                                        draft_path,
                                                        draft_op,
                                                        draft_from,
                                                        draft_to,
                                                        draft_modes,
                                                        draft_kind,
                                                    );
                                                    import_text.set(String::new());
                                                    import_open.set(false);
                                                }
                                            })
                                            .child(
                                                text(tr("确认导入", "Confirm Import"))
                                                    .text_size(px(11.0))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(move || {
                                                        if ready() {
                                                            AdaptiveColor::rgb(0xffffff, 0xffffff)
                                                        } else {
                                                            fg_muted()
                                                        }
                                                    }),
                                            ),
                                    ),
                            ),
                    )
                    .child(
                        guard_culled_pointer_events(view())
                            .w_full()
                            .h(px(116.0))
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(border_control())
                            .bg(bg_code())
                            .overflow_hidden()
                            .child(
                                editor()
                                    .value(move || SharedString::from(import_text.get()))
                                    .auto_focus(true)
                                    .language("toml")
                                    .line_numbers(true)
                                    .folding(false)
                                    .soft_wrap(true)
                                    .font_size(Points(12.0))
                                    .line_height(Points(18.0))
                                    .scroll_beyond_last_line(false)
                                    .placeholder(move || {
                                        SharedString::from(tr(
                                            "在此粘贴导出的 [[profiles]] TOML 内容...",
                                            "Paste exported [[profiles]] TOML content here...",
                                        )())
                                    })
                                    .on_change(move |text: &SharedString| {
                                        import_text.set(text.to_string());
                                    })
                                    .size_full(),
                            ),
                    )
            },
        ))
        .child(
            view()
                .w_full()
                .h(px(30.0))
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(
                    menu()
                        .open(move || Some(dropdown_open.get() && !saved.with(Vec::is_empty)))
                        .on_open_change(move |event: &nsh_base::MenuEvent| {
                            if let nsh_base::MenuEvent::OpenChange(next, _) = event {
                                dropdown_open.set(*next && !saved.with(Vec::is_empty));
                            }
                        })
                        .placement(Placement::Top)
                        .align(Align::Start)
                        .offset(px(6.0))
                        .margin(px(8.0))
                        .trigger(
                            view()
                                .min_w(px(132.0))
                                .max_w(px(176.0))
                                .h(px(30.0))
                                .px_2p5()
                                .flex_shrink_0()
                                .rounded(px(7.0))
                                .border_1()
                                .border_color(move || {
                                    if dropdown_open.get() || shown_name().is_some() {
                                        accent_blue_border()
                                    } else {
                                        border_control()
                                    }
                                })
                                .bg(move || {
                                    if dropdown_open.get() || shown_name().is_some() {
                                        accent_blue_soft()
                                    } else {
                                        bg_control()
                                    }
                                })
                                .hover(|style| {
                                    style.bg(move || {
                                        if saved.with(Vec::is_empty) {
                                            bg_control()
                                        } else if dropdown_open.get() || shown_name().is_some() {
                                            accent_blue_soft_hover()
                                        } else {
                                            bg_control_hover()
                                        }
                                    })
                                })
                                .active(|style| {
                                    style.opacity(move || {
                                        if saved.with(Vec::is_empty) {
                                            1.0_f32
                                        } else {
                                            0.8_f32
                                        }
                                    })
                                })
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .cursor(Cursor::Pointer)
                                .disabled(move || saved.with(Vec::is_empty))
                                .disabled_style(|style| style.cursor(Cursor::Default))
                                .child(
                                    view()
                                        .w(px(6.0))
                                        .h(px(6.0))
                                        .rounded_full()
                                        .flex_shrink_0()
                                        .bg(move || {
                                            if shown_name().is_some() {
                                                accent_blue()
                                            } else {
                                                fg_muted()
                                            }
                                        }),
                                )
                                .child(
                                    text({
                                        let langs = ui_langs();
                                        move || -> SharedString {
                                            let lang = ui_lang(langs);
                                            if let Some(name) = shown_name() {
                                                name.into()
                                            } else if saved.with(Vec::is_empty) {
                                                t(lang, "暂无已保存配置", "No saved setups").into()
                                            } else {
                                                t(lang, "选择已保存配置", "Saved setups").into()
                                            }
                                        }
                                    })
                                    .flex_auto()
                                    .min_w(px(0.0))
                                    .number_of_lines(1)
                                    .text_overflow(EllipsizeMode::Tail)
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(move || {
                                        if shown_name().is_some() {
                                            accent_blue_text()
                                        } else if saved.with(Vec::is_empty) {
                                            fg_muted()
                                        } else {
                                            fg_secondary()
                                        }
                                    }),
                                )
                                .child(when(
                                    move || !saved.with(Vec::is_empty),
                                    move || {
                                        view()
                                            .h(px(16.0))
                                            .px_1()
                                            .rounded(px(4.0))
                                            .flex_shrink_0()
                                            .flex_row()
                                            .items_center()
                                            .bg(bg_card())
                                            .child(
                                                mono(move || saved.with(Vec::len).to_string())
                                                    .tabular_nums()
                                                    .text_size(px(10.5))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(fg_muted()),
                                            )
                                    },
                                ))
                                .child(glyph_px(12.0, || Glyph::Chevron, || color(fg_muted()))),
                        )
                        .child(
                            view()
                                .w(px(360.0))
                                .max_w(move || px((window().width.0 - 24.0).clamp(220.0, 360.0)))
                                .max_h(move || px((window().height.0 - 96.0).clamp(140.0, 220.0)))
                                .overflow_hidden()
                                .rounded(px(10.0))
                                .border_1()
                                .border_color(border_strong())
                                .bg(bg_card())
                                .shadow_md()
                                .child(
                                    scroll()
                                        .w_full()
                                        .max_h(move || {
                                            px((window().height.0 - 98.0).clamp(138.0, 218.0))
                                        })
                                        .child(
                                            view()
                                                .w_full()
                                                .p_1()
                                                .gap_0p5()
                                                .child(when(
                                                    move || shown_name().is_some(),
                                                    {
                                                        let clear_controls =
                                                            preset_controls.clone();
                                                        move || {
                                                            let clear_controls =
                                                                clear_controls.clone();
                                                            view()
                                                                .w_full()
                                                                .gap_1()
                                                                .child(
                                                                    menu_item()
                                                                        .w_full()
                                                                        .h(px(28.0))
                                                                        .px_2()
                                                                        .rounded(px(6.0))
                                                                        .flex_row()
                                                                        .items_center()
                                                                        .gap_2()
                                                                        .hover(|style| {
                                                                            style.bg(
                                                                                bg_control_hover(),
                                                                            )
                                                                        })
                                                                        .active(|style| {
                                                                            style.opacity(0.8)
                                                                        })
                                                                        .cursor(Cursor::Pointer)
                                                                        .on_select(move || {
                                                                            adopted.set(true);
                                                                            deselect_active_profile(
                                                                                profiles,
                                                                                &clear_controls,
                                                                            );
                                                                            dropdown_open
                                                                                .set(false);
                                                                        })
                                                                        .child(glyph_px(
                                                                            11.0,
                                                                            || Glyph::Close,
                                                                            || color(fg_muted()),
                                                                        ))
                                                                        .child(
                                                                            text(tr(
                                                                                "取消选择当前配置",
                                                                                "Deselect current setup",
                                                                            ))
                                                                            .flex_1()
                                                                            .min_w(px(0.0))
                                                                            .number_of_lines(1)
                                                                            .text_overflow(
                                                                                EllipsizeMode::Tail,
                                                                            )
                                                                            .text_size(px(11.5))
                                                                            .font_weight(
                                                                                FontWeight::MEDIUM,
                                                                            )
                                                                            .text_color(
                                                                                fg_secondary(),
                                                                            ),
                                                                        ),
                                                                )
                                                                .child(
                                                                    view()
                                                                        .w_full()
                                                                        .h(px(1.0))
                                                                        .my_0p5()
                                                                        .bg(border_subtle()),
                                                                )
                                                        }
                                                    },
                                                ))
                                                .child(each(
                                                saved,
                                                |item| {
                                                    format!(
                                                        "{}\t{}\t{}\t{}\t{}",
                                                        item.name,
                                                        item.filter,
                                                        item.bypass_hosts,
                                                        item.rules,
                                                        item.upstream_proxy
                                                    )
                                                },
                                                move |item, _| {
                                                    let apply = item;
                                                    let remove = item;
                                                    let mark = item;
                                                    let display_name = item;
                                                    let display_rules = item;
                                                    let preset_controls = preset_controls.clone();
                                                    let remove_controls = preset_controls.clone();
                                                    let filter_apply = filter;
                                                    let bypass_apply = bypass_text;
                                                    let rules_apply = rules_text;
                                                    let upstream_apply = upstream_text;
                                                    let rules_on_apply = rules_on;
                                                    let breaks_on_apply = breaks_on;
                                                    let tunnels_apply = show_tunnels;
                                                    let is_active = move || {
                                                        let typed = rule_name.get();
                                                        let typed = typed.trim();
                                                        if !typed.is_empty() {
                                                            return mark.with(|preset| {
                                                                preset
                                                                    .name
                                                                    .eq_ignore_ascii_case(typed)
                                                            });
                                                        }
                                                        let f = filter.get();
                                                        let b = bypass_text.get();
                                                        let r = rules_text.get();
                                                        let u = upstream_text.get();
                                                        if f.trim().is_empty()
                                                            && b.trim().is_empty()
                                                            && r.trim().is_empty()
                                                            && u.trim().is_empty()
                                                        {
                                                            return false;
                                                        }
                                                        mark.with(|preset| {
                                                            preset.filter.trim() == f.trim()
                                                                && preset.bypass_hosts.trim()
                                                                    == b.trim()
                                                                && preset.rules.trim() == r.trim()
                                                                && preset.upstream_proxy.trim()
                                                                    == u.trim()
                                                        })
                                                    };
                                                    view()
                                                        .h(px(30.0))
                                                        .gap_1()
                                                        .flex_row()
                                                        .items_center()
                                                        .child(
                                                            menu_item()
                                                                .flex_1()
                                                                .min_w(px(0.0))
                                                                .h_full()
                                                                .px_2()
                                                                .rounded(px(6.0))
                                                                .overflow_hidden()
                                                                .flex_row()
                                                                .items_center()
                                                                .gap_2()
                                                                .border_1()
                                                                .border_color(move || {
                                                                    if is_active() {
                                                                        accent_blue_border()
                                                                    } else {
                                                                        AdaptiveColor::TRANSPARENT
                                                                    }
                                                                })
                                                                .bg(move || {
                                                                    if is_active() {
                                                                        accent_blue_soft()
                                                                    } else {
                                                                        AdaptiveColor::TRANSPARENT
                                                                    }
                                                                })
                                                                .hover(|style| {
                                                                    style.bg(move || {
                                                                        if is_active() {
                                                                            accent_blue_soft_hover()
                                                                        } else {
                                                                            bg_control_hover()
                                                                        }
                                                                    })
                                                                })
                                                                .active(|style| style.opacity(0.8))
                                                                .cursor(Cursor::Pointer)
                                                                .on_select(move || {
                                                                    if is_active() {
                                                                        adopted.set(true);
                                                                        deselect_active_profile(
                                                                            profiles,
                                                                            &preset_controls,
                                                                        );
                                                                        dropdown_open.set(false);
                                                                        return;
                                                                    }
                                                                    let preset = apply.with(
                                                                        |preset| preset.clone(),
                                                                    );
                                                                    record_profile_history(
                                                                        &rule_name.get(),
                                                                        &filter_apply.get(),
                                                                        &bypass_apply.get(),
                                                                        &rules_apply.get(),
                                                                        &upstream_apply.get(),
                                                                        rules_on_apply.get(),
                                                                        breaks_on_apply.get(),
                                                                        tunnels_apply.get(),
                                                                        history,
                                                                    );
                                                                    rule_name
                                                                        .set(preset.name.clone());
                                                                    clear_rule_draft(
                                                                        draft_host,
                                                                        draft_path,
                                                                        draft_op,
                                                                        draft_from,
                                                                        draft_to,
                                                                        draft_modes,
                                                                        draft_kind,
                                                                    );
                                                                    apply_saved_profile(
                                                                        &preset,
                                                                        filter_apply,
                                                                        bypass_apply,
                                                                        rules_apply,
                                                                        upstream_apply,
                                                                        rules_on_apply,
                                                                        breaks_on_apply,
                                                                        tunnels_apply,
                                                                        &preset_controls,
                                                                    );
                                                                    record_profile_history(
                                                                        &preset.name,
                                                                        &preset.filter,
                                                                        &preset.bypass_hosts,
                                                                        &preset.rules,
                                                                        &preset.upstream_proxy,
                                                                        preset.rules_enabled,
                                                                        preset.breaks_enabled,
                                                                        preset.show_tunnel_failures,
                                                                        history,
                                                                    );
                                                                })
                                                                .child(
                                                                    view()
                                                                        .w(px(6.0))
                                                                        .h(px(6.0))
                                                                        .rounded_full()
                                                                        .flex_shrink_0()
                                                                        .bg(move || {
                                                                            if is_active() {
                                                                                accent_blue()
                                                                            } else {
                                                                                fg_muted()
                                                                            }
                                                                        }),
                                                                )
                                                                .child(
                                                                    view()
                                                                        .h(px(18.0))
                                                                        .max_w(px(124.0))
                                                                        .px_1p5()
                                                                        .rounded(px(4.0))
                                                                        .flex_shrink_0()
                                                                        .overflow_hidden()
                                                                        .flex_row()
                                                                        .items_center()
                                                                        .bg(accent_blue_soft())
                                                                        .child(
                                                                            text(move || {
                                                                                display_name.with(
                                                                                    |preset| {
                                                                                        preset
                                                                                            .name
                                                                                            .clone()
                                                                                    },
                                                                                )
                                                                            })
                                                                            .max_w(px(112.0))
                                                                            .min_w(px(0.0))
                                                                            .number_of_lines(1)
                                                                            .text_overflow(
                                                                                EllipsizeMode::Tail,
                                                                            )
                                                                            .text_size(px(11.0))
                                                                            .font_weight(
                                                                                FontWeight::MEDIUM,
                                                                            )
                                                                            .text_color(
                                                                                accent_blue_text(),
                                                                            ),
                                                                        ),
                                                                )
                                                                .child(
                                                                    control_text(move || {
                                                                        display_rules
                                                                            .with(profile_preview)
                                                                    })
                                                                    .flex_1()
                                                                    .min_w(px(0.0))
                                                                    .number_of_lines(1)
                                                                    .text_overflow(
                                                                        EllipsizeMode::Tail,
                                                                    )
                                                                    .text_color(fg_secondary()),
                                                                )
                                                                .child(when(is_active, || {
                                                                    text(tr(
                                                                        "点击取消",
                                                                        "Click to clear",
                                                                    ))
                                                                    .flex_shrink_0()
                                                                    .text_size(px(10.5))
                                                                    .font_weight(FontWeight::MEDIUM)
                                                                    .text_color(accent_blue_text())
                                                                })),
                                                        )
                                                        .child(
                                                            view()
                                                                .w(px(26.0))
                                                                .h(px(26.0))
                                                                .flex_shrink_0()
                                                                .flex_row()
                                                                .items_center()
                                                                .justify_center()
                                                                .rounded(px(6.0))
                                                                .cursor(Cursor::Pointer)
                                                                .hover(|style| {
                                                                    style.bg(bg_control_hover())
                                                                })
                                                                .active(|style| style.opacity(0.75))
                                                                .label(tr(
                                                                    "删除配置",
                                                                    "Delete setup",
                                                                ))
                                                                .tooltip(label_tip(
                                                                    "删除配置",
                                                                    "Delete setup",
                                                                ))
                                                                .on_press(move || {
                                                                    let target = remove.with(
                                                                        |preset| preset.clone(),
                                                                    );
                                                                    adopted.set(true);
                                                                    drop_saved_profile(
                                                                        &target,
                                                                        rule_name,
                                                                        saved,
                                                                        history,
                                                                        &remove_controls,
                                                                    );
                                                                    if saved.with(Vec::is_empty) {
                                                                        dropdown_open.set(false);
                                                                    }
                                                                })
                                                                .child(glyph_px(
                                                                    12.0,
                                                                    || Glyph::Trash,
                                                                    || color(fg_muted()),
                                                                )),
                                                        )
                                                },
                                            )),
                                        ),
                                ),
                        ),
                )
                .child(
                    menu()
                        .open(move || Some(history_open.get() && !history.with(Vec::is_empty)))
                        .on_open_change(move |event: &nsh_base::MenuEvent| {
                            if let nsh_base::MenuEvent::OpenChange(next, _) = event {
                                history_open.set(*next && !history.with(Vec::is_empty));
                                if *next {
                                    dropdown_open.set(false);
                                }
                            }
                        })
                        .placement(Placement::Top)
                        .align(Align::Start)
                        .offset(px(6.0))
                        .margin(px(8.0))
                        .trigger(
                            view()
                                .h(px(30.0))
                                .px_2()
                                .flex_shrink_0()
                                .rounded(px(7.0))
                                .border_1()
                                .border_color(move || {
                                    if history_open.get() {
                                        accent_blue_border()
                                    } else {
                                        border_control()
                                    }
                                })
                                .bg(move || {
                                    if history_open.get() {
                                        accent_blue_soft()
                                    } else {
                                        bg_control()
                                    }
                                })
                                .hover(|style| {
                                    style.bg(move || {
                                        if history.with(Vec::is_empty) {
                                            bg_control()
                                        } else if history_open.get() {
                                            accent_blue_soft_hover()
                                        } else {
                                            bg_control_hover()
                                        }
                                    })
                                })
                                .active(|style| {
                                    style.opacity(move || {
                                        if history.with(Vec::is_empty) {
                                            1.0_f32
                                        } else {
                                            0.8_f32
                                        }
                                    })
                                })
                                .flex_row()
                                .items_center()
                                .gap_1()
                                .cursor(Cursor::Pointer)
                                .disabled(move || history.with(Vec::is_empty))
                                .disabled_style(|style| style.cursor(Cursor::Default))
                                .tooltip(label_tip(
                                    "最近 10 个配置历史版本，点击可恢复",
                                    "Last 10 setup versions, click to restore",
                                ))
                                .child(glyph_px(
                                    12.5,
                                    || Glyph::History,
                                    move || {
                                        color(if history_open.get() {
                                            accent_blue_text()
                                        } else if history.with(Vec::is_empty) {
                                            fg_muted()
                                        } else {
                                            fg_secondary()
                                        })
                                    },
                                ))
                                .child(
                                    text(tr("历史", "History"))
                                        .text_size(px(11.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(move || {
                                            if history_open.get() {
                                                accent_blue_text()
                                            } else if history.with(Vec::is_empty) {
                                                fg_muted()
                                            } else {
                                                fg_secondary()
                                            }
                                        }),
                                )
                                .child(when(
                                    move || !history.with(Vec::is_empty),
                                    move || {
                                        view()
                                            .h(px(16.0))
                                            .px_1()
                                            .rounded(px(4.0))
                                            .flex_shrink_0()
                                            .flex_row()
                                            .items_center()
                                            .bg(bg_card())
                                            .child(
                                                mono(move || history.with(Vec::len).to_string())
                                                    .tabular_nums()
                                                    .text_size(px(10.5))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(fg_muted()),
                                            )
                                    },
                                )),
                        )
                        .child(
                            view()
                                .w(px(380.0))
                                .max_w(move || px((window().width.0 - 24.0).clamp(220.0, 380.0)))
                                .max_h(move || px((window().height.0 - 96.0).clamp(140.0, 240.0)))
                                .overflow_hidden()
                                .rounded(px(10.0))
                                .border_1()
                                .border_color(border_strong())
                                .bg(bg_card())
                                .shadow_md()
                                .child(
                                    scroll()
                                        .w_full()
                                        .max_h(move || {
                                            px((window().height.0 - 98.0).clamp(138.0, 238.0))
                                        })
                                        .child(
                                            view().w_full().p_1().gap_0p5().child(each(
                                                history,
                                                |item| {
                                                    format!(
                                                        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                                                        item.name,
                                                        item.filter,
                                                        item.bypass_hosts,
                                                        item.rules,
                                                        item.upstream_proxy,
                                                        item.rules_enabled,
                                                        item.breaks_enabled,
                                                        item.show_tunnel_failures
                                                    )
                                                },
                                                move |item, index| {
                                                    let apply = item;
                                                    let mark = item;
                                                    let display_name = item;
                                                    let display_rules = item;
                                                    let history_controls =
                                                        history_controls.clone();
                                                    let filter_apply = filter;
                                                    let bypass_apply = bypass_text;
                                                    let rules_apply = rules_text;
                                                    let upstream_apply = upstream_text;
                                                    let rules_on_apply = rules_on;
                                                    let breaks_on_apply = breaks_on;
                                                    let tunnels_apply = show_tunnels;
                                                    let is_current = move || {
                                                        mark.with(|preset| {
                                                            preset.filter.trim()
                                                                == filter.get().trim()
                                                                && preset.bypass_hosts.trim()
                                                                    == bypass_text.get().trim()
                                                                && preset.rules.trim()
                                                                    == rules_text.get().trim()
                                                                && preset.upstream_proxy.trim()
                                                                    == upstream_text.get().trim()
                                                                && preset.rules_enabled
                                                                    == rules_on.get()
                                                                && preset.breaks_enabled
                                                                    == breaks_on.get()
                                                                && preset.show_tunnel_failures
                                                                    == show_tunnels.get()
                                                        })
                                                    };
                                                    menu_item()
                                                        .w_full()
                                                        .h(px(30.0))
                                                        .px_2()
                                                        .rounded(px(6.0))
                                                        .overflow_hidden()
                                                        .flex_row()
                                                        .items_center()
                                                        .gap_1p5()
                                                        .border_1()
                                                        .border_color(move || {
                                                            if is_current() {
                                                                accent_blue_border()
                                                            } else {
                                                                AdaptiveColor::TRANSPARENT
                                                            }
                                                        })
                                                        .bg(move || {
                                                            if is_current() {
                                                                accent_blue_soft()
                                                            } else {
                                                                AdaptiveColor::TRANSPARENT
                                                            }
                                                        })
                                                        .hover(|style| {
                                                            style.bg(move || {
                                                                if is_current() {
                                                                    accent_blue_soft_hover()
                                                                } else {
                                                                    bg_control_hover()
                                                                }
                                                            })
                                                        })
                                                        .active(|style| style.opacity(0.8))
                                                        .cursor(Cursor::Pointer)
                                                        .on_select(move || {
                                                            let preset = apply
                                                                .with(|preset| preset.clone());
                                                            record_profile_history(
                                                                &rule_name.get(),
                                                                &filter_apply.get(),
                                                                &bypass_apply.get(),
                                                                &rules_apply.get(),
                                                                &upstream_apply.get(),
                                                                rules_on_apply.get(),
                                                                breaks_on_apply.get(),
                                                                tunnels_apply.get(),
                                                                history,
                                                            );
                                                            rule_name.set(preset.name.clone());
                                                            clear_rule_draft(
                                                                draft_host,
                                                                draft_path,
                                                                draft_op,
                                                                draft_from,
                                                                draft_to,
                                                                draft_modes,
                                                                draft_kind,
                                                            );
                                                            apply_saved_profile(
                                                                &preset,
                                                                filter_apply,
                                                                bypass_apply,
                                                                rules_apply,
                                                                upstream_apply,
                                                                rules_on_apply,
                                                                breaks_on_apply,
                                                                tunnels_apply,
                                                                &history_controls,
                                                            );
                                                            set_status(
                                                                &history_controls,
                                                                StatusMsg::ProfileRestored(
                                                                    preset.name.clone(),
                                                                ),
                                                            );
                                                        })
                                                        .child(
                                                            mono(move || {
                                                                format!("#{}", index.get() + 1)
                                                            })
                                                            .tabular_nums()
                                                            .text_size(px(10.5))
                                                            .text_color(fg_muted()),
                                                        )
                                                        .child(
                                                            view()
                                                                .h(px(18.0))
                                                                .max_w(px(120.0))
                                                                .px_1p5()
                                                                .rounded(px(4.0))
                                                                .flex_shrink_0()
                                                                .overflow_hidden()
                                                                .flex_row()
                                                                .items_center()
                                                                .bg(accent_blue_soft())
                                                                .child(
                                                                    text(move || {
                                                                        display_name.with(
                                                                            |preset| {
                                                                                preset.name.clone()
                                                                            },
                                                                        )
                                                                    })
                                                                    .max_w(px(108.0))
                                                                    .min_w(px(0.0))
                                                                    .number_of_lines(1)
                                                                    .text_overflow(
                                                                        EllipsizeMode::Tail,
                                                                    )
                                                                    .text_size(px(11.0))
                                                                    .font_weight(FontWeight::MEDIUM)
                                                                    .text_color(accent_blue_text()),
                                                                ),
                                                        )
                                                        .child(
                                                            control_text(move || {
                                                                display_rules.with(profile_preview)
                                                            })
                                                            .flex_1()
                                                            .min_w(px(0.0))
                                                            .number_of_lines(1)
                                                            .text_overflow(EllipsizeMode::Tail)
                                                            .text_color(fg_secondary()),
                                                        )
                                                },
                                            )),
                                        ),
                                ),
                        ),
                )
                .child(view().flex_1().min_w(px(0.0)).child(side_input(
                    rule_name,
                    tr("配置名称 (可选，同名覆盖)", "Setup name (optional)"),
                    30.0,
                    false,
                    move |text| rule_name.set(text.to_string()),
                )))
                .child(
                    view()
                        .h(px(30.0))
                        .px_2p5()
                        .flex_shrink_0()
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(border_control())
                        .bg(bg_control())
                        .hover(|style| style.bg(bg_control_hover()))
                        .active(|style| style.opacity(0.8))
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .cursor(Cursor::Pointer)
                        .tooltip(label_tip(
                            "导出配置 TOML 到剪贴板",
                            "Copy setups as TOML to clipboard",
                        ))
                        .on_press(move || {
                            dropdown_open.set(false);
                            let kind_now = draft_kind.get();
                            let rules_now = draft_rules_with_op(
                                &rules_text.get(),
                                &draft_host.get(),
                                &draft_path.get(),
                                &draft_op.get(),
                                &draft_from.get(),
                                &draft_to.get(),
                                draft_modes.mode_for(kind_now),
                                kind_now,
                            );
                            let profiles = profiles_for_export(
                                &saved.get(),
                                &rule_name.get(),
                                &filter.get(),
                                &bypass_text.get(),
                                &rules_now,
                                &upstream_text.get(),
                                rules_on.get(),
                                breaks_on.get(),
                                show_tunnels.get(),
                            );
                            export_profiles_clipboard(&export_controls, &profiles);
                        })
                        .child(
                            text(tr("导出", "Export"))
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg_secondary()),
                        ),
                )
                .child(
                    view()
                        .h(px(30.0))
                        .px_2p5()
                        .flex_shrink_0()
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(move || {
                            if import_open.get() {
                                accent_blue_border()
                            } else {
                                border_control()
                            }
                        })
                        .bg(move || {
                            if import_open.get() {
                                accent_blue_soft()
                            } else {
                                bg_control()
                            }
                        })
                        .hover(|style| {
                            style.bg(move || {
                                if import_open.get() {
                                    accent_blue_soft_hover()
                                } else {
                                    bg_control_hover()
                                }
                            })
                        })
                        .active(|style| style.opacity(0.8))
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .cursor(Cursor::Pointer)
                        .tooltip(label_tip(
                            "粘贴 TOML 配置文本导入（或拖拽 .toml 到窗口）",
                            "Paste TOML setup text to import (or drop .toml)",
                        ))
                        .on_press(move || {
                            dropdown_open.set(false);
                            if !import_open.get() {
                                import_open.set(true);
                            } else if import_text.get().trim().is_empty() {
                                import_open.set(false);
                            } else if import_profiles_text(
                                &import_controls,
                                &import_text.get(),
                                profiles,
                            ) {
                                clear_rule_draft(
                                    draft_host,
                                    draft_path,
                                    draft_op,
                                    draft_from,
                                    draft_to,
                                    draft_modes,
                                    draft_kind,
                                );
                                import_text.set(String::new());
                                import_open.set(false);
                            }
                        })
                        .child(
                            text(tr("导入", "Import"))
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || {
                                    if import_open.get() {
                                        accent_blue_text()
                                    } else {
                                        fg_secondary()
                                    }
                                }),
                        ),
                )
                .child(
                    view()
                        .h(px(30.0))
                        .px_3()
                        .flex_shrink_0()
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(border_control())
                        .bg(bg_control())
                        .hover(|style| style.bg(bg_control_hover()))
                        .active(|style| style.opacity(0.8))
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .cursor(Cursor::Pointer)
                        .on_press(move || {
                            dropdown_open.set(false);
                            history_open.set(false);
                            adopted.set(true);
                            record_profile_history(
                                &rule_name.get(),
                                &filter.get(),
                                &bypass_text.get(),
                                &rules_text.get(),
                                &upstream_text.get(),
                                rules_on.get(),
                                breaks_on.get(),
                                show_tunnels.get(),
                                history,
                            );
                            blank_working_setup(
                                rule_name,
                                filter,
                                bypass_text,
                                rules_text,
                                upstream_text,
                                rules_on,
                                breaks_on,
                                show_tunnels,
                                draft_host,
                                draft_path,
                                draft_op,
                                draft_from,
                                draft_to,
                                draft_modes,
                                draft_kind,
                                &blank_controls,
                            );
                        })
                        .child(
                            text(tr("新建配置", "New setup"))
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg_secondary()),
                        ),
                )
                .child({
                    let save_ready = computed(move || {
                        let kind_now = draft_kind.get();
                        saved.with(|saved_list| {
                            profile_save_ready(
                                saved_list,
                                &rule_name.get(),
                                &filter.get(),
                                &bypass_text.get(),
                                &rules_text.get(),
                                &upstream_text.get(),
                                rules_on.get(),
                                breaks_on.get(),
                                show_tunnels.get(),
                                &draft_host.get(),
                                &draft_path.get(),
                                &draft_op.get(),
                                &draft_from.get(),
                                &draft_to.get(),
                                draft_modes.mode_for(kind_now),
                                kind_now,
                            )
                        })
                    });
                    let ready_text = move || save_ready.get();
                    view()
                        .h(px(30.0))
                        .px_3p5()
                        .flex_shrink_0()
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(move || {
                            if save_ready.get() {
                                accent_blue()
                            } else {
                                border_control()
                            }
                        })
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .bg(move || {
                            if save_ready.get() {
                                accent_blue()
                            } else {
                                bg_control()
                            }
                        })
                        .hover(|style| {
                            style.bg(move || {
                                if save_ready.get() {
                                    accent_blue_hover()
                                } else {
                                    bg_control()
                                }
                            })
                        })
                        .active(|style| {
                            style.opacity(move || {
                                if save_ready.get() { 0.85_f32 } else { 1.0_f32 }
                            })
                        })
                        .cursor(Cursor::Pointer)
                        .disabled(move || !save_ready.get())
                        .disabled_style(|style| style.cursor(Cursor::Default))
                        .on_press(move || {
                            if !save_ready.get() {
                                return;
                            }
                            dropdown_open.set(false);
                            // A row the user actually filled is part of this
                            // setup. An untouched row must not become `*`.
                            let host = draft_host.get();
                            let path = draft_path.get();
                            let op = draft_op.get();
                            let from = draft_from.get();
                            let to = draft_to.get();
                            let kind_now = draft_kind.get();
                            let current = rules_text.get();
                            let rules_now = draft_rules_with_op(
                                &current,
                                &host,
                                &path,
                                &op,
                                &from,
                                &to,
                                draft_modes.mode_for(kind_now),
                                kind_now,
                            );
                            if rules_now != current {
                                rules_text.set(rules_now.clone());
                                save_controls.hub.set_rules(&rules_now);
                                draft_host.set(String::new());
                                draft_path.set(String::new());
                                draft_op.set(String::new());
                                draft_from.set(String::new());
                                draft_to.set(String::new());
                            }
                            let saved_name = resolve_profile_name(
                                &rule_name.get(),
                                filter.get().trim(),
                                rules_now.trim(),
                            );
                            rule_name.set(saved_name.clone());
                            match store_saved_profile(
                                &saved_name,
                                &filter.get(),
                                &bypass_text.get(),
                                &rules_now,
                                &upstream_text.get(),
                                rules_on.get(),
                                breaks_on.get(),
                                show_tunnels.get(),
                                saved,
                                history,
                            ) {
                                Ok(()) => {
                                    set_status(&save_controls, StatusMsg::ProfileSaved(saved_name));
                                }
                                Err(err) => {
                                    set_status(
                                        &save_controls,
                                        StatusMsg::PrefUnsaved {
                                            name: PrefName::Profiles,
                                            err: err.to_string(),
                                        },
                                    );
                                }
                            }
                        })
                        .child(
                            text(tr("保存", "Save"))
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || {
                                    if ready_text() {
                                        AdaptiveColor::rgb(0xffffff, 0xffffff)
                                    } else {
                                        fg_muted()
                                    }
                                }),
                        )
                }),
        )
}
