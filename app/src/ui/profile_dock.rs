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
    let blank_controls = controls.clone();
    let export_controls = controls.clone();
    let import_controls = controls.clone();
    let import_panel_controls = controls.clone();
    let save_controls = controls.clone();
    let dropdown_open = signal(false);
    let history_open = signal(false);
    let import_text = signal(String::new());
    let draft = QuickRuleDraftSignals {
        kind: draft_kind,
        host: draft_host,
        path: draft_path,
        op: draft_op,
        from: draft_from,
        to: draft_to,
        modes: draft_modes,
    };
    let menu_ctx = ProfileDockMenuCtx {
        profiles,
        draft,
        adopted: signal(false),
        dropdown_open,
        history_open,
        controls,
    };
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
    let adopted = menu_ctx.adopted;
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
                    .rounded(px(RADIUS_MD))
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
                                    "粘贴 TOML / Whistle / Charles XML 导入",
                                    "Paste TOML, Whistle, or Charles XML to import",
                                ))
                                .flex_1()
                                .min_w(px(0.0))
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Tail)
                                .text_size(px(TEXT_SM))
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
                                                .role(AccessibilityRole::Button)
                                                .h(px(ControlSize::Xs.height()))
                                                .px_2()
                                                .rounded(px(RADIUS_SM))
                                                .border_1()
                                                .border_color(border_control())
                                                .bg(bg_control())
                                                .hover(|style| style.bg(bg_control_hover()))
                                                .active(|style| style.bg(bg_control_active()))
                                                .transition_colors()
                                                .duration(MOTION_INSTANT_MS)
                                                .ease_out()
                                                .focus_visible(apply_focus_ring)
                                                .flex_row()
                                                .items_center()
                                                .on_press(move || import_text.set(String::new()))
                                                .child(
                                                    text(tr("清空", "Clear"))
                                                        .text_size(px(TEXT_XS))
                                                        .text_color(fg_secondary()),
                                                )
                                        },
                                    ))
                                    .child(
                                        view()
                                            .role(AccessibilityRole::Button)
                                            .h(px(ControlSize::Xs.height()))
                                            .px_2p5()
                                            .rounded(px(RADIUS_SM))
                                            .border_1()
                                            .border_color(border_control())
                                            .bg(bg_control())
                                            .hover(|style| style.bg(bg_control_hover()))
                                            .active(|style| style.bg(bg_control_active()))
                                            .transition_colors()
                                            .duration(MOTION_INSTANT_MS)
                                            .ease_out()
                                            .focus_visible(apply_focus_ring)
                                            .flex_row()
                                            .items_center()
                                            .on_press(move || {
                                                import_text.set(String::new());
                                                import_open.set(false);
                                            })
                                            .child(
                                                text(tr("取消", "Cancel"))
                                                    .text_size(px(TEXT_XS))
                                                    .text_color(fg_secondary()),
                                            ),
                                    )
                                    .child(
                                        view()
                                            .role(AccessibilityRole::Button)
                                            .h(px(ControlSize::Xs.height()))
                                            .px_2p5()
                                            .rounded(px(RADIUS_SM))
                                            .border_1()
                                            .border_color(move || {
                                                if ready() {
                                                    accent_blue()
                                                } else {
                                                    border_control()
                                                }
                                            })
                                            .bg(
                                                move || {
                                                    if ready() {
                                                        accent_blue()
                                                    } else {
                                                        bg_control()
                                                    }
                                                },
                                            )
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
                                                style.bg(move || {
                                                    if ready() {
                                                        accent_blue_active()
                                                    } else {
                                                        bg_control()
                                                    }
                                                })
                                            })
                                            .transition_colors()
                                            .duration(MOTION_INSTANT_MS)
                                            .ease_out()
                                            .focus_visible(apply_focus_ring)
                                            .flex_row()
                                            .items_center()
                                            .disabled(move || !ready())
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
                                                    .text_size(px(TEXT_XS))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(move || {
                                                        if ready() {
                                                            fg_on_accent()
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
                            .rounded(px(RADIUS_MD))
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
                                            "粘贴 TOML 配置、Whistle 规则或 Charles XML...",
                                            "Paste TOML setup, Whistle rules, or Charles XML...",
                                        )(
                                        ))
                                    })
                                    .on_change(move |text: &SharedString| {
                                        import_text.set(text.to_string());
                                    })
                                    .size_full(),
                            ),
                    )
                    .child(when(
                        {
                            let langs = ui_langs();
                            move || {
                                preview_rule_import(&import_text.get(), ui_lang(langs)).is_some()
                            }
                        },
                        {
                            let langs = ui_langs();
                            move || {
                                text(move || {
                                    SharedString::from(
                                        preview_rule_import(&import_text.get(), ui_lang(langs))
                                            .unwrap_or_default(),
                                    )
                                })
                                .text_size(px(TEXT_XS))
                                .text_color(accent_blue_text())
                            }
                        },
                    ))
            },
        ))
        .child(
            view()
                .w_full()
                .h(px(CONTROL_H))
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(profile_saved_menu(menu_ctx.clone()))
                .child(profile_history_menu(menu_ctx))
                .child(view().flex_1().min_w(px(0.0)).child(side_input(
                    rule_name,
                    tr("配置名称 (可选，同名覆盖)", "Setup name (optional)"),
                    CONTROL_H,
                    false,
                    move |text| rule_name.set(text.to_string()),
                )))
                .child(
                    view()
                        .role(AccessibilityRole::Button)
                        .h(px(CONTROL_H))
                        .px_2p5()
                        .flex_shrink_0()
                        .rounded(px(RADIUS_MD))
                        .border_1()
                        .border_color(border_control())
                        .bg(bg_control())
                        .hover(|style| style.bg(bg_control_hover()))
                        .active(|style| style.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .flex_row()
                        .items_center()
                        .justify_center()
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
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg_secondary()),
                        ),
                )
                .child(
                    view()
                        .role(AccessibilityRole::Button)
                        .h(px(CONTROL_H))
                        .px_2p5()
                        .flex_shrink_0()
                        .rounded(px(RADIUS_MD))
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
                        .active(|style| style.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .flex_row()
                        .items_center()
                        .justify_center()
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
                                .text_size(px(TEXT_SM))
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
                        .role(AccessibilityRole::Button)
                        .h(px(CONTROL_H))
                        .px_3()
                        .flex_shrink_0()
                        .rounded(px(RADIUS_MD))
                        .border_1()
                        .border_color(border_control())
                        .bg(bg_control())
                        .hover(|style| style.bg(bg_control_hover()))
                        .active(|style| style.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .flex_row()
                        .items_center()
                        .justify_center()
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
                                .text_size(px(TEXT_SM))
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
                        .role(AccessibilityRole::Button)
                        .h(px(CONTROL_H))
                        .px_3p5()
                        .flex_shrink_0()
                        .rounded(px(RADIUS_MD))
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
                            style.bg(move || {
                                if save_ready.get() {
                                    accent_blue_active()
                                } else {
                                    bg_control()
                                }
                            })
                        })
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .disabled(move || !save_ready.get())
                        .on_press(move || {
                            if !save_ready.get() {
                                return;
                            }
                            dropdown_open.set(false);
                            commit_and_save_profile(
                                profiles,
                                QuickRuleDraftSignals {
                                    kind: draft_kind,
                                    host: draft_host,
                                    path: draft_path,
                                    op: draft_op,
                                    from: draft_from,
                                    to: draft_to,
                                    modes: draft_modes,
                                },
                                None,
                                &save_controls,
                            );
                        })
                        .child(
                            text(tr("保存", "Save"))
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || {
                                    if ready_text() {
                                        fg_on_accent()
                                    } else {
                                        fg_muted()
                                    }
                                }),
                        )
                }),
        )
}
