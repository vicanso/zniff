#[allow(unused_imports)]
use super::*;

#[derive(Clone, PartialEq)]
pub(crate) struct DomainGroup {
    pub(crate) root: String,
    pub(crate) total_count: usize,
    pub(crate) total_errors: usize,
    pub(crate) entries: Vec<session::FacetEntry>,
}

/// Extracts a coarse registrable/root domain for grouping subdomains in the sidebar.
pub(crate) fn root_domain(host: &str) -> String {
    let bare = host.split(':').next().unwrap_or(host).trim();
    if bare.is_empty() || bare.parse::<std::net::IpAddr>().is_ok() {
        return bare.to_string();
    }
    let parts: Vec<&str> = bare.split('.').filter(|p| !p.is_empty()).collect();
    if parts.len() <= 2 {
        return bare.to_string();
    }
    let last = parts[parts.len() - 1];
    let second_last = parts[parts.len() - 2];
    let is_cc_second_level = last.len() == 2
        && matches!(
            second_last.to_ascii_lowercase().as_str(),
            "com" | "net" | "org" | "gov" | "edu" | "co" | "ac"
        );
    let keep = if is_cc_second_level && parts.len() >= 3 {
        3
    } else {
        2
    };
    parts[parts.len() - keep..].join(".")
}

/// Groups domain facet entries by `root_domain` while preserving descending count order.
pub(crate) fn group_domains_by_root(
    entries: &[session::FacetEntry],
    query: &str,
) -> Vec<DomainGroup> {
    let q = query.trim().to_ascii_lowercase();
    let mut order: Vec<String> = Vec::new();
    let mut map: std::collections::HashMap<String, Vec<session::FacetEntry>> =
        std::collections::HashMap::new();
    for entry in entries {
        if !q.is_empty()
            && !entry.label.to_ascii_lowercase().contains(&q)
            && !entry.token.to_ascii_lowercase().contains(&q)
        {
            continue;
        }
        let root = root_domain(&entry.token);
        if !map.contains_key(&root) {
            order.push(root.clone());
        }
        map.entry(root).or_default().push(entry.clone());
    }
    let mut groups: Vec<DomainGroup> = order
        .into_iter()
        .filter_map(|root| {
            let items = map.remove(&root)?;
            let total_count = items.iter().map(|e| e.count).sum();
            let total_errors = items.iter().map(|e| e.error_count).sum();
            Some(DomainGroup {
                root,
                total_count,
                total_errors,
                entries: items,
            })
        })
        .collect();
    groups.sort_by(|a, b| {
        b.total_count
            .cmp(&a.total_count)
            .then_with(|| a.root.cmp(&b.root))
    });
    groups
}

/// Left navigation sidebar for 3-column layout: Apps & Domains vertical tree.
pub(crate) fn facet_sidebar(
    facets: Signal<session::SessionFacets>,
    filter: Signal<String>,
    sidebar_width: Signal<f32>,
    sidebar_collapsed: Signal<bool>,
) -> impl IntoElement {
    let side_query = signal(String::new());
    let apps_open = signal(true);
    let domains_open = signal(true);
    let collapsed_roots = signal(std::collections::HashSet::<String>::new());
    let resize_origin = signal(SIDEBAR_WIDTH_DEFAULT);
    let resize_state = signal(ResizeHandleState::Idle);

    let filtered_apps = computed(move || -> Vec<session::FacetEntry> {
        let q = side_query.get().trim().to_ascii_lowercase();
        facets.with(|f| {
            f.apps
                .iter()
                .filter(|e| {
                    q.is_empty()
                        || e.label.to_ascii_lowercase().contains(&q)
                        || e.token.to_ascii_lowercase().contains(&q)
                })
                .cloned()
                .collect()
        })
    });

    let grouped_domains = computed(move || -> Vec<DomainGroup> {
        let q = side_query.get();
        facets.with(|f| group_domains_by_root(&f.domains, &q))
    });

    when(
        move || !sidebar_collapsed.get(),
        move || {
            view()
                .relative()
                .w(move || px(clamp_sidebar_px(sidebar_width.get())))
                .h_full()
                .flex_shrink_0()
                .flex_row()
                .bg(bg_chrome())
            .child(
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .overflow_hidden()
                    .child(
                        view()
                            .w_full()
                            .p_2()
                            .gap_1p5()
                            .border_b_1()
                            .border_b_color(border_subtle())
                            .child(
                                view().w_full().flex_row().items_center().gap_1().child(
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .relative()
                                        .child(
                                            input()
                                                .value(side_query)
                                                .placeholder(tr(
                                                    "过滤域名或应用…",
                                                    "Filter domains / apps…",
                                                ))
                                                .placeholder_color(move || color(fg_muted()))
                                                .w_full()
                                                .h(px(CONTROL_H))
                                                .pl_2()
                                                .pr(move || {
                                                    if side_query.get().is_empty() {
                                                        px(8.0)
                                                    } else {
                                                        px(24.0)
                                                    }
                                                })
                                                .rounded(px(RADIUS_MD))
                                                .bg(bg_code())
                                                .border_1()
                                                .border_color(border_control())
                                                .focus(|style| style.border_color(accent_blue()))
                                                .text_size(px(TEXT_SM))
                                                .text_color(fg_primary())
                                                .on_change(move |text: &str| {
                                                    side_query.set(text.to_string())
                                                }),
                                        )
                                        .child(when(
                                            move || !side_query.get().is_empty(),
                                            move || {
                                                button()
                                                    .absolute()
                                                    .right(px(3.0))
                                                    .top(px(3.0))
                                                    .w(px(ControlSize::Xs.height()))
                                                    .h(px(ControlSize::Xs.height()))
                                                    .rounded(px(RADIUS_SM))
                                                    .flex_row()
                                                    .items_center()
                                                    .justify_center()
                                                    .hover(|s| s.bg(bg_control_hover()))
                                                    .active(|s| s.bg(bg_control_active()))
                                                    .transition_colors()
                                                    .duration(move || motion_ms(MOTION_INSTANT_MS))
                                                    .ease_out()
                                                    .focus_visible(apply_focus_ring)
                                                    .accessibility_label(tr_opt("清空过滤", "Clear filter"))
                                                    .on_press(move |_: &()| {
                                                        side_query.set(String::new())
                                                    })
                                                    .child(
                                                        text("×")
                                                            .text_size(px(TEXT_MD))
                                                            .text_color(fg_muted()),
                                                    )
                                            },
                                        )),
                                ),
                            )
                            .child(
                                view()
                                    .w_full()
                                    .flex_row()
                                    .flex_wrap()
                                    .gap(px(3.0))
                                    .child(drawer_quick_chip(
                                        filter,
                                        "慢请求",
                                        "Slow",
                                        "slow:",
                                        SLOW_ALIASES,
                                        "过滤耗时 ≥ 1s 或进行中的慢请求 (slow:)",
                                        "Filter slow requests >= 1s (slow:)",
                                    ))
                                    .child(drawer_quick_chip(
                                        filter,
                                        "出错",
                                        "Error",
                                        "status:4xx|5xx|err",
                                        ERR_ALIASES,
                                        "过滤 4xx / 5xx / 连接失败请求 (status:4xx|5xx|err)",
                                        "Filter 4xx / 5xx / failed requests (status:4xx|5xx|err)",
                                    ))
                                    .child(drawer_quick_chip(
                                        filter,
                                        "JSON",
                                        "JSON",
                                        "type:json",
                                        JSON_ALIASES,
                                        "过滤 JSON 请求/响应 (type:json)",
                                        "Filter JSON traffic (type:json)",
                                    ))
                                    .child(drawer_quick_chip(
                                        filter,
                                        "星标",
                                        "Star",
                                        "pin:",
                                        PIN_ALIASES,
                                        "只看已星标请求 (pin:)",
                                        "Show starred requests only (pin:)",
                                    )),
                            ),
                    )
                    .child(
                        scroll().flex_1().w_full().min_h(px(0.0)).child(
                            view()
                                .w_full()
                                .p_1p5()
                                .gap_1()
                                .child(
                                    collapsible()
                                        .w_full()
                                        .open(move || apps_open.get())
                                        .reveal(true)
                                        .content(
                                            view().w_full().pt_1().child(
                                                when(
                                                    move || filtered_apps.with(Vec::is_empty),
                                                    move || {
                                                        let has_query =
                                                            !side_query.get().trim().is_empty();
                                                        let any_total =
                                                            facets.with(|f| !f.apps.is_empty());
                                                        text(if has_query && any_total {
                                                            tr("无匹配应用", "No matching apps")()
                                                        } else {
                                                            tr("暂无应用", "No apps yet")()
                                                        })
                                                        .px_2p5()
                                                        .py_1()
                                                        .text_size(px(TEXT_XS))
                                                        .text_color(fg_muted())
                                                    },
                                                )
                                                .otherwise(move || {
                                                    view().w_full().gap_0p5().child(each(
                                                        move || filtered_apps.get(),
                                                        |entry| entry.token.clone(),
                                                        move |entry, _| {
                                                            sidebar_app_row(entry, filter)
                                                        },
                                                    ))
                                                }),
                                            ),
                                        )
                                        .child(sidebar_section_header(
                                            "应用 / 来源",
                                            "Apps",
                                            move || filtered_apps.with(Vec::len),
                                            apps_open,
                                        )),
                                )
                                .child(view().h(px(4.0)))
                                .child(
                                    collapsible()
                                        .w_full()
                                        .open(move || domains_open.get())
                                        .reveal(true)
                                        .content(
                                            view().w_full().pt_1().child(
                                                when(
                                                    move || grouped_domains.with(Vec::is_empty),
                                                    move || {
                                                        let has_query =
                                                            !side_query.get().trim().is_empty();
                                                        let any_total =
                                                            facets.with(|f| !f.domains.is_empty());
                                                        text(if has_query && any_total {
                                                            tr("无匹配域名", "No matching domains")(
                                                            )
                                                        } else {
                                                            tr("暂无域名", "No domains yet")()
                                                        })
                                                        .px_2p5()
                                                        .py_1()
                                                        .text_size(px(TEXT_XS))
                                                        .text_color(fg_muted())
                                                    },
                                                )
                                                .otherwise(move || {
                                                    view().w_full().gap_0p5().child(each(
                                                        move || grouped_domains.get(),
                                                        |group| group.root.clone(),
                                                        move |group, _| {
                                                            sidebar_domain_group_view(
                                                                group,
                                                                filter,
                                                                side_query,
                                                                collapsed_roots,
                                                            )
                                                        },
                                                    ))
                                                }),
                                            ),
                                        )
                                        .child(sidebar_section_header(
                                            "域名",
                                            "Domains",
                                            move || {
                                                grouped_domains.with(|groups| {
                                                    groups
                                                        .iter()
                                                        .map(|g| g.entries.len())
                                                        .sum::<usize>()
                                                })
                                            },
                                            domains_open,
                                        )),
                                ),
                        ),
                    ),
            )
            .child(
                resize_handle()
                    .axis(Axis::Horizontal)
                    .edge(HandleEdge::Trailing)
                    .accessibility_label(tr_opt("调整侧栏宽度", "Resize sidebar width"))
                    .on_state_change(move |next: &ResizeHandleState| resize_state.set(*next))
                    .on_drag_start(move || {
                        resize_origin.set(clamp_sidebar_px(sidebar_width.get_untracked()));
                    })
                    .on_drag_move(move |offset: &Points| {
                        sidebar_width.set(clamp_sidebar_px(resize_origin.get() + offset.0));
                    })
                    .on_drag_end(move |offset: &Points| {
                        let next = clamp_sidebar_px(resize_origin.get() + offset.0);
                        sidebar_width.set(next);
                        let width_u32 = next.round() as u32;
                        let _ = save_config(|cfg| cfg.sidebar_width = width_u32);
                    })
                    .child(splitter_indicator(Axis::Horizontal, resize_state, 2.5)),
            )
        },
    )
}

fn sidebar_section_header(
    zh: &'static str,
    en: &'static str,
    count: impl Fn() -> usize + 'static,
    open: Signal<bool>,
) -> impl IntoElement {
    let langs = ui_langs();
    button()
        .w_full()
        .h(px(24.0))
        .px_1p5()
        .rounded(px(RADIUS_SM))
        .flex_row()
        .items_center()
        .gap_1()
        .hover(|s| s.bg(bg_control_hover()))
        .active(|s| s.bg(bg_control_active()))
        .transition_colors()
        .duration(move || motion_ms(MOTION_INSTANT_MS))
        .ease_out()
        .focus_visible(apply_focus_ring)
        .accessibility_label(tr_opt(zh, en))
        .on_press(move |_: &()| open.update(|v| *v = !*v))
        .child(ui_rotating_chevron(
            10.0,
            move || open.get(),
            || color(fg_muted()),
        ))
        .child(
            text(tr(zh, en))
                .flex_1()
                .min_w(px(0.0))
                .number_of_lines(1)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_muted()),
        )
        .child(
            mono(move || -> SharedString {
                let _ = langs;
                present::format_thousands(count() as u64).into()
            })
            .tabular_nums()
            .text_size(px(TEXT_XS))
            .text_color(fg_muted()),
        )
}

fn sidebar_error_pill(errors: impl Fn() -> usize + Copy + 'static) -> impl IntoElement {
    when(
        move || errors() > 0,
        move || {
            view()
                .h(px(15.0))
                .px_1()
                .flex_shrink_0()
                .rounded(px(RADIUS_SM))
                .bg(status_err_bg())
                .flex_row()
                .items_center()
                .justify_center()
                .child(
                    mono(move || -> SharedString {
                        present::format_thousands(errors() as u64).into()
                    })
                    .tabular_nums()
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(status_err_fg()),
                )
        },
    )
}

pub(crate) fn sidebar_app_row(
    entry: ReadSignal<session::FacetEntry>,
    filter: Signal<String>,
) -> impl IntoElement {
    let langs = ui_langs();
    let is_active = move || {
        let f = filter.get();
        let tok = entry.with(|e| e.token.clone());
        app_facet_active(&f, &tok)
    };
    let display_label = move || -> SharedString {
        let lang = ui_lang(langs);
        entry
            .with(|e| session::format_client_label(&e.label, lang))
            .into()
    };
    view()
        .w_full()
        .tooltip(
            tooltip().child(ui_tooltip_surface(
                text(display_label)
                    .text_size(px(TEXT_SM))
                    .text_color(fg_primary()),
            )),
        )
        .child(
            button()
                .selected(is_active)
                .w_full()
                .h(px(25.0))
                .pl_3()
                .pr_1p5()
                .rounded(px(RADIUS_SM))
                .flex_row()
                .items_center()
                .gap_1p5()
                .bg(move || {
                    if is_active() {
                        accent_blue_soft()
                    } else {
                        AdaptiveColor::TRANSPARENT
                    }
                })
                .hover(|s| {
                    s.bg(move || {
                        if is_active() {
                            accent_blue_soft_hover()
                        } else {
                            bg_control_hover()
                        }
                    })
                })
                .active(|s| s.bg(bg_control_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .accessibility_label(move || Some(display_label()))
                .on_press(move |_: &()| {
                    let tok = entry.with(|e| e.token.clone());
                    filter.set(toggle_app_facet(&filter.get(), &tok));
                })
                .child(facet_app_or_device_icon(entry, is_active))
                .child(
                    text(display_label)
                        .flex_1()
                        .min_w(px(0.0))
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .text_size(px(TEXT_SM))
                        .text_color(move || {
                            if is_active() {
                                accent_blue_text()
                            } else {
                                fg_primary()
                            }
                        }),
                )
                .child(sidebar_error_pill(move || entry.with(|e| e.error_count)))
                .child(
                    mono(move || -> SharedString {
                        entry
                            .with(|e| present::format_thousands(e.count as u64))
                            .into()
                    })
                    .tabular_nums()
                    .text_size(px(TEXT_XS))
                    .text_color(fg_muted()),
                ),
        )
}

pub(crate) fn sidebar_domain_group_view(
    group: ReadSignal<DomainGroup>,
    filter: Signal<String>,
    side_query: Signal<String>,
    collapsed_roots: Signal<std::collections::HashSet<String>>,
) -> impl IntoElement {
    let single = move || group.with(|g| g.entries.len() <= 1);
    let is_expanded = move || {
        !side_query.get().trim().is_empty()
            || !collapsed_roots.with(|set| group.with(|g| set.contains(&g.root)))
    };
    let root_active = move || {
        let f = filter.get();
        let root = group.with(|g| g.root.clone());
        root_domain_facet_active(&f, &root)
    };
    let root_muted = move || {
        let f = filter.get();
        let root = group.with(|g| g.root.clone());
        mute_facet_active(&f, &root)
    };
    when(single, move || {
        view().w_full().child(each(
            move || group.with(|g| g.entries.clone()),
            |entry| entry.token.clone(),
            move |entry, _| sidebar_domain_leaf_row(entry, false, filter),
        ))
    })
    .otherwise(move || {
        collapsible()
            .w_full()
            .open(is_expanded)
            .reveal(true)
            .content(view().w_full().pt_0p5().gap_0p5().child(each(
                move || group.with(|g| g.entries.clone()),
                |entry| entry.token.clone(),
                move |entry, _| sidebar_domain_leaf_row(entry, true, filter),
            )))
            .child(
                view()
                    .w_full()
                    .h(px(24.0))
                    .pl_1()
                    .pr_1()
                    .rounded(px(RADIUS_SM))
                    .flex_row()
                    .items_center()
                    .gap_0p5()
                    .bg(move || {
                        if root_active() {
                            accent_blue_soft()
                        } else if root_muted() {
                            status_err_bg()
                        } else {
                            AdaptiveColor::TRANSPARENT
                        }
                    })
                    .hover(|s| {
                        s.bg(move || {
                            if root_active() {
                                accent_blue_soft_hover()
                            } else if root_muted() {
                                status_err_bg_hover()
                            } else {
                                bg_control_hover()
                            }
                        })
                    })
                    .child(when(
                        move || side_query.get().trim().is_empty(),
                        move || {
                            button()
                                .w(px(18.0))
                                .h(px(ControlSize::Xs.height()))
                                .flex_shrink_0()
                                .rounded(px(RADIUS_SM))
                                .flex_row()
                                .items_center()
                                .justify_center()
                                .hover(|s| s.bg(bg_control_hover()))
                                .active(|s| s.bg(bg_control_active()))
                                .transition_colors()
                                .duration(MOTION_INSTANT_MS)
                                .ease_out()
                                .focus_visible(apply_focus_ring)
                                .accessibility_label(tr_opt("展开或收起子域", "Toggle subdomains"))
                                .on_press(move |_: &()| {
                                    let root = group.with(|g| g.root.clone());
                                    collapsed_roots.update(|set| {
                                        if !set.remove(&root) {
                                            set.insert(root);
                                        }
                                    });
                                })
                                .child(ui_rotating_chevron(10.0, is_expanded, || color(fg_muted())))
                        },
                    ))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .h_full()
                            .tooltip(label_tip(
                                "点击只看此根域及其子域",
                                "Click to filter this root domain & subdomains",
                            ))
                            .child(
                                button()
                                    .selected(root_active)
                                    .w_full()
                                    .h_full()
                                    .flex_row()
                                    .items_center()
                                    .gap_1()
                                    .rounded(px(RADIUS_SM))
                                    .active(|s| s.bg(bg_control_active()))
                                    .transition_colors()
                                    .duration(MOTION_INSTANT_MS)
                                    .ease_out()
                                    .focus_visible(apply_focus_ring)
                                    .accessibility_label(move || {
                                        Some(group.with(|g| g.root.clone()).into())
                                    })
                                    .on_press(move |_: &()| {
                                        let root = group.with(|g| g.root.clone());
                                        filter.set(toggle_root_domain_facet(&filter.get(), &root));
                                    })
                                    .child(
                                        mono(move || -> SharedString {
                                            group.with(|g| g.root.clone()).into()
                                        })
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .number_of_lines(1)
                                        .text_overflow(EllipsizeMode::Middle)
                                        .text_size(px(TEXT_SM))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(
                                            move || {
                                                if root_active() {
                                                    accent_blue_text()
                                                } else if root_muted() {
                                                    status_err_fg()
                                                } else {
                                                    fg_secondary()
                                                }
                                            },
                                        ),
                                    )
                                    .child(sidebar_error_pill(move || {
                                        group.with(|g| g.total_errors)
                                    }))
                                    .child(
                                        mono(move || -> SharedString {
                                            group
                                                .with(|g| {
                                                    present::format_thousands(g.total_count as u64)
                                                })
                                                .into()
                                        })
                                        .tabular_nums()
                                        .text_size(px(TEXT_XS))
                                        .text_color(fg_muted()),
                                    ),
                            ),
                    )
                    .child({
                        let langs = ui_langs();
                        view()
                            .flex_shrink_0()
                            .tooltip(
                                tooltip().child(ui_tooltip_surface(
                                    text(move || -> SharedString {
                                        let lang = ui_lang(langs);
                                        if root_muted() {
                                            t(lang, "取消屏蔽整组域名", "Unmute root domain group")
                                        } else {
                                            t(lang, "屏蔽整组域名", "Mute root domain group")
                                        }
                                        .into()
                                    })
                                    .text_size(px(TEXT_SM))
                                    .text_color(fg_primary()),
                                )),
                            )
                            .child(
                                button()
                                    .w(px(ControlSize::Xs.height()))
                                    .h(px(ControlSize::Xs.height()))
                                    .rounded(px(RADIUS_SM))
                                    .flex_row()
                                    .items_center()
                                    .justify_center()
                                    .hover(|s| s.bg(bg_control_hover()))
                                    .active(|s| s.bg(bg_control_active()))
                                    .transition_colors()
                                    .duration(MOTION_INSTANT_MS)
                                    .ease_out()
                                    .focus_visible(apply_focus_ring)
                                    .accessibility_label(tr_opt(
                                        "屏蔽或取消屏蔽整组域名",
                                        "Toggle mute root domain",
                                    ))
                                    .on_press(move |_: &()| {
                                        let root = group.with(|g| g.root.clone());
                                        filter.set(toggle_mute_facet(&filter.get(), &root));
                                    })
                                    .child(text("×").text_size(px(TEXT_SM)).text_color(
                                        move || {
                                            if root_muted() {
                                                status_err_fg()
                                            } else {
                                                fg_muted()
                                            }
                                        },
                                    )),
                            )
                    }),
            )
    })
}

fn sidebar_domain_leaf_row(
    entry: ReadSignal<session::FacetEntry>,
    indented: bool,
    filter: Signal<String>,
) -> impl IntoElement {
    let is_active = move || {
        let f = filter.get();
        let tok = entry.with(|e| e.token.clone());
        host_facet_active(&f, &tok)
    };
    let is_muted = move || {
        let f = filter.get();
        let tok = entry.with(|e| e.token.clone());
        mute_facet_active(&f, &tok)
    };
    view()
        .w_full()
        .h(px(25.0))
        .pl(if indented { px(18.0) } else { px(10.0) })
        .pr_1()
        .rounded(px(RADIUS_SM))
        .flex_row()
        .items_center()
        .gap_1()
        .bg(move || {
            if is_active() {
                accent_blue_soft()
            } else if is_muted() {
                status_err_bg()
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .hover(|s| {
            s.bg(move || {
                if is_active() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .tooltip(
                    tooltip().child(ui_tooltip_surface(
                        mono(move || -> SharedString { entry.with(|e| e.label.clone()).into() })
                            .text_size(px(TEXT_SM))
                            .text_color(fg_primary()),
                    )),
                )
                .child(
                    button()
                        .selected(is_active)
                        .w_full()
                        .h_full()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .rounded(px(RADIUS_SM))
                        .active(|s| s.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .accessibility_label(move || Some(entry.with(|e| e.label.clone()).into()))
                        .on_press(move |_: &()| {
                            let tok = entry.with(|e| e.token.clone());
                            filter.set(toggle_host_facet(&filter.get(), &tok));
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
                                    } else if is_muted() {
                                        status_err_fg()
                                    } else {
                                        fg_muted()
                                    }
                                }),
                        )
                        .child(
                            mono(move || -> SharedString {
                                entry.with(|e| e.label.clone()).into()
                            })
                            .flex_1()
                            .min_w(px(0.0))
                            .number_of_lines(1)
                            .text_overflow(EllipsizeMode::Middle)
                            .text_size(px(TEXT_SM))
                            .text_color(move || {
                                if is_active() {
                                    accent_blue_text()
                                } else if is_muted() {
                                    status_err_fg()
                                } else {
                                    fg_primary()
                                }
                            }),
                        )
                        .child(sidebar_error_pill(move || entry.with(|e| e.error_count)))
                        .child(
                            mono(move || -> SharedString {
                                entry
                                    .with(|e| present::format_thousands(e.count as u64))
                                    .into()
                            })
                            .tabular_nums()
                            .text_size(px(TEXT_XS))
                            .text_color(fg_muted()),
                        ),
                ),
        )
        .child({
            let langs = ui_langs();
            view()
                .flex_shrink_0()
                .tooltip(
                    tooltip().child(ui_tooltip_surface(
                        text(move || -> SharedString {
                            let lang = ui_lang(langs);
                            if is_muted() {
                                t(lang, "取消屏蔽此域名", "Unmute this domain")
                            } else {
                                t(lang, "屏蔽此域名", "Mute this domain")
                            }
                            .into()
                        })
                        .text_size(px(TEXT_SM))
                        .text_color(fg_primary()),
                    )),
                )
                .child(
                    button()
                        .w(px(ControlSize::Xs.height()))
                        .h(px(ControlSize::Xs.height()))
                        .rounded(px(RADIUS_SM))
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(bg_control_hover()))
                        .active(|s| s.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .accessibility_label(tr_opt("屏蔽或取消屏蔽此域名", "Toggle mute domain"))
                        .on_press(move |_: &()| {
                            let tok = entry.with(|e| e.token.clone());
                            filter.set(toggle_mute_facet(&filter.get(), &tok));
                        })
                        .child(text("×").text_size(px(TEXT_SM)).text_color(move || {
                            if is_muted() {
                                status_err_fg()
                            } else {
                                fg_muted()
                            }
                        })),
                )
        })
}
