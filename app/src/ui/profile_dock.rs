#[allow(unused_imports)]
use super::*;

fn profile_export_current_snapshot(
    profiles: ProfileSignals,
    draft: QuickRuleDraftSignals,
) -> Option<config::SavedProfile> {
    let kind_now = draft.kind.get();
    let rules_now = if profiles.editing_group.with(|g| g.is_some()) {
        profiles.rules.get()
    } else {
        draft_rules_with_op(
            &profiles.rules.get(),
            &draft.host.get(),
            &draft.path.get(),
            &draft.op.get(),
            &draft.from.get(),
            &draft.to.get(),
            draft.modes.mode_for(kind_now),
            kind_now,
        )
    };
    let name_now = profiles.name.get();
    let filter_now = profiles.filter.get();
    let bypass_now = profiles.bypass.get();
    let upstream_now = profiles.upstream.get();
    let effective_name = if !name_now.trim().is_empty() {
        name_now.trim().to_string()
    } else {
        profiles
            .saved
            .with(|items| {
                matched_saved_profile(items, &filter_now, &bypass_now, &rules_now, &upstream_now)
            })
            .unwrap_or_default()
    };
    let mut current = push_saved_profile(
        &[],
        &effective_name,
        &filter_now,
        &bypass_now,
        &rules_now,
        &upstream_now,
        profiles.rules_on.get(),
        profiles.breaks_on.get(),
        profiles.show_tunnels.get(),
    );
    if let Some(item) = current.pop() {
        return Some(item);
    }
    if !effective_name.is_empty() {
        return profiles.saved.with(|items| {
            items
                .iter()
                .find(|preset| preset.name.eq_ignore_ascii_case(&effective_name))
                .cloned()
        });
    }
    None
}

fn profile_export_menu(
    profiles: ProfileSignals,
    draft: QuickRuleDraftSignals,
    dropdown_open: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let langs = ui_langs();
    let open = signal(false);
    bind_escape_while(
        move || open.get(),
        nsh_core::keymap::When::Always,
        move || open.set(false),
    );
    let can_current = move || profile_export_current_snapshot(profiles, draft).is_some();
    let can_all = move || !profiles.saved.with(Vec::is_empty);
    let current_controls = controls.clone();
    let all_controls = controls;
    menu()
        .open(move || Some(open.get()))
        .on_open_change(move |event: &nsh_base::MenuEvent| {
            if let nsh_base::MenuEvent::OpenChange(next, _) = event {
                if *next {
                    dropdown_open.set(false);
                }
                open.set(*next);
            }
        })
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(4.0))
        .margin(px(8.0))
        .popup_style(ui_popup_style)
        .popup_starting_style(ui_popup_starting_style)
        .popup_ending_style(ui_popup_ending_style)
        .trigger(
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
                .gap_1()
                .tooltip(label_tip_unless(
                    open,
                    "选择导出当前预设或全部已保存预设（TOML）",
                    "Export current or all saved presets as TOML",
                ))
                .child(
                    text(tr("导出", "Export"))
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg_secondary()),
                )
                .child(glyph_px(10.0, || Glyph::Chevron, || color(fg_muted()))),
        )
        .child(
            view()
                .w(px(236.0))
                .p_1()
                .gap_0p5()
                .child(action_item_enabled(
                    tr("导出当前预设 TOML", "Export current preset"),
                    can_current,
                    move || {
                        open.set(false);
                        if let Some(snapshot) = profile_export_current_snapshot(profiles, draft) {
                            export_profiles_clipboard(&current_controls, &[snapshot]);
                        }
                    },
                ))
                .child(action_item_enabled(
                    move || -> SharedString {
                        let n = profiles.saved.with(Vec::len);
                        match ui_lang(langs) {
                            Lang::Zh => format!("导出全部已保存预设（{n}）"),
                            Lang::En => format!("Export all saved presets ({n})"),
                        }
                        .into()
                    },
                    can_all,
                    move || {
                        open.set(false);
                        let all = profiles.saved.get_untracked();
                        if !all.is_empty() {
                            export_profiles_clipboard(&all_controls, &all);
                        }
                    },
                )),
        )
}

fn preview_import_status(raw: &str, lang: Lang) -> Option<(String, bool)> {
    if let Some(msg) = preview_rule_import(raw, lang) {
        return Some((msg, false));
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    match parse_profiles_from_text_or_path(trimmed) {
        Ok(list) => Some((
            match lang {
                Lang::Zh => format!("检测到 TOML 预设：{} 个", list.len()),
                Lang::En => format!("TOML presets detected: {}", list.len()),
            },
            false,
        )),
        Err(err) => Some((
            match lang {
                Lang::Zh => format!("解析提示：{err}"),
                Lang::En => format!("Parse notice: {err}"),
            },
            true,
        )),
    }
}

pub(crate) fn profile_dock(
    profiles: ProfileSignals,
    draft: QuickRuleDraftSignals,
    import_open: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let langs = ui_langs();
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
    let import_panel_controls = controls.clone();
    let save_controls = controls.clone();
    let dropdown_open = signal(false);
    let history_open = signal(false);
    let import_text = signal(String::new());
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
        clear_quick_draft(draft);
    });

    let save_ready = computed(move || {
        let kind_now = draft.kind.get();
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
                &draft.host.get(),
                &draft.path.get(),
                &draft.op.get(),
                &draft.from.get(),
                &draft.to.get(),
                draft.modes.mode_for(kind_now),
                kind_now,
            )
        })
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
                                            button()
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
                                                .on_press(move |_: &()| import_text.set(String::new()))
                                                .child(
                                                    text(tr("清空", "Clear"))
                                                        .text_size(px(TEXT_XS))
                                                        .text_color(fg_secondary()),
                                                )
                                        },
                                    ))
                                    .child(
                                        button()
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
                                            .on_press(move |_: &()| {
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
                                        button()
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
                                            .disabled_style(|s| s.opacity(0.55))
                                            .on_press(move |_: &()| {
                                                if !ready() {
                                                    return;
                                                }
                                                if import_profiles_text(
                                                    &confirm_controls,
                                                    &import_text.get(),
                                                    profiles,
                                                ) {
                                                    clear_quick_draft(draft);
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
                                    .font_size(move || scaled_px(12.0))
                                    .line_height(move || scaled_px(18.0))
                                    .scroll_beyond_last_line(false)
                                    .placeholder(move || {
                                        SharedString::from(tr(
                                            "粘贴 TOML 预设、Whistle 规则或 Charles XML…",
                                            "Paste TOML preset, Whistle rules, or Charles XML…",
                                        )(
                                        ))
                                    })
                                    .on_change(move |text: &SharedString| {
                                        import_text.set(text.to_string());
                                    })
                                    .size_full(),
                            ),
                    )
                    .child({
                        let import_preview = computed(move || {
                            import_text.with(|txt| preview_import_status(txt, ui_lang(langs)))
                        });
                        when(
                            move || import_preview.with(Option::is_some),
                            move || {
                                let is_err = move || {
                                    import_preview
                                        .with(|p| p.as_ref().is_some_and(|(_, err)| *err))
                                };
                                text(move || {
                                    SharedString::from(import_preview.with(|p| {
                                        p.as_ref()
                                            .map(|(msg, _)| msg.clone())
                                            .unwrap_or_default()
                                    }))
                                })
                                .number_of_lines(2)
                                .text_overflow(EllipsizeMode::Tail)
                                .text_size(px(TEXT_XS))
                                .text_color(move || {
                                    if is_err() {
                                        status_warn_fg()
                                    } else {
                                        accent_blue_text()
                                    }
                                })
                            },
                        )
                    })
            },
        ))
        .child(
            view()
                .w_full()
                .flex_col()
                .gap_1p5()
                .child(
                    view()
                        .w_full()
                        .h(px(CONTROL_H))
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_1p5()
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .child(profile_saved_menu(menu_ctx.clone()))
                                .child(profile_history_menu(menu_ctx)),
                        )
                        .child(
                            view()
                                .flex_shrink_0()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .child(profile_export_menu(
                                    profiles,
                                    draft,
                                    dropdown_open,
                                    export_controls,
                                ))
                                .child(
                                    view()
                                        .flex_shrink_0()
                                        .tooltip(label_tip(
                                            if cfg!(target_arch = "wasm32") {
                                                "展开/收起粘贴 TOML / Whistle / Charles XML 导入面板"
                                            } else {
                                                "展开/收起导入面板，支持 TOML / Whistle / Charles XML（或拖拽文件到窗口）"
                                            },
                                            if cfg!(target_arch = "wasm32") {
                                                "Toggle import drawer (TOML / Whistle / Charles XML)"
                                            } else {
                                                "Toggle import drawer for TOML, Whistle, or Charles XML (or drop file)"
                                            },
                                        ))
                                        .child(
                                            toggle()
                                                .pressed(move || import_open.get())
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
                                                .on_change(move |_: &bool| {
                                                    dropdown_open.set(false);
                                                    history_open.set(false);
                                                    import_open.update(|open| *open = !*open);
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
                                        ),
                                ),
                        ),
                )
                .child(
                    view()
                        .w_full()
                        .h(px(CONTROL_H))
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .child(view().flex_1().min_w(px(0.0)).child(side_input(
                            rule_name,
                            tr("预设名称（可选，同名覆盖）", "Preset name (optional, overwrites match)"),
                            CONTROL_H,
                            false,
                            move |text| rule_name.set(text.to_string()),
                        )))
                        .child(
                            view()
                                .flex_shrink_0()
                                .tooltip(label_tip(
                                    "清空当前工作区条件以开始新预设（原内容会保留在历史中）",
                                    "Clear workspace filter/rules to start a new preset (previous state kept in History)",
                                ))
                                .child(
                                    button()
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
                                        .on_press(move |_: &()| {
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
                                            blank_working_setup(profiles, draft, &blank_controls);
                                        })
                                        .child(
                                            text(tr("新建预设", "New preset"))
                                                .text_size(px(TEXT_SM))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(fg_secondary()),
                                        ),
                                ),
                        )
                        .child({
                            let ready_text = move || save_ready.get();
                            view()
                                .flex_shrink_0()
                                .tooltip(tooltip().child(ui_tooltip_surface(
                                    text(move || -> SharedString {
                                        let lang = ui_lang(langs);
                                        if save_ready.get() {
                                            t(lang, "保存或更新同名预设", "Save or update matching preset")
                                        } else if !config_set(filter, bypass_text, upstream_text)
                                            && rules_text.get().trim().is_empty()
                                        {
                                            t(
                                                lang,
                                                "工作区为空，请先填写过滤、规则、直通或上游",
                                                "Workspace is empty — add filter, rules, bypass, or upstream first",
                                            )
                                        } else {
                                            t(
                                                lang,
                                                "当前工作区与已保存预设一致，无需重复保存",
                                                "No unsaved changes against matching preset",
                                            )
                                        }
                                        .into()
                                    })
                                    .text_size(px(TEXT_SM))
                                    .text_color(fg_primary()),
                                )))
                                .child(
                                    button()
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
                                        .disabled_style(|s| s.opacity(0.55))
                                        .on_press(move |_: &()| {
                                            if !save_ready.get() {
                                                return;
                                            }
                                            dropdown_open.set(false);
                                            commit_and_save_profile(profiles, draft, None, &save_controls);
                                            dismiss_keyboard();
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
                                        ),
                                )
                        }),
                ),
        )
}
