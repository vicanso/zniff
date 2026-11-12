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
    let upstream_text = profiles.upstream;
    let rules_on = profiles.rules_on;
    let breaks_on = profiles.breaks_on;
    let active_rules_count = computed(move || crate::rules::parse_rules(&rules_text.get()).len());
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
                    move || {
                        !filter.get().trim().is_empty()
                            || config_set(bypass_text, rules_text, upstream_text)
                            || capture_muted(rules_on, breaks_on)
                    },
                    move || {
                        view().w(px(6.0)).h(px(6.0)).rounded_full().bg(move || {
                            if capture_muted(rules_on, breaks_on) {
                                status_warn_fg()
                            } else if !filter.get().trim().is_empty() {
                                accent_blue()
                            } else {
                                status_ok_fg()
                            }
                        })
                    },
                ))
                .child(glyph_px(
                    13.5,
                    || Glyph::Sliders,
                    move || {
                        color(if capture_muted(rules_on, breaks_on) {
                            status_warn_fg()
                        } else if is_tinted() || config_set(bypass_text, rules_text, upstream_text)
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
    let rules_text = profiles.rules;
    let upstream_text = profiles.upstream;
    let rules_on = profiles.rules_on;
    let breaks_on = profiles.breaks_on;
    let show_tunnels = profiles.show_tunnels;
    let list_width = profiles.list_width;
    let parsed_rules_count = computed(move || crate::rules::parse_rules(&rules_text.get()).len());
    let invalid_lines = computed(move || crate::rules::invalid_rule_lines(&rules_text.get()));
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
    let rules_controls = controls.clone();
    let rules_save_controls = controls.clone();
    let rules_clear_controls = controls.clone();
    let upstream_controls = controls.clone();
    let proxy_auth_text = profiles.proxy_auth;
    let proxy_auth_controls = controls.clone();
    let relay_url_text = profiles.relay_url;
    let relay_token_text = signal(lock_string(&controls.relay_token));
    let relay_accept_on = profiles.relay_accept;
    let relay_url_controls = controls.clone();
    let relay_token_controls = controls.clone();
    let relay_accept_controls = controls.clone();
    let tunnel_controls = controls;
    let draft_host = signal(String::new());
    let draft_path = signal(String::new());
    let draft_op = signal(String::new());
    let draft_from = signal(String::new());
    let draft_to = signal(String::new());
    let draft_modes = QuickRuleModeSignals::new();
    let draft_kind = signal(0_u8);
    let import_open = signal(false);

    view()
        .w(move || {
            px(adaptive_filter_panel_width(
                window().width.0,
                list_width.get(),
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
                            "过滤表达式",
                            "Filter Expression",
                            focused_input(
                                field,
                                tr(
                                    "host:a.com path:/api type:json，支持 | 表示 OR",
                                    "host:a.com path:/api type:json, use | for OR",
                                ),
                                28.0,
                                false,
                                move |text| field.set(text.to_string()),
                                None,
                            ),
                        ))
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
                                            "报错",
                                            "Errors",
                                            "失败",
                                            FAIL_ALIASES,
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
                        ),
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
                                                    move || !rules_text.get().trim().is_empty(),
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
                                                                rules_text.set(String::new());
                                                                clear_controls.hub.set_rules("");
                                                                save_config_or_notify(
                                                                    &clear_controls,
                                                                    PrefName::Rules,
                                                                    |config| config.rules.clear(),
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
                                .child(rules_editor(
                                    rules_text,
                                    rules_controls.clone(),
                                    import_open,
                                )),
                        )
                        .child(replace_row(
                            rules_text,
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
                                    "[user:pass@]host:port，空着直连",
                                    "[user:pass@]host:port, empty = direct",
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
                        ))
                        .child(config_field(
                            "代理认证",
                            "Proxy auth",
                            side_input(
                                proxy_auth_text,
                                tr(
                                    "user:pass，空着不校验",
                                    "user:pass, empty disables auth",
                                ),
                                28.0,
                                false,
                                move |text| {
                                    proxy_auth_text.set(text.to_string());
                                    let stored = text.trim().to_string();
                                    proxy_auth_controls.hub.set_proxy_auth(&stored);
                                    if let Err(err) =
                                        save_config(|config| config.proxy_auth = stored)
                                    {
                                        set_status(
                                            &proxy_auth_controls,
                                            StatusMsg::PrefUnsaved {
                                                name: PrefName::ProxyAuth,
                                                err: err.to_string(),
                                            },
                                        );
                                    }
                                },
                            ),
                        ))
                        .child(config_field(
                            "中转地址",
                            "Relay address",
                            side_input(
                                relay_url_text,
                                tr(
                                    "http://主机:端口，空着只在本机分享",
                                    "http://host:port, empty shares locally",
                                ),
                                28.0,
                                false,
                                move |text| {
                                    relay_url_text.set(text.to_string());
                                    let stored = text.trim().to_string();
                                    *relay_url_controls
                                        .relay_url
                                        .lock()
                                        .unwrap_or_else(|err| err.into_inner()) = stored.clone();
                                    if let Err(err) =
                                        save_config(|config| config.relay_url = stored)
                                    {
                                        set_status(
                                            &relay_url_controls,
                                            StatusMsg::PrefUnsaved {
                                                name: PrefName::Relay,
                                                err: err.to_string(),
                                            },
                                        );
                                    }
                                },
                            ),
                        ))
                        .child(config_field(
                            "中转密钥",
                            "Relay token",
                            side_input(
                                relay_token_text,
                                tr(
                                    "上传和接收用同一个密钥",
                                    "Same secret for upload and receive",
                                ),
                                28.0,
                                false,
                                move |text| {
                                    relay_token_text.set(text.to_string());
                                    let stored = text.trim().to_string();
                                    *relay_token_controls
                                        .relay_token
                                        .lock()
                                        .unwrap_or_else(|err| err.into_inner()) = stored.clone();
                                    relay_token_controls.hub.set_relay_token(&stored);
                                    if let Err(err) =
                                        save_config(|config| config.relay_token = stored)
                                    {
                                        set_status(
                                            &relay_token_controls,
                                            StatusMsg::PrefUnsaved {
                                                name: PrefName::Relay,
                                                err: err.to_string(),
                                            },
                                        );
                                    }
                                },
                            ),
                        ))
                        .child(
                            view().w_full().flex_row().child(enable_switch(
                                relay_accept_on,
                                "接收中继",
                                "Accept relay",
                                move |on| {
                                    relay_accept_controls.hub.set_relay_accept(on);
                                    if on && !relay_accept_controls.hub.relay_token_ready() {
                                        set_status(
                                            &relay_accept_controls,
                                            StatusMsg::RelayAcceptNeedToken,
                                        );
                                    }
                                    if let Err(err) =
                                        save_config(|config| config.relay_accept = on)
                                    {
                                        set_status(
                                            &relay_accept_controls,
                                            StatusMsg::PrefUnsaved {
                                                name: PrefName::Relay,
                                                err: err.to_string(),
                                            },
                                        );
                                    }
                                },
                            )),
                        ),
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
        let preview = rule_preset_preview(rules);
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
