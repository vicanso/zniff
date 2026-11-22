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
    match (enabled_groups > 0, lang) {
        (true, Lang::Zh) => format!("{rule_count} 条 · {enabled_groups} 组"),
        (true, Lang::En) => format!("{rule_count} · {enabled_groups}g"),
        (false, Lang::Zh) => format!("{rule_count} 条"),
        (false, Lang::En) => rule_count.to_string(),
    }
}

/// Formats the full hover tooltip for the top-bar rules button.
pub(crate) fn format_rules_pill_tooltip(
    rule_count: usize,
    enabled_groups: usize,
    rules_on: bool,
    lang: Lang,
) -> String {
    let hint = t(
        lang,
        "规则、分组、直通与上游代理",
        "Rules, groups, TLS bypass & upstream",
    );
    if rule_count == 0 && enabled_groups == 0 {
        return hint.to_string();
    }
    let summary = match (rules_on, enabled_groups > 0, lang) {
        (true, true, Lang::Zh) => {
            format!("已启用 {rule_count} 条规则 · {enabled_groups} 个分组")
        }
        (true, false, Lang::Zh) => format!("已启用 {rule_count} 条规则"),
        (false, true, Lang::Zh) => {
            format!("规则已暂停（{rule_count} 条规则 · {enabled_groups} 个分组）")
        }
        (false, false, Lang::Zh) => format!("规则已暂停（{rule_count} 条规则）"),
        (true, true, Lang::En) => {
            format!("{rule_count} active rules · {enabled_groups} groups")
        }
        (true, false, Lang::En) => format!("{rule_count} active rules"),
        (false, true, Lang::En) => {
            format!("Rules paused ({rule_count} rules · {enabled_groups} groups)")
        }
        (false, false, Lang::En) => format!("Rules paused ({rule_count} rules)"),
    };
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
        .trigger(
            view()
                .window_drag(false)
                .h(px(CONTROL_H))
                .px_2()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_1p5()
                .rounded(px(CONTROL_RADIUS))
                .border_1()
                .border_color(move || {
                    if capture_muted(rules_on, breaks_on) {
                        status_warn_fg()
                    } else if open.get() || has_rules_config() {
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
                .active(|style| style.opacity(0.8))
                .cursor(Cursor::Pointer)
                .tooltip(
                    tooltip().child(
                        view()
                            .max_w(px(360.0))
                            .px_2p5()
                            .py_1p5()
                            .rounded(px(6.0))
                            .bg(bg_card())
                            .border_1()
                            .border_color(border_strong())
                            .child(
                                text(move || -> SharedString {
                                    format_rules_pill_tooltip(
                                        active_rules_count.get(),
                                        enabled_groups_count.get(),
                                        rules_on.get(),
                                        ui_lang(langs),
                                    )
                                    .into()
                                })
                                .text_size(px(11.5))
                                .text_color(fg_primary()),
                            ),
                    ),
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
                    13.5,
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
                            .rounded(px(4.0))
                            .overflow_hidden()
                            .flex_row()
                            .items_center()
                            .justify_center()
                            .bg(move || {
                                if rules_on.get() {
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
                                .text_size(px(10.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || {
                                    if rules_on.get() {
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

fn rules_panel(
    profiles: ProfileSignals,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
) -> impl IntoElement {
    let langs = ui_langs();
    let bypass_text = profiles.bypass;
    let rule_target = RuleEditTarget::from_profiles(profiles);
    let upstream_text = profiles.upstream;
    let rules_on = profiles.rules_on;
    let breaks_on = profiles.breaks_on;
    let show_tunnels = profiles.show_tunnels;
    let parsed_rules_count = computed(move || {
        let effective =
            effective_rules_text(&rule_target.workspace.get(), &rule_target.groups.get());
        crate::rules::parse_rules(&effective).len()
    });
    let invalid_lines =
        computed(move || crate::rules::invalid_rule_lines(&rule_target.current_text()));
    let import_closed = signal(false);

    let prefill = profiles.quick_rule_prefill;
    effect(move || {
        if let Some(next) = prefill.get() {
            apply_quick_rule_prefill(next, draft);
            prefill.set(None);
        }
    });

    let bypass_controls = controls.clone();
    let rule_groups_controls = controls.clone();
    let rules_controls = controls.clone();
    let rule_strip_controls = controls.clone();
    let rules_clear_controls = controls.clone();
    let templates_controls = controls.clone();
    let upstream_controls = controls.clone();
    let sync_controls = controls.clone();
    let tunnel_controls = controls;
    let show_sync_bar = computed(move || live_preset_save_state(profiles, draft).show_bar());

    view()
        .w(move || px((window().width.0 - 32.0).clamp(480.0, 680.0)))
        .max_w(move || px((window().width.0 - 16.0).max(280.0)))
        .max_h(move || px(adaptive_filter_panel_max_height(window().height.0)))
        .flex_col()
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
                        .gap_2()
                        .flex_col()
                        .child(rules_panel_header(parsed_rules_count, rules_on, langs))
                        .child(
                            view()
                                .w_full()
                                .flex_row()
                                .gap_1p5()
                                .child(enable_switch(rules_on, "启用规则", "Rules", {
                                    let controls = rules_controls.clone();
                                    move |enabled| {
                                        controls.hub.set_rules_enabled(enabled);
                                    }
                                }))
                                .child(enable_switch(breaks_on, "启用断点", "Breaks", {
                                    let controls = rules_controls.clone();
                                    move |enabled| {
                                        controls.hub.set_breaks_enabled(enabled);
                                    }
                                }))
                                .child(enable_switch(
                                    show_tunnels,
                                    "隧道失败",
                                    "Tunnel errors",
                                    move |on| {
                                        tunnel_controls.hub.set_show_tunnel_failures(on);
                                        save_config_or_notify(
                                            &tunnel_controls,
                                            PrefName::Tunnel,
                                            |cfg| cfg.show_tunnel_failures = on,
                                        );
                                    },
                                )),
                        )
                        .child(config_field(
                            "直通名单 (不解密)",
                            "TLS Bypass (No MITM)",
                            side_input(
                                bypass_text,
                                tr(
                                    "example.com | *.pin.dev 或 only:api.example.com",
                                    "example.com | *.pin.dev or only:api.example.com",
                                ),
                                28.0,
                                false,
                                move |text| {
                                    bypass_text.set(text.to_string());
                                    bypass_controls.hub.set_bypass(text);
                                    save_config_or_notify(
                                        &bypass_controls,
                                        PrefName::Bypass,
                                        |cfg| cfg.bypass_hosts = text.to_string(),
                                    );
                                },
                            ),
                        ))
                        .child(
                            view()
                                .gap_1()
                                .w_full()
                                .child(rules_editor_header(
                                    rule_target,
                                    invalid_lines,
                                    rules_clear_controls,
                                    langs,
                                ))
                                .child(rule_groups_bar(rule_target, rule_groups_controls))
                                .child(rules_editor(
                                    rule_target,
                                    rules_controls.clone(),
                                    import_closed,
                                ))
                                .child(rule_line_strip(
                                    rule_target,
                                    rule_strip_controls,
                                    draft.host,
                                    draft.path,
                                    draft.op,
                                    draft.from,
                                    draft.to,
                                    draft.modes,
                                    draft.kind,
                                ))
                                .child(rule_dry_run_strip(
                                    rule_target,
                                    rules_controls.clone(),
                                    move || {
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
                                        if line.trim().is_empty() {
                                            None
                                        } else {
                                            Some(line)
                                        }
                                    },
                                )),
                        )
                        .child(replace_row(
                            rule_target,
                            rules_controls,
                            draft.host,
                            draft.path,
                            draft.op,
                            draft.from,
                            draft.to,
                            draft.modes,
                            draft.kind,
                        ))
                        .child(rule_templates_section(rule_target, templates_controls))
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
                                    save_config_or_notify(
                                        &upstream_controls,
                                        PrefName::Upstream,
                                        |cfg| cfg.upstream_proxy = text.to_string(),
                                    );
                                },
                            ),
                        )),
                ),
        )
        .child(preset_sync_bar(profiles, draft, sync_controls))
}

fn rules_panel_header(
    parsed_rules_count: Memo<usize>,
    rules_on: Signal<bool>,
    langs: Signal<Option<Lang>>,
) -> impl IntoElement {
    view()
        .h(px(28.0))
        .flex_row()
        .items_center()
        .justify_between()
        .child(
            text(tr("规则与代理配置", "Rules & Proxy Config"))
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_primary()),
        )
        .child(when(
            move || parsed_rules_count.get() > 0,
            move || {
                view()
                    .h(px(20.0))
                    .px_2()
                    .rounded(px(99.0))
                    .flex_row()
                    .items_center()
                    .bg(move || {
                        if rules_on.get() {
                            accent_blue_soft()
                        } else {
                            status_warn_bg()
                        }
                    })
                    .child(
                        text(move || {
                            let count = parsed_rules_count.get();
                            let lang = ui_lang(langs);
                            if rules_on.get() {
                                match lang {
                                    Lang::Zh => format!("{count} 条规则生效"),
                                    Lang::En => format!("{count} active"),
                                }
                            } else {
                                match lang {
                                    Lang::Zh => format!("{count} 条规则已暂停"),
                                    Lang::En => format!("{count} paused"),
                                }
                            }
                        })
                        .tabular_nums()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if rules_on.get() {
                                accent_blue_text()
                            } else {
                                status_warn_fg()
                            }
                        }),
                    )
            },
        ))
}

fn rules_editor_header(
    rule_target: RuleEditTarget,
    invalid_lines: Memo<Vec<usize>>,
    clear_controls: Arc<Controls>,
    langs: Signal<Option<Lang>>,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
            text(tr("转发与改写规则", "Rules"))
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_muted()),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(when(
                    move || !invalid_lines.with(Vec::is_empty),
                    move || {
                        view()
                            .h(px(20.0))
                            .px_1p5()
                            .rounded(px(4.0))
                            .flex_row()
                            .items_center()
                            .bg(status_warn_bg())
                            .child(
                                text(move || {
                                    let lines = invalid_lines.with(|bad| {
                                        bad.iter()
                                            .map(usize::to_string)
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    });
                                    match ui_lang(langs) {
                                        Lang::Zh => format!("第 {lines} 行语法有误"),
                                        Lang::En => format!("Syntax error on line {lines}"),
                                    }
                                })
                                .text_size(px(10.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(status_warn_fg()),
                            )
                    },
                ))
                .child(when(
                    move || !rule_target.current_text().trim().is_empty(),
                    move || {
                        let clear_controls = clear_controls.clone();
                        view()
                            .h(px(24.0))
                            .px_2()
                            .flex_shrink_0()
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
                                rule_target.commit(&clear_controls, String::new());
                            })
                            .child(
                                text(tr("清空", "Clear"))
                                    .text_size(px(11.5))
                                    .text_color(fg_secondary()),
                            )
                    },
                )),
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
    let import_open = signal(false);
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
        .trigger(
            view()
                .window_drag(false)
                .h(px(24.0))
                .max_w(px(140.0))
                .min_w(px(0.0))
                .px_1p5()
                .gap_1()
                .rounded(px(6.0))
                .overflow_hidden()
                .flex_shrink(1.0)
                .flex_row()
                .items_center()
                .cursor(Cursor::Pointer)
                .border_1()
                .border_color(move || {
                    if is_dirty() && !open.get() {
                        status_warn_fg()
                    } else if is_active() {
                        accent_blue_border()
                    } else {
                        border_control()
                    }
                })
                .bg(move || {
                    if is_active() {
                        accent_blue_soft()
                    } else {
                        bg_control()
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
                .tooltip(
                    tooltip().child(
                        view()
                            .max_w(px(360.0))
                            .px_2p5()
                            .py_1p5()
                            .rounded(px(6.0))
                            .bg(bg_card())
                            .border_1()
                            .border_color(border_strong())
                            .child(
                                text(move || -> SharedString {
                                    format_preset_pill_tooltip(
                                        active_name().as_deref(),
                                        is_dirty(),
                                        ui_lang(langs),
                                    )
                                    .into()
                                })
                                .text_size(px(11.5))
                                .text_color(fg_primary()),
                            ),
                    ),
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
                            10.5,
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
                    .text_size(px(11.5))
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
                .rounded(px(10.0))
                .border_1()
                .border_color(border_strong())
                .bg(bg_card())
                .shadow_md()
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
                                "配置预设（同时保存过滤、规则、直通与上游）",
                                "Presets (saves filter, rules, bypass & upstream)",
                            ))
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg_secondary()),
                        ),
                )
                .child(profile_dock(
                    profiles,
                    draft.host,
                    draft.path,
                    draft.op,
                    draft.from,
                    draft.to,
                    draft.modes,
                    draft.kind,
                    import_open,
                    controls,
                )),
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
