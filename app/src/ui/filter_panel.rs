#[allow(unused_imports)]
use super::*;

pub(crate) const SAVED_RULE_CAP: usize = 16;

pub(crate) fn saved_filter_menu(
    profiles: ProfileSignals,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
) -> impl IntoElement {
    let filter = profiles.filter;
    let filter_enabled = profiles.filter_enabled;
    let open = profiles.filter_panel_open;
    bind_popover_dismiss(open);
    let has_filter = move || !filter.get().trim().is_empty();
    let is_paused = move || has_filter() && !filter_enabled.get();
    let is_tinted = move || open.get() || (has_filter() && filter_enabled.get());
    popover()
        .open(move || Some(open.get()))
        .on_open_change(move |event: &nsh_base::PopoverEvent| {
            if let nsh_base::PopoverEvent::OpenChange(next, _) = event {
                if *next {
                    profiles.panel_open.set(false);
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
                    if is_paused() {
                        status_warn_fg()
                    } else if is_tinted() {
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
                .duration(move || motion_ms(MOTION_INSTANT_MS))
                .ease_out()
                .focus_visible(apply_focus_ring)
                .tooltip(label_tip("视图流量过滤", "Traffic view filter"))
                .child(when(is_paused, move || {
                    view()
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded_full()
                        .bg(status_warn_fg())
                }))
                .child(glyph_px(
                    ControlSize::Sm.icon_size(),
                    || Glyph::List,
                    move || {
                        color(if is_paused() {
                            status_warn_fg()
                        } else if is_tinted() {
                            accent_blue_text()
                        } else {
                            fg_secondary()
                        })
                    },
                ))
                .child(control_text(tr("过滤", "Filter")).text_color(move || {
                    if is_paused() {
                        status_warn_fg()
                    } else if is_tinted() {
                        accent_blue_text()
                    } else {
                        fg_primary()
                    }
                })),
        )
        .child(saved_filter_panel(profiles, controls, draft))
}

pub(crate) fn saved_filter_panel(
    profiles: ProfileSignals,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
) -> impl IntoElement {
    let filter = profiles.filter;
    let field = filter;
    let reset_filter = filter;
    let host_field = signal(extract_host_or(&filter.get_untracked()));
    let path_field = signal(extract_path_or(&filter.get_untracked()));
    let exclude_field = signal(extract_exclude_or(&filter.get_untracked()));
    let show_sync_bar = computed(move || live_preset_save_state(profiles, draft).show_bar());
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

    view()
        .w(move || px((window().width.0 - 32.0).clamp(360.0, 480.0)))
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
                    view()
                        .w_full()
                        .p_3()
                        .gap_2()
                        .flex_col()
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
                                                .text_size(px(TEXT_MD))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(fg_primary()),
                                        )
                                        .child(
                                            text(tr(
                                                "多项用 | 或逗号分隔",
                                                "Separate OR items with | or comma",
                                            ))
                                            .text_size(px(TEXT_XS))
                                            .text_color(fg_muted()),
                                        ),
                                )
                                .child(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .gap_1p5()
                                        .child(when(
                                            move || !reset_filter.get().trim().is_empty(),
                                            move || {
                                                let langs = ui_langs();
                                                let enabled = profiles.filter_enabled;
                                                view()
                                                    .role(AccessibilityRole::Button)
                                                    .h(px(ControlSize::Xs.height()))
                                                    .px_2()
                                                    .rounded(px(RADIUS_SM))
                                                    .border_1()
                                                    .border_color(move || {
                                                        if enabled.get() {
                                                            border_control()
                                                        } else {
                                                            status_warn_fg()
                                                        }
                                                    })
                                                    .bg(move || {
                                                        if enabled.get() {
                                                            bg_control()
                                                        } else {
                                                            status_warn_bg()
                                                        }
                                                    })
                                                    .hover(|style| style.bg(bg_control_hover()))
                                                    .active(|style| style.bg(bg_control_active()))
                                                    .transition_colors()
                                                    .duration(move || motion_ms(MOTION_INSTANT_MS))
                                                    .ease_out()
                                                    .focus_visible(apply_focus_ring)
                                                    .flex_row()
                                                    .items_center()
                                                    .on_press(move || enabled.set(!enabled.get()))
                                                    .child(
                                                        text(move || -> SharedString {
                                                            let lang = ui_lang(langs);
                                                            if enabled.get() {
                                                                t(lang, "暂停过滤", "Pause")
                                                            } else {
                                                                t(lang, "恢复过滤", "Resume")
                                                            }
                                                            .into()
                                                        })
                                                        .text_size(px(TEXT_XS))
                                                        .text_color(move || {
                                                            if enabled.get() {
                                                                fg_secondary()
                                                            } else {
                                                                status_warn_fg()
                                                            }
                                                        }),
                                                    )
                                            },
                                        ))
                                        .child(when(
                                            move || !reset_filter.get().trim().is_empty(),
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
                                                    .duration(move || motion_ms(MOTION_INSTANT_MS))
                                                    .ease_out()
                                                    .focus_visible(apply_focus_ring)
                                                    .flex_row()
                                                    .items_center()
                                                    .on_press(move || {
                                                        profiles.filter_enabled.set(true);
                                                        reset_filter.set(String::new());
                                                    })
                                                    .child(
                                                        text(tr("清空条件", "Reset"))
                                                            .text_size(px(TEXT_XS))
                                                            .text_color(fg_secondary()),
                                                    )
                                            },
                                        )),
                                ),
                        )
                        .child(when(
                            move || {
                                !profiles.filter_enabled.get()
                                    && !reset_filter.get().trim().is_empty()
                            },
                            move || {
                                view()
                                    .w_full()
                                    .px_2p5()
                                    .py_1p5()
                                    .rounded(px(RADIUS_MD))
                                    .border_1()
                                    .border_color(status_warn_fg())
                                    .bg(status_warn_bg())
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .child(
                                        text(tr(
                                            "主过滤当前已暂停，以下条件暂不生效",
                                            "Primary filter is paused — conditions below are inactive",
                                        ))
                                        .flex_1()
                                        .text_size(px(TEXT_XS))
                                        .text_color(status_warn_fg()),
                                    )
                                    .child(
                                        ui_button(ButtonVariant::Outline, ControlSize::Xs)
                                            .on_press(move || profiles.filter_enabled.set(true))
                                            .child(
                                                control_text(tr("恢复", "Resume"))
                                                    .text_color(fg_primary()),
                                            ),
                                    )
                            },
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
                                    text(tr("快捷过滤", "Quick Filters"))
                                        .text_size(px(TEXT_XS))
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
                ),
        )
        .child(preset_sync_bar(profiles, draft, controls))
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
