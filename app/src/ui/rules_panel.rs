//! Dedicated Rules popover (`规则`) and unified SavedProfile preset pill (`预设`) per ADR-17.

#[allow(unused_imports)]
use super::*;

/// Counts how many rule groups are currently enabled.
pub(crate) fn count_enabled_rule_groups(groups: &[config::RuleGroup]) -> usize {
    groups.iter().filter(|group| group.enabled).count()
}

/// Formats the compact top-bar rules badge (`N 条 · M 组` / `N · Mg`).
pub(crate) fn format_rules_indicator(
    rule_count: usize,
    enabled_groups: usize,
    lang: Lang,
) -> String {
    if rule_count == 0 && enabled_groups == 0 {
        return String::new();
    }
    let rules_str = present::format_thousands(rule_count as u64);
    let groups_str = present::format_thousands(enabled_groups as u64);
    match (enabled_groups > 0, lang) {
        (true, Lang::Zh) => format!("{rules_str} 条 · {groups_str} 组"),
        (true, Lang::En) => format!("{rules_str} · {groups_str}g"),
        (false, Lang::Zh) => format!("{rules_str} 条"),
        (false, Lang::En) => rules_str,
    }
}

/// Formats the full hover tooltip for the top-bar rules button.
pub(crate) fn format_rules_pill_tooltip(
    rule_count: usize,
    enabled_groups: usize,
    rules_on: bool,
    breaks_on: bool,
    lang: Lang,
) -> String {
    let hint = t(
        lang,
        "规则、分组、直通与上游代理",
        "Rules, groups, TLS bypass & upstream",
    );
    if rule_count == 0 && enabled_groups == 0 && breaks_on {
        return hint.to_string();
    }
    let rules_str = present::format_thousands(rule_count as u64);
    let groups_str = present::format_thousands(enabled_groups as u64);
    let mut summary = match (rules_on, enabled_groups > 0, lang) {
        (true, true, Lang::Zh) => {
            format!("已启用 {rules_str} 条规则 · {groups_str} 个分组")
        }
        (true, false, Lang::Zh) => format!("已启用 {rules_str} 条规则"),
        (false, true, Lang::Zh) => {
            format!("规则已暂停（{rules_str} 条规则 · {groups_str} 个分组）")
        }
        (false, false, Lang::Zh) => format!("规则已暂停（{rules_str} 条规则）"),
        (true, true, Lang::En) => {
            format!("{rules_str} active rules · {groups_str} groups")
        }
        (true, false, Lang::En) => format!("{rules_str} active rules"),
        (false, true, Lang::En) => {
            format!("Rules paused ({rules_str} rules · {groups_str} groups)")
        }
        (false, false, Lang::En) => format!("Rules paused ({rules_str} rules)"),
    };
    if !breaks_on {
        summary.push_str(t(lang, " · 断点已暂停", " · Breaks paused"));
    }
    format!("{summary}\n{hint}")
}

/// Formats the full hover tooltip for the top-bar preset pill.
pub(crate) fn format_preset_pill_tooltip(
    active_name: Option<&str>,
    dirty: bool,
    lang: Lang,
) -> String {
    let hint = t(
        lang,
        "切换或保存配置预设（包含过滤与规则）",
        "Switch or save presets (filter + rules)",
    );
    match active_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) if dirty => match lang {
            Lang::Zh => format!(
                "当前预设: {name}（有未保存修改）\n点击打开预设面板保存，或在过滤/规则面板底部直接更新"
            ),
            Lang::En => format!(
                "Active preset: {name} (unsaved changes)\nOpen Presets to save, or update from Filter/Rules"
            ),
        },
        Some(name) => {
            let prefix = t(lang, "当前预设: ", "Active preset: ");
            format!("{prefix}{name}\n{hint}")
        }
        None => hint.to_string(),
    }
}

/// Top-bar `规则` / `Rules` popover entry hosting rule groups, editor, quick add, TLS bypass, and upstream.
pub(crate) fn rules_menu_popover(
    profiles: ProfileSignals,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
) -> impl IntoElement {
    let langs = ui_langs();
    let bypass_text = profiles.bypass;
    let rules_text = profiles.rules;
    let rule_groups = profiles.rule_groups;
    let upstream_text = profiles.upstream;
    let rules_on = profiles.rules_on;
    let breaks_on = profiles.breaks_on;
    let open = profiles.panel_open;
    bind_popover_dismiss(open);
    let active_rules_count = computed(move || {
        let effective = effective_rules_text(&rules_text.get(), &rule_groups.get());
        crate::rules::parse_rules(&effective).len()
    });
    let enabled_groups_count =
        computed(move || rule_groups.with(|groups| count_enabled_rule_groups(groups)));
    let has_rules_config = move || {
        config_set(bypass_text, rules_text, upstream_text)
            || has_enabled_group_rules(&rule_groups.get())
            || enabled_groups_count.get() > 0
    };
    let is_tinted = move || open.get() || active_rules_count.get() > 0;
    popover()
        .open(move || Some(open.get()))
        .on_open_change(move |event: &nsh_base::PopoverEvent| {
            if let nsh_base::PopoverEvent::OpenChange(next, _) = event {
                if *next {
                    profiles.filter_panel_open.set(false);
                    profiles.preset_panel_open.set(false);
                }
                open.set(*next);
            }
        })
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(10.0))
        .popup_style(ui_popup_style)
        .popup_starting_style(ui_popup_starting_style)
        .popup_ending_style(ui_popup_ending_style)
        .trigger(
            view()
                .window_drag(false)
                .role(AccessibilityRole::Button)
                .h(px(CONTROL_H))
                .px_2()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_1p5()
                .rounded(px(RADIUS_MD))
                .border_1()
                .border_color(move || {
                    if capture_muted(rules_on, breaks_on) {
                        status_warn_fg()
                    } else if open.get() || has_rules_config() {
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
                .tooltip(
                    tooltip()
                        .disabled(move || open.get())
                        .child(ui_tooltip_surface(
                            text(move || -> SharedString {
                                format_rules_pill_tooltip(
                                    active_rules_count.get(),
                                    enabled_groups_count.get(),
                                    rules_on.get(),
                                    breaks_on.get(),
                                    ui_lang(langs),
                                )
                                .into()
                            })
                            .text_size(px(TEXT_SM))
                            .text_color(fg_primary()),
                        )),
                )
                .child(when(
                    move || capture_muted(rules_on, breaks_on),
                    move || {
                        view()
                            .w(px(6.0))
                            .h(px(6.0))
                            .rounded_full()
                            .bg(status_warn_fg())
                    },
                ))
                .child(glyph_px(
                    ControlSize::Sm.icon_size(),
                    || Glyph::Sliders,
                    move || {
                        color(if capture_muted(rules_on, breaks_on) {
                            status_warn_fg()
                        } else if is_tinted() || has_rules_config() {
                            accent_blue_text()
                        } else {
                            fg_secondary()
                        })
                    },
                ))
                .child(control_text(tr("规则", "Rules")).text_color(move || {
                    if capture_muted(rules_on, breaks_on) {
                        status_warn_fg()
                    } else if is_tinted() {
                        accent_blue_text()
                    } else {
                        fg_primary()
                    }
                }))
                .child(when(
                    move || active_rules_count.get() > 0 || enabled_groups_count.get() > 0,
                    move || {
                        view()
                            .h(px(16.0))
                            .max_w(px(120.0))
                            .px_1p5()
                            .rounded(px(RADIUS_SM))
                            .overflow_hidden()
                            .flex_row()
                            .items_center()
                            .justify_center()
                            .bg(move || {
                                if !capture_muted(rules_on, breaks_on) {
                                    accent_blue_soft()
                                } else {
                                    status_warn_bg()
                                }
                            })
                            .child(
                                mono(move || {
                                    SharedString::from(format_rules_indicator(
                                        active_rules_count.get(),
                                        enabled_groups_count.get(),
                                        ui_lang(langs),
                                    ))
                                })
                                .min_w(px(0.0))
                                .flex_shrink(1.0)
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Tail)
                                .tabular_nums()
                                .text_size(px(TEXT_XS))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || {
                                    if !capture_muted(rules_on, breaks_on) {
                                        accent_blue_text()
                                    } else {
                                        status_warn_fg()
                                    }
                                }),
                            )
                    },
                )),
        )
        .child(rules_panel(profiles, controls, draft))
}

pub(crate) fn rules_panel(
    profiles: ProfileSignals,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
) -> impl IntoElement {
    let langs = ui_langs();
    let rule_target = RuleEditTarget::from_profiles(profiles);
    let parsed_rules_count = computed(move || {
        let effective =
            effective_rules_text(&rule_target.workspace.get(), &rule_target.groups.get());
        crate::rules::parse_rules(&effective).len()
    });
    let invalid_lines =
        computed(move || crate::rules::invalid_rule_lines(&rule_target.current_text()));
    let active_tab = profiles.rules_panel_tab;
    let quick_add_open = signal(false);

    let prefill = profiles.quick_rule_prefill;
    effect(move || {
        if let Some(next) = prefill.get() {
            apply_quick_rule_prefill(next, draft);
            active_tab.set(0);
            quick_add_open.set(true);
            prefill.set(None);
        }
    });

    let focused_rule_line = profiles.focused_rule_line;
    effect(move || {
        if focused_rule_line.with(Option::is_some) {
            active_tab.set(0);
        }
    });

    let top_controls = controls.clone();
    let tab0_controls = controls.clone();
    let tab1_controls = controls.clone();
    let sync_controls = controls;
    let show_sync_bar = computed(move || live_preset_save_state(profiles, draft).show_bar());

    view()
        .w(move || px((window().width.0 - 32.0).clamp(480.0, 680.0)))
        .max_w(move || px((window().width.0 - 16.0).max(280.0)))
        .max_h(move || px(adaptive_filter_panel_max_height(window().height.0)))
        .flex_col()
        .overflow_hidden()
        .child(
            scroll()
                .w_full()
                .max_h(move || {
                    let reserve = if show_sync_bar.get() {
                        PRESET_SYNC_BAR_HEIGHT
                    } else {
                        0.0
                    };
                    px(
                        (adaptive_filter_panel_max_height(window().height.0) - 2.0 - reserve)
                            .max(120.0),
                    )
                })
                .child(
                    uncull_nested_scroll(view())
                        .w_full()
                        .p_3()
                        .gap_2p5()
                        .flex_col()
                        .child(rules_panel_top_bar(
                            profiles,
                            active_tab,
                            parsed_rules_count,
                            top_controls,
                        ))
                        .child(when(
                            move || active_tab.get() == 0,
                            move || {
                                rules_tab_body(
                                    rule_target,
                                    invalid_lines,
                                    profiles.rules_on,
                                    tab0_controls.clone(),
                                    draft,
                                    quick_add_open,
                                    langs,
                                )
                            },
                        ))
                        .child(when(
                            move || active_tab.get() == 1,
                            move || proxy_tab_body(profiles, tab1_controls.clone()),
                        )),
                ),
        )
        .child(preset_sync_bar(profiles, draft, sync_controls))
}

fn rules_panel_top_bar(
    profiles: ProfileSignals,
    active_tab: Signal<u8>,
    parsed_rules_count: Memo<usize>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let rules_on = profiles.rules_on;
    let breaks_on = profiles.breaks_on;
    let bypass_text = profiles.bypass;
    let upstream_text = profiles.upstream;
    let show_tunnels = profiles.show_tunnels;
    let rules_controls = controls.clone();
    let breaks_controls = controls;
    let has_proxy_cfg = move || {
        !bypass_text.get().trim().is_empty()
            || !upstream_text.get().trim().is_empty()
            || show_tunnels.get()
    };

    view()
        .w_full()
        .pb_2()
        .border_b_1()
        .border_b_color(border_subtle())
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            tabs()
                .flex_row()
                .items_center()
                .gap_4()
                .indicator_style(|s| {
                    s.bottom(px(0.0))
                        .h(px(2.0))
                        .rounded(px(1.0))
                        .bg(accent_blue())
                })
                .child(rules_panel_tab_button(
                    active_tab,
                    0,
                    "规则",
                    "Rules",
                    move || parsed_rules_count.get(),
                    || false,
                ))
                .child(rules_panel_tab_button(
                    active_tab,
                    1,
                    "直通与上游",
                    "Bypass & Proxy",
                    || 0,
                    has_proxy_cfg,
                )),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(mini_toggle_switch(
                    rules_on,
                    "启用",
                    "Enabled",
                    move |enabled| {
                        rules_controls.hub.set_rules_enabled(enabled);
                        save_config_or_notify(&rules_controls, PrefName::Rules, |cfg| {
                            cfg.rules_enabled = enabled;
                        });
                    },
                ))
                .child(mini_toggle_switch(
                    breaks_on,
                    "断点",
                    "Breaks",
                    move |enabled| {
                        breaks_controls.hub.set_breaks_enabled(enabled);
                        save_config_or_notify(&breaks_controls, PrefName::Rules, |cfg| {
                            cfg.breaks_enabled = enabled;
                        });
                    },
                )),
        )
}

fn rules_panel_tab_button(
    active_tab: Signal<u8>,
    tab_idx: u8,
    zh: &'static str,
    en: &'static str,
    badge_count: impl Fn() -> usize + Copy + 'static,
    show_dot: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    let selected = move || active_tab.get() == tab_idx;
    tab()
        .selected(selected)
        .on_press(move |_: &()| active_tab.set(tab_idx))
        .h(px(26.0))
        .px_0p5()
        .pb_1()
        .flex_row()
        .items_center()
        .gap_1p5()
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || if selected() { fg_primary() } else { fg_muted() }),
        )
        .child(when(
            move || badge_count() > 0,
            move || {
                view()
                    .h(px(16.0))
                    .px_1p5()
                    .rounded(px(RADIUS_SM))
                    .flex_row()
                    .items_center()
                    .bg(bg_control())
                    .child(
                        mono(move || present::format_thousands(badge_count() as u64))
                            .tabular_nums()
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg_secondary()),
                    )
            },
        ))
        .child(when(show_dot, move || {
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .bg(accent_blue())
        }))
}

fn rules_tab_body(
    rule_target: RuleEditTarget,
    invalid_lines: Memo<Vec<usize>>,
    rules_on: Signal<bool>,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
    quick_add_open: Signal<bool>,
    langs: Signal<Option<Lang>>,
) -> impl IntoElement {
    let groups_controls = controls.clone();
    let toolbar_controls = controls.clone();
    let editor_controls = controls.clone();
    let strip_controls = controls.clone();
    let dry_run_controls = controls.clone();
    let quick_controls = controls;
    view()
        .w_full()
        .gap_2()
        .flex_col()
        .child(
            view()
                .w_full()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(rule_groups_bar(rule_target, groups_controls)),
                )
                .child(rules_editor_toolbar(
                    rule_target,
                    invalid_lines,
                    toolbar_controls,
                    draft,
                    quick_add_open,
                    langs,
                )),
        )
        .child(rules_editor(rule_target, editor_controls, quick_add_open))
        .child(rule_line_strip(
            rule_target,
            strip_controls,
            draft,
            quick_add_open,
        ))
        .child(rule_dry_run_strip_live(
            rule_target,
            Some(rules_on),
            dry_run_controls,
            move || {
                if !quick_add_open.get() {
                    return DryRunInput::Existing;
                }
                let kind = draft.kind.get();
                let response = draft.modes.mode_for(kind);
                let line = draft_rules_with_op(
                    "",
                    &draft.host.get(),
                    &draft.path.get(),
                    &draft.op.get(),
                    &draft.from.get(),
                    &draft.to.get(),
                    response,
                    kind,
                );
                if !line.trim().is_empty() {
                    DryRunInput::Draft(line)
                } else if quick_draft_has_input(draft) {
                    DryRunInput::IncompleteDraft
                } else {
                    DryRunInput::Existing
                }
            },
        ))
        .child(quick_add_section(
            rule_target,
            quick_controls,
            draft,
            quick_add_open,
        ))
}

fn rules_editor_toolbar(
    rule_target: RuleEditTarget,
    invalid_lines: Memo<Vec<usize>>,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
    quick_add_open: Signal<bool>,
    langs: Signal<Option<Lang>>,
) -> impl IntoElement {
    let templates_controls = controls.clone();
    let clear_controls = controls;
    view()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_1p5()
        .child(when(
            move || !invalid_lines.with(Vec::is_empty),
            move || {
                view()
                    .h(px(20.0))
                    .max_w(px(190.0))
                    .px_1p5()
                    .rounded(px(RADIUS_SM))
                    .overflow_hidden()
                    .flex_row()
                    .items_center()
                    .bg(status_warn_bg())
                    .child(
                        text(move || {
                            invalid_lines.with(|bad| {
                                let preview = bad
                                    .iter()
                                    .take(2)
                                    .map(usize::to_string)
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                match (ui_lang(langs), bad.len()) {
                                    (Lang::Zh, 0..=2) => format!("第 {preview} 行语法有误"),
                                    (Lang::Zh, n) => format!("第 {preview} 等 {n} 行语法有误"),
                                    (Lang::En, 0..=2) => format!("Syntax error on line {preview}"),
                                    (Lang::En, n) => {
                                        format!("Syntax errors on {preview} (+{})", n - 2)
                                    }
                                }
                            })
                        })
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(status_warn_fg()),
                    )
            },
        ))
        .child(rule_templates_menu(
            rule_target,
            templates_controls,
            draft,
            quick_add_open,
        ))
        .child(when(
            move || !rule_target.current_text().trim().is_empty(),
            move || {
                let clear_controls = clear_controls.clone();
                let confirm_key = signal(None::<(Option<String>, String)>);
                let current_key = move || (rule_target.editing.get(), rule_target.current_text());
                let is_confirming =
                    move || confirm_key.with(|k| k.as_ref() == Some(&current_key()));
                view()
                    .role(AccessibilityRole::Button)
                    .h(px(ControlSize::Xs.height()))
                    .px_1p5()
                    .flex_shrink_0()
                    .rounded(px(RADIUS_SM))
                    .bg(move || {
                        if is_confirming() {
                            danger_button_bg()
                        } else {
                            AdaptiveColor::TRANSPARENT
                        }
                    })
                    .hover(|style| {
                        style.bg(move || {
                            if is_confirming() {
                                danger_button_hover()
                            } else {
                                bg_control_hover()
                            }
                        })
                    })
                    .active(|style| {
                        style.bg(move || {
                            if is_confirming() {
                                danger_button_active()
                            } else {
                                bg_control_active()
                            }
                        })
                    })
                    .transition_colors()
                    .duration(MOTION_INSTANT_MS)
                    .ease_out()
                    .focus_visible(apply_focus_ring)
                    .flex_row()
                    .items_center()
                    .label(move || -> SharedString {
                        let lang = ui_lang(langs);
                        if is_confirming() {
                            t(
                                lang,
                                "再次点击确认清空当前规则",
                                "Click again to clear rules",
                            )
                            .into()
                        } else {
                            t(lang, "清空", "Clear").into()
                        }
                    })
                    .tooltip(label_tip(
                        "清空当前规则内容（点击两次确认）",
                        "Clear current rules (click twice to confirm)",
                    ))
                    .on_press(move || {
                        let key = current_key();
                        if confirm_key.with_untracked(|k| k.as_ref() == Some(&key)) {
                            confirm_key.set(None);
                            rule_target.commit(&clear_controls, String::new());
                            set_status(&clear_controls, StatusMsg::RulesCleared);
                        } else {
                            confirm_key.set(Some(key));
                        }
                    })
                    .child(
                        text(move || -> SharedString {
                            let lang = ui_lang(langs);
                            if is_confirming() {
                                t(lang, "确认清空？", "Confirm clear?").into()
                            } else {
                                t(lang, "清空", "Clear").into()
                            }
                        })
                        .number_of_lines(1)
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if is_confirming() {
                                fg_on_accent()
                            } else {
                                fg_muted()
                            }
                        }),
                    )
            },
        ))
}

fn proxy_tab_body(profiles: ProfileSignals, controls: Arc<Controls>) -> impl IntoElement {
    let bypass_text = profiles.bypass;
    let upstream_text = profiles.upstream;
    let show_tunnels = profiles.show_tunnels;
    let bypass_controls = controls.clone();
    let upstream_controls = controls.clone();
    let tunnel_controls = controls;
    view()
        .w_full()
        .gap_2p5()
        .flex_col()
        .child(config_field(
            "直通名单 (不解密)",
            "TLS Bypass (No MITM)",
            side_input(
                bypass_text,
                tr(
                    "每行或用 | 分隔：example.com | *.pin.dev 或 only:api.example.com",
                    "example.com | *.pin.dev or only:api.example.com",
                ),
                72.0,
                true,
                move |text| {
                    bypass_text.set(text.to_string());
                    bypass_controls.hub.set_bypass(text);
                    save_config_or_notify(&bypass_controls, PrefName::Bypass, |cfg| {
                        cfg.bypass_hosts = text.to_string();
                    });
                },
            ),
        ))
        .child(config_field(
            "上游代理",
            "Upstream Proxy",
            side_input(
                upstream_text,
                tr(
                    "http://127.0.0.1:7890 或 socks5h://127.0.0.1:1080",
                    "http://127.0.0.1:7890 or socks5h://127.0.0.1:1080",
                ),
                28.0,
                false,
                move |text| {
                    upstream_text.set(text.to_string());
                    upstream_controls.hub.set_upstream(text);
                    save_config_or_notify(&upstream_controls, PrefName::Upstream, |cfg| {
                        cfg.upstream_proxy = text.to_string();
                    });
                },
            ),
        ))
        .child(
            view()
                .w_full()
                .px_2p5()
                .py_2()
                .rounded(px(RADIUS_MD))
                .bg(bg_canvas())
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_0p5()
                        .flex_col()
                        .child(
                            text(tr("显示隧道失败", "Show tunnel failures"))
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg_primary()),
                        )
                        .child(
                            text(tr(
                                "在流量列表显示直通或源站 TLS 握手失败的隧道记录",
                                "Show bypassed or failed TLS CONNECT tunnels in the flow list",
                            ))
                            .text_size(px(TEXT_XS))
                            .text_color(fg_muted()),
                        ),
                )
                .child(mini_toggle_switch(show_tunnels, "开启", "On", move |on| {
                    tunnel_controls.hub.set_show_tunnel_failures(on);
                    save_config_or_notify(&tunnel_controls, PrefName::Tunnel, |cfg| {
                        cfg.show_tunnel_failures = on;
                    });
                })),
        )
}

/// Unified top-bar Preset pill hosting `SavedProfile` switching, saving, new setup, and TOML import/export.
pub(crate) fn preset_pill_popover(
    profiles: ProfileSignals,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
) -> impl IntoElement {
    let langs = ui_langs();
    let open = profiles.preset_panel_open;
    bind_popover_dismiss(open);
    let import_open = profiles.import_open;
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
    let is_active = move || open.get() || active_name().is_some();
    let save_state = computed(move || live_preset_save_state(profiles, draft));
    let is_dirty = move || save_state.with(PresetSaveState::is_dirty);

    popover()
        .open(move || Some(open.get()))
        .on_open_change(move |event: &nsh_base::PopoverEvent| {
            if let nsh_base::PopoverEvent::OpenChange(next, _) = event {
                if *next {
                    profiles.filter_panel_open.set(false);
                    profiles.panel_open.set(false);
                }
                open.set(*next);
            }
        })
        .placement(Placement::Bottom)
        .align(Align::Start)
        .offset(px(6.0))
        .margin(px(10.0))
        .popup_style(ui_popup_style)
        .popup_starting_style(ui_popup_starting_style)
        .popup_ending_style(ui_popup_ending_style)
        .trigger(
            view()
                .window_drag(false)
                .role(AccessibilityRole::Button)
                .h(px(CONTROL_H))
                .max_w(px(140.0))
                .min_w(px(0.0))
                .px_2()
                .gap_1p5()
                .rounded(px(RADIUS_MD))
                .overflow_hidden()
                .flex_shrink(1.0)
                .flex_row()
                .items_center()
                .border_1()
                .border_color(move || {
                    if is_dirty() && !open.get() {
                        status_warn_fg()
                    } else if is_active() {
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
                .active(|style| style.bg(bg_control_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .tooltip(
                    tooltip().disabled(move || open.get()).child(ui_tooltip_surface(
                        text(move || -> SharedString {
                            format_preset_pill_tooltip(
                                active_name().as_deref(),
                                is_dirty(),
                                ui_lang(langs),
                            )
                            .into()
                        })
                        .text_size(px(TEXT_SM))
                        .text_color(fg_primary()),
                    )),
                )
                .child(when(is_dirty, move || {
                    view()
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded_full()
                        .flex_shrink_0()
                        .bg(status_warn_fg())
                }))
                .child(when(
                    move || !is_dirty(),
                    move || {
                        glyph_px(
                            12.0,
                            move || {
                                if active_name().is_some() {
                                    Glyph::Check
                                } else {
                                    Glyph::Badge
                                }
                            },
                            move || {
                                color(if is_active() {
                                    accent_blue_text()
                                } else {
                                    fg_muted()
                                })
                            },
                        )
                    },
                ))
                .child(
                    text(move || {
                        active_name()
                            .unwrap_or_else(|| t(ui_lang(langs), "预设", "Presets").to_string())
                    })
                    .min_w(px(0.0))
                    .flex_shrink(1.0)
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail)
                    .text_size(px(TEXT_SM))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || {
                        if is_active() {
                            accent_blue_text()
                        } else {
                            fg_secondary()
                        }
                    }),
                )
                .child(glyph_px(
                    10.0,
                    || Glyph::Chevron,
                    move || {
                        color(if is_active() {
                            accent_blue_text()
                        } else {
                            fg_muted()
                        })
                    },
                )),
        )
        .child(
            view()
                .w(move || px((window().width.0 - 32.0).clamp(520.0, 680.0)))
                .max_w(move || px((window().width.0 - 16.0).max(280.0)))
                .flex_col()
                .overflow_hidden()
                .child(
                    view()
                        .w_full()
                        .px_3()
                        .py_2()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(
                            text(tr(
                                "预设（保存过滤、当前规则、直通与上游；规则分组全局共享）",
                                "Presets (saves filter, workspace rules, bypass & upstream; groups stay global)",
                            ))
                            .text_size(px(TEXT_SM))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg_secondary()),
                        ),
                )
                .child(profile_dock(profiles, draft, import_open, controls)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_indicator_formats_counts_and_groups() {
        assert_eq!(format_rules_indicator(0, 0, Lang::Zh), "");
        assert_eq!(format_rules_indicator(0, 0, Lang::En), "");
        assert_eq!(format_rules_indicator(4, 0, Lang::Zh), "4 条");
        assert_eq!(format_rules_indicator(4, 0, Lang::En), "4");
        assert_eq!(format_rules_indicator(5, 2, Lang::Zh), "5 条 · 2 组");
        assert_eq!(format_rules_indicator(5, 2, Lang::En), "5 · 2g");
        assert_eq!(format_rules_indicator(0, 1, Lang::Zh), "0 条 · 1 组");
        assert_eq!(format_rules_indicator(0, 1, Lang::En), "0 · 1g");
        assert!(
            format_rules_pill_tooltip(1234, 12, true, true, Lang::En)
                .starts_with("1,234 active rules · 12 groups\n")
        );
        assert!(
            format_rules_pill_tooltip(1234, 0, false, true, Lang::Zh)
                .starts_with("规则已暂停（1,234 条规则）\n")
        );
        assert!(format_rules_pill_tooltip(3, 0, true, false, Lang::Zh).contains("断点已暂停"));
    }

    #[test]
    fn count_enabled_rule_groups_ignores_disabled() {
        let groups = vec![
            config::RuleGroup {
                name: "a".into(),
                enabled: true,
                rules: String::new(),
            },
            config::RuleGroup {
                name: "b".into(),
                enabled: false,
                rules: String::new(),
            },
            config::RuleGroup {
                name: "c".into(),
                enabled: true,
                rules: String::new(),
            },
        ];
        assert_eq!(count_enabled_rule_groups(&groups), 2);
    }
}
