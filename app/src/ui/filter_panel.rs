#[allow(unused_imports)]
use super::*;

pub(crate) const SAVED_RULE_CAP: usize = 16;

pub(crate) fn saved_filter_menu(
    profiles: ProfileSignals,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let filter = profiles.filter;
    let saved = profiles.saved;
    let bypass_text = profiles.bypass;
    let rules_text = profiles.rules;
    let rule_groups = profiles.rule_groups;
    let upstream_text = profiles.upstream;
    let rules_on = profiles.rules_on;
    let breaks_on = profiles.breaks_on;
    let active_rules_count = computed(move || {
        let effective = effective_rules_text(&rules_text.get(), &rule_groups.get());
        crate::rules::parse_rules(&effective).len()
    });
    let open = profiles.panel_open;
    let is_tinted = move || open.get() || !filter.get().trim().is_empty();
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
        .margin(px(10.0))
        .trigger(
            view()
                .window_drag(false)
                .h(px(CONTROL_H))
                .px_3()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_1p5()
                .rounded(px(CONTROL_RADIUS))
                .border_1()
                .border_color(move || {
                    if capture_muted(rules_on, breaks_on) {
                        status_warn_fg()
                    } else if open.get()
                        || !filter.get().trim().is_empty()
                        || !saved.with(Vec::is_empty)
                        || config_set(bypass_text, rules_text, upstream_text)
                        || has_enabled_group_rules(&rule_groups.get())
                    {
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
                .tooltip(label_tip("过滤与规则配置", "Filters & Rules Config"))
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
                        } else if is_tinted()
                            || config_set(bypass_text, rules_text, upstream_text)
                            || has_enabled_group_rules(&rule_groups.get())
                        {
                            accent_blue_text()
                        } else {
                            fg_secondary()
                        })
                    },
                ))
                .child(control_text(tr("过滤", "Filters")).text_color(move || {
                    if capture_muted(rules_on, breaks_on) {
                        status_warn_fg()
                    } else if is_tinted() {
                        accent_blue_text()
                    } else {
                        fg_primary()
                    }
                }))
                .child(when(
                    move || active_rules_count.get() > 0,
                    move || {
                        view()
                            .h(px(16.0))
                            .px_1()
                            .rounded(px(4.0))
                            .flex_row()
                            .items_center()
                            .justify_center()
                            .bg(accent_blue_soft())
                            .child(
                                mono(move || active_rules_count.get().to_string())
                                    .tabular_nums()
                                    .text_size(px(10.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(accent_blue_text()),
                            )
                    },
                )),
        )
        .child(saved_filter_panel(profiles, controls))
}

pub(crate) fn saved_filter_panel(
    profiles: ProfileSignals,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let langs = ui_langs();
    let filter = profiles.filter;
    let bypass_text = profiles.bypass;
    let rule_target = RuleEditTarget::from_profiles(profiles);
    let upstream_text = profiles.upstream;
    let rules_on = profiles.rules_on;
    let breaks_on = profiles.breaks_on;
    let show_tunnels = profiles.show_tunnels;
    let list_width = profiles.list_width;
    let layout_mode = profiles.layout_mode;
    let sidebar_width = profiles.sidebar_width;
    let sidebar_collapsed = profiles.sidebar_collapsed;
    let parsed_rules_count = computed(move || {
        let effective =
            effective_rules_text(&rule_target.workspace.get(), &rule_target.groups.get());
        crate::rules::parse_rules(&effective).len()
    });
    let invalid_lines =
        computed(move || crate::rules::invalid_rule_lines(&rule_target.current_text()));
    let field = filter;
    let reset_filter = filter;
    let host_field = signal(extract_host_or(&filter.get_untracked()));
    let path_field = signal(extract_path_or(&filter.get_untracked()));
    let exclude_field = signal(extract_exclude_or(&filter.get_untracked()));
    effect(move || {
        let current = filter.get();
        if extract_host_or(&set_host_or("", &host_field.get_untracked()))
            != extract_host_or(&current)
        {
            host_field.set(extract_host_or(&current));
        }
        if extract_path_or(&set_path_or("", &path_field.get_untracked()))
            != extract_path_or(&current)
        {
            path_field.set(extract_path_or(&current));
        }
        if extract_exclude_or(&set_exclude_or("", &exclude_field.get_untracked()))
            != extract_exclude_or(&current)
        {
            exclude_field.set(extract_exclude_or(&current));
        }
    });

    let bypass_controls = controls.clone();
    let rule_groups_controls = controls.clone();
    let rules_controls = controls.clone();
    let rule_strip_controls = controls.clone();
    let rules_save_controls = controls.clone();
    let rules_clear_controls = controls.clone();
    let upstream_controls = controls.clone();
    let tunnel_controls = controls;
    let draft_host = signal(String::new());
    let draft_path = signal(String::new());
    let draft_op = signal(String::new());
    let draft_from = signal(String::new());
    let draft_to = signal(String::new());
    let draft_modes = QuickRuleModeSignals::new();
    let draft_kind = signal(0_u8);
    let import_open = signal(false);
    let prefill = profiles.quick_rule_prefill;
    let draft_signals = QuickRuleDraftSignals {
        kind: draft_kind,
        host: draft_host,
        path: draft_path,
        op: draft_op,
        from: draft_from,
        to: draft_to,
        modes: draft_modes,
    };
    effect(move || {
        if let Some(next) = prefill.get() {
            apply_quick_rule_prefill(next, draft_signals);
            prefill.set(None);
        }
    });

    view()
        .w(move || {
            let mode = layout_mode.get();
            let side = effective_sidebar_px(mode, sidebar_width.get(), sidebar_collapsed.get());
            px(adaptive_filter_panel_width_for_layout(
                window().width.0,
                list_width.get(),
                mode,
                side,
            ))
        })
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
                    px(adaptive_filter_body_max_height(
                        window().height.0,
                        import_open.get(),
                    ))
                })
                .child(
                    uncull_nested_scroll(view())
                        .w_full()
                        .p_3()
                        .gap_3()
                        .flex_row()
                        .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_2()
                        .child(
                            view()
                                .h(px(28.0))
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .child(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            text(tr("流量过滤", "Traffic Filter"))
                                                .text_size(px(13.0))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(fg_primary()),
                                        )
                                        .child(
                                            text(tr(
                                                "多项用 | 或逗号分隔",
                                                "Separate OR items with | or comma",
                                            ))
                                            .text_size(px(11.0))
                                            .text_color(fg_muted()),
                                        ),
                                )
                                .child(view().flex_row().items_center().gap_1p5().child(when(
                                    move || !reset_filter.get().trim().is_empty(),
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
                                            .on_press(move || reset_filter.set(String::new()))
                                            .child(
                                                text(tr("清空条件", "Reset"))
                                                    .text_size(px(11.5))
                                                    .text_color(fg_secondary()),
                                            )
                                    },
                                ))),
                        )
                        .child(config_field(
                            "包含域名",
                            "Include Domains",
                            side_input(
                                host_field,
                                tr("api.example.com | *.cdn.com", "api.example.com | *.cdn.com"),
                                28.0,
                                false,
                                move |text| {
                                    host_field.set(text.to_string());
                                    filter.set(set_host_or(&filter.get(), text));
                                },
                            ),
                        ))
                        .child(config_field(
                            "包含 Path 前缀",
                            "Include Path Prefixes",
                            side_input(
                                path_field,
                                tr("/api/v1 | /auth | /graphql", "/api/v1 | /auth | /graphql"),
                                28.0,
                                false,
                                move |text| {
                                    path_field.set(text.to_string());
                                    filter.set(set_path_or(&filter.get(), text));
                                },
                            ),
                        ))
                        .child(config_field(
                            "排除域名或路径",
                            "Exclude Host / Path",
                            side_input(
                                exclude_field,
                                tr("ads.example.com | /health", "ads.example.com | /health"),
                                28.0,
                                false,
                                move |text| {
                                    exclude_field.set(text.to_string());
                                    filter.set(set_exclude_or(&filter.get(), text));
                                },
                            ),
                        ))
                        .child(
                            view()
                                .gap_1()
                                .w_full()
                                .child(
                                    text(tr("快捷筛选", "Quick Filters"))
                                        .text_size(px(11.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(fg_muted()),
                                )
                                .child(
                                    view()
                                        .w_full()
                                        .flex_row()
                                        .items_center()
                                        .gap_1p5()
                                        .child(filter_chip(
                                            filter,
                                            "JSON",
                                            "JSON",
                                            "type:json",
                                            JSON_ALIASES,
                                        ))
                                        .child(filter_chip(
                                            filter,
                                            "出错",
                                            "Errors",
                                            "status:4xx|5xx|err",
                                            ERR_ALIASES,
                                        ))
                                        .child(filter_chip(
                                            filter,
                                            "星标",
                                            "Starred",
                                            "pin:",
                                            PIN_ALIASES,
                                        ))
                                        .child(filter_chip(
                                            filter,
                                            "慢请求",
                                            "Slow",
                                            "slow:",
                                            SLOW_ALIASES,
                                        )),
                                ),
                        )
                        .child(config_field(
                            "过滤表达式",
                            "Filter Expression",
                            view()
                                .w_full()
                                .flex_col()
                                .gap_1()
                                .child(filter_assist_input(
                                    field,
                                    tr(
                                        "host:a.com path:/api type:json，支持 | 表示 OR",
                                        "host:a.com path:/api type:json, use | for OR",
                                    ),
                                    28.0,
                                    10.0,
                                    element_ref(),
                                ))
                                .child(filter_syntax_error_inline(field)),
                        )),
                )
                .child(view().w(px(1.0)).self_stretch().bg(border_subtle()))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_2()
                        .child(
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
                                                            Lang::Zh => {
                                                                format!("{count} 条规则生效")
                                                            }
                                                            Lang::En => format!("{count} active"),
                                                        }
                                                    } else {
                                                        match lang {
                                                            Lang::Zh => {
                                                                format!("{count} 条规则已暂停")
                                                            }
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
                                )),
                        )
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
                                        if let Err(err) =
                                            save_config(|config| config.show_tunnel_failures = on)
                                        {
                                            set_status(
                                                &tunnel_controls,
                                                StatusMsg::PrefUnsaved {
                                                    name: PrefName::Tunnel,
                                                    err: err.to_string(),
                                                },
                                            );
                                        }
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
                                    if let Err(err) =
                                        save_config(|config| config.bypass_hosts = text.to_string())
                                    {
                                        set_status(
                                            &bypass_controls,
                                            StatusMsg::PrefUnsaved {
                                                name: PrefName::Bypass,
                                                err: err.to_string(),
                                            },
                                        );
                                    }
                                },
                            ),
                        ))
                        .child(
                            view()
                                .gap_1()
                                .w_full()
                                .child(
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
                                                                    let lines = invalid_lines.with(
                                                                        |bad| {
                                                                            bad.iter()
                                                                                .map(
                                                                                    usize::to_string,
                                                                                )
                                                                                .collect::<Vec<_>>()
                                                                                .join(", ")
                                                                        },
                                                                    );
                                                                    let lang = ui_lang(langs);
                                                                    match lang {
                                                                        Lang::Zh => {
                                                                            format!("第 {lines} 行语法有误")
                                                                        }
                                                                        Lang::En => {
                                                                            format!("Syntax error on line {lines}")
                                                                        }
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
                                                        let clear_controls =
                                                            rules_clear_controls.clone();
                                                        view()
                                                            .h(px(24.0))
                                                            .px_2()
                                                            .flex_shrink_0()
                                                            .rounded(px(6.0))
                                                            .border_1()
                                                            .border_color(border_control())
                                                            .bg(bg_control())
                                                            .hover(|style| {
                                                                style.bg(bg_control_hover())
                                                            })
                                                            .active(|style| style.opacity(0.8))
                                                            .flex_row()
                                                            .items_center()
                                                            .cursor(Cursor::Pointer)
                                                            .on_press(move || {
                                                                rule_target.commit(
                                                                    &clear_controls,
                                                                    String::new(),
                                                                );
                                                            })
                                                            .child(
                                                                text(tr("清空", "Clear"))
                                                                    .text_size(px(11.5))
                                                                    .text_color(fg_secondary()),
                                                            )
                                                    },
                                                )),
                                        ),
                                )
                                .child(rule_groups_bar(rule_target, rule_groups_controls))
                                .child(rules_editor(
                                    rule_target,
                                    rules_controls.clone(),
                                    import_open,
                                ))
                                .child(rule_line_strip(
                                    rule_target,
                                    rule_strip_controls,
                                    draft_host,
                                    draft_path,
                                    draft_op,
                                    draft_from,
                                    draft_to,
                                    draft_modes,
                                    draft_kind,
                                ))
                                .child(rule_dry_run_strip(
                                    rule_target,
                                    rules_controls.clone(),
                                    move || {
                                        let kind = draft_kind.get();
                                        let response = draft_modes.mode_for(kind);
                                        let line = draft_rules_with_op(
                                            "",
                                            &draft_host.get(),
                                            &draft_path.get(),
                                            &draft_op.get(),
                                            &draft_from.get(),
                                            &draft_to.get(),
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
                            draft_host,
                            draft_path,
                            draft_op,
                            draft_from,
                            draft_to,
                            draft_modes,
                            draft_kind,
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
                                    if let Err(err) = save_config(|config| {
                                        config.upstream_proxy = text.to_string()
                                    }) {
                                        set_status(
                                            &upstream_controls,
                                            StatusMsg::PrefUnsaved {
                                                name: PrefName::Upstream,
                                                err: err.to_string(),
                                            },
                                        );
                                    }
                                },
                            ),
                        )),
                ),
                ),
        )
        .child(profile_dock(
            profiles,
            draft_host,
            draft_path,
            draft_op,
            draft_from,
            draft_to,
            draft_modes,
            draft_kind,
            import_open,
            rules_save_controls,
        ))
}

pub(crate) fn matched_saved_profile(
    saved: &[config::SavedProfile],
    filter: &str,
    bypass: &str,
    rules: &str,
    upstream: &str,
) -> Option<String> {
    let filter = filter.trim();
    let bypass = bypass.trim();
    let rules = rules.trim();
    let upstream = upstream.trim();
    if filter.is_empty() && bypass.is_empty() && rules.is_empty() && upstream.is_empty() {
        return None;
    }
    saved
        .iter()
        .find(|preset| {
            preset.filter.trim() == filter
                && preset.bypass_hosts.trim() == bypass
                && preset.rules.trim() == rules
                && preset.upstream_proxy.trim() == upstream
        })
        .map(|preset| preset.name.clone())
}

pub(crate) fn toolbar_active_profile_name(
    saved: &[config::SavedProfile],
    typed_name: &str,
    filter: &str,
    bypass: &str,
    rules: &str,
    upstream: &str,
) -> Option<String> {
    let typed = typed_name.trim();
    if !typed.is_empty() {
        return Some(typed.to_string());
    }
    matched_saved_profile(saved, filter, bypass, rules, upstream)
}

pub(crate) fn redact_upstream_summary(upstream: &str) -> String {
    let trimmed = upstream.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let without_scheme = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("HTTP://"))
        .unwrap_or(trimmed);
    let authority = without_scheme
        .split_once('/')
        .map(|(head, _)| head)
        .unwrap_or(without_scheme);
    if let Some((userinfo, host_port)) = authority.rsplit_once('@') {
        let user = userinfo
            .split_once(':')
            .map(|(u, _)| u)
            .unwrap_or(userinfo)
            .trim();
        if user.is_empty() {
            return host_port.to_string();
        }
        return format!("{user}@{host_port}");
    }
    authority.to_string()
}

pub(crate) fn toolbar_rules_preview(rules: &str, lang: Lang) -> String {
    let first = rules
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or("");
    let count = crate::rules::parse_rules(rules).len();
    if count > 1 {
        let prefix = match lang {
            Lang::Zh => "规则",
            Lang::En => "Rules",
        };
        format!("{prefix}×{count}: {first}")
    } else {
        first.to_string()
    }
}

pub(crate) fn toolbar_filter_conditions(
    filter: &str,
    bypass: &str,
    rules: &str,
    upstream: &str,
    rules_enabled: bool,
    lang: Lang,
) -> (String, bool) {
    let filter = filter.trim();
    if !filter.is_empty() {
        return (filter.to_string(), true);
    }
    let mut parts = Vec::new();
    if rules_enabled {
        let preview = toolbar_rules_preview(rules, lang);
        if !preview.is_empty() {
            parts.push(preview);
        }
    }
    let bypass = bypass.trim();
    if !bypass.is_empty() {
        parts.push(format!("bypass:{bypass}"));
    }
    let upstream = redact_upstream_summary(upstream);
    if !upstream.is_empty() {
        parts.push(format!("upstream:{upstream}"));
    }
    if !parts.is_empty() {
        return (parts.join(" · "), true);
    }
    (
        t(
            lang,
            "全部流量 · 未设置过滤条件",
            "All traffic · No filter set",
        )
        .to_string(),
        false,
    )
}

/// Computes toolbar status bar copy, distinguishing primary filter pause state from secondary list filter.
/// Returns `(display_text, has_condition, dimmed)`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn toolbar_status_summary(
    primary_filter: &str,
    effective_filter: &str,
    filter_enabled: bool,
    bypass: &str,
    rules: &str,
    upstream: &str,
    rules_enabled: bool,
    lang: Lang,
) -> (String, bool, bool) {
    let primary = primary_filter.trim();
    let effective = effective_filter.trim();
    if !primary.is_empty() && !filter_enabled {
        let paused_tag = match lang {
            Lang::Zh => "已暂停",
            Lang::En => "Paused",
        };
        let mut extras = Vec::new();
        if !effective.is_empty() {
            extras.push(effective.to_string());
        } else {
            if rules_enabled {
                let preview = toolbar_rules_preview(rules, lang);
                if !preview.is_empty() {
                    extras.push(preview);
                }
            }
            let bypass = bypass.trim();
            if !bypass.is_empty() {
                extras.push(format!("bypass:{bypass}"));
            }
            let upstream = redact_upstream_summary(upstream);
            if !upstream.is_empty() {
                extras.push(format!("upstream:{upstream}"));
            }
        }
        return if extras.is_empty() {
            (format!("({paused_tag}) {primary}"), true, true)
        } else {
            let prefix = match lang {
                Lang::Zh => "主过滤已暂停",
                Lang::En => "Filter paused",
            };
            (
                format!("{} · ({prefix}: {primary})", extras.join(" · ")),
                true,
                false,
            )
        };
    }
    let (summary, has_cond) =
        toolbar_filter_conditions(effective, bypass, rules, upstream, rules_enabled, lang);
    (summary, has_cond, false)
}
