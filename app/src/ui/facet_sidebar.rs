#[allow(unused_imports)]
use super::*;

#[derive(Clone, PartialEq)]
pub(crate) struct DomainGroup {
    pub(crate) root: String,
    pub(crate) total_count: usize,
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
            Some(DomainGroup {
                root,
                total_count,
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
) -> impl IntoElement {
    let side_query = signal(String::new());
    let apps_open = signal(true);
    let domains_open = signal(true);
    let collapsed_roots = signal(std::collections::HashSet::<String>::new());

    let filtered_apps = move || -> Vec<session::FacetEntry> {
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
    };

    let grouped_domains = move || -> Vec<DomainGroup> {
        let q = side_query.get();
        facets.with(|f| group_domains_by_root(&f.domains, &q))
    };

    view()
        .w(px(246.0))
        .h_full()
        .flex_shrink_0()
        .bg(bg_chrome())
        .border_r_1()
        .border_r_color(border_strong())
        .child(
            view()
                .w_full()
                .p_2()
                .gap_1p5()
                .border_b_1()
                .border_b_color(border_subtle())
                .child(
                    view()
                        .w_full()
                        .relative()
                        .child(
                            input()
                                .value(side_query)
                                .placeholder(tr("过滤域名或应用…", "Filter domains / apps…"))
                                .placeholder_color(move || color(fg_muted()))
                                .w_full()
                                .h(px(28.0))
                                .pl_2()
                                .pr(move || {
                                    if side_query.get().is_empty() {
                                        px(8.0)
                                    } else {
                                        px(24.0)
                                    }
                                })
                                .rounded(px(6.0))
                                .bg(bg_code())
                                .border_1()
                                .border_color(border_control())
                                .focus(|style| style.border_color(accent_blue()))
                                .text_size(px(11.5))
                                .text_color(fg_primary())
                                .on_change(move |text: &str| side_query.set(text.to_string())),
                        )
                        .child(when(
                            move || !side_query.get().is_empty(),
                            move || {
                                view()
                                    .absolute()
                                    .right(px(3.0))
                                    .top(px(3.0))
                                    .w(px(22.0))
                                    .h(px(22.0))
                                    .rounded(px(4.0))
                                    .flex_row()
                                    .items_center()
                                    .justify_center()
                                    .cursor(Cursor::Pointer)
                                    .hover(|s| s.bg(bg_control_hover()))
                                    .on_press(move || side_query.set(String::new()))
                                    .child(
                                        text("×")
                                            .text_size(px(13.0))
                                            .text_color(fg_muted()),
                                    )
                            },
                        )),
                )
                .child(
                    view()
                        .w_full()
                        .flex_row()
                        .flex_wrap()
                        .gap_1()
                        .child(drawer_quick_chip(
                            filter,
                            "慢请求",
                            "Slow",
                            "slow:",
                            SLOW_ALIASES,
                            "筛选耗时 ≥ 1s 或进行中的慢请求 (slow:)",
                            "Filter slow requests >= 1s (slow:)",
                        ))
                        .child(drawer_quick_chip(
                            filter,
                            "出错",
                            "Errors",
                            "status:4xx|5xx|err",
                            ERR_ALIASES,
                            "筛选 4xx / 5xx / 连接失败请求 (status:4xx|5xx|err)",
                            "Filter 4xx / 5xx / failed requests (status:4xx|5xx|err)",
                        ))
                        .child(drawer_quick_chip(
                            filter,
                            "JSON",
                            "JSON",
                            "type:json",
                            JSON_ALIASES,
                            "筛选 JSON 请求/响应 (type:json)",
                            "Filter JSON traffic (type:json)",
                        ))
                        .child(drawer_quick_chip(
                            filter,
                            "星标",
                            "Starred",
                            "pin:",
                            PIN_ALIASES,
                            "只看已星标请求 (pin:)",
                            "Show starred requests only (pin:)",
                        )),
                ),
        )
        .child(
            scroll()
                .flex_1()
                .w_full()
                .min_h(px(0.0))
                .child(
                    view()
                        .w_full()
                        .p_1p5()
                        .gap_1()
                        .child(sidebar_section_header(
                            "应用 / 来源",
                            "Apps",
                            move || facets.with(|f| f.apps.len()),
                            apps_open,
                        ))
                        .child(when(
                            move || apps_open.get(),
                            move || {
                                view().w_full().gap_0p5().child(each(
                                    filtered_apps,
                                    |entry| entry.token.clone(),
                                    move |entry, _| sidebar_app_row(entry, filter),
                                ))
                            },
                        ))
                        .child(view().h(px(4.0)))
                        .child(sidebar_section_header(
                            "域名",
                            "Domains",
                            move || facets.with(|f| f.domains.len()),
                            domains_open,
                        ))
                        .child(when(
                            move || domains_open.get(),
                            move || {
                                view().w_full().gap_0p5().child(each(
                                    grouped_domains,
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
                            },
                        )),
                ),
        )
}

fn sidebar_section_header(
    zh: &'static str,
    en: &'static str,
    count: impl Fn() -> usize + 'static,
    open: Signal<bool>,
) -> impl IntoElement {
    let langs = ui_langs();
    view()
        .w_full()
        .h(px(24.0))
        .px_1p5()
        .rounded(px(5.0))
        .flex_row()
        .items_center()
        .gap_1()
        .cursor(Cursor::Pointer)
        .hover(|s| s.bg(bg_control_hover()))
        .on_press(move || open.update(|v| *v = !*v))
        .child(glyph_px(
            10.0,
            move || {
                if open.get() {
                    Glyph::Chevron
                } else {
                    Glyph::ChevronRight
                }
            },
            || color(fg_muted()),
        ))
        .child(
            text(tr(zh, en))
                .flex_1()
                .min_w(px(0.0))
                .number_of_lines(1)
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_muted()),
        )
        .child(
            mono(move || -> SharedString {
                let _ = langs;
                count().to_string().into()
            })
            .tabular_nums()
            .text_size(px(10.5))
            .text_color(fg_muted()),
        )
}

pub(crate) fn sidebar_app_row(
    entry: ReadSignal<session::FacetEntry>,
    filter: Signal<String>,
) -> impl IntoElement {
    let is_active = move || {
        let f = filter.get();
        let tok = entry.with(|e| e.token.clone());
        app_facet_active(&f, &tok)
    };
    view()
        .w_full()
        .h(px(25.0))
        .pl_3()
        .pr_1p5()
        .rounded(px(5.0))
        .flex_row()
        .items_center()
        .gap_1p5()
        .cursor(Cursor::Pointer)
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
        .on_press(move || {
            let tok = entry.with(|e| e.token.clone());
            filter.set(toggle_app_facet(&filter.get(), &tok));
        })
        .child(
            view()
                .w(px(5.0))
                .h(px(5.0))
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
            text(move || -> SharedString { entry.with(|e| e.label.clone()).into() })
                .flex_1()
                .min_w(px(0.0))
                .number_of_lines(1)
                .text_overflow(EllipsizeMode::Tail)
                .text_size(px(11.5))
                .text_color(move || {
                    if is_active() {
                        accent_blue_text()
                    } else {
                        fg_primary()
                    }
                }),
        )
        .child(
            mono(move || -> SharedString { entry.with(|e| e.count.to_string()).into() })
                .tabular_nums()
                .text_size(px(10.5))
                .text_color(fg_muted()),
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
    when(single, move || {
        let entry_opt = group.with(|g| g.entries.first().cloned());
        view()
            .w_full()
            .children(entry_opt.map(|item| sidebar_domain_leaf_row(item, false, filter)))
    })
    .otherwise(move || {
        view()
            .w_full()
            .gap_0p5()
            .child(
                view()
                    .w_full()
                    .h(px(24.0))
                    .pl_1p5()
                    .pr_1p5()
                    .rounded(px(5.0))
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .cursor(Cursor::Pointer)
                    .hover(|s| s.bg(bg_control_hover()))
                    .on_press(move || {
                        let root = group.with(|g| g.root.clone());
                        collapsed_roots.update(|set| {
                            if !set.remove(&root) {
                                set.insert(root);
                            }
                        });
                    })
                    .child(glyph_px(
                        9.5,
                        move || {
                            if is_expanded() {
                                Glyph::Chevron
                            } else {
                                Glyph::ChevronRight
                            }
                        },
                        || color(fg_muted()),
                    ))
                    .child(
                        mono(move || -> SharedString { group.with(|g| g.root.clone()).into() })
                            .flex_1()
                            .min_w(px(0.0))
                            .number_of_lines(1)
                            .text_overflow(EllipsizeMode::Middle)
                            .text_size(px(11.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg_secondary()),
                    )
                    .child(
                        mono(move || -> SharedString {
                            group.with(|g| g.total_count.to_string()).into()
                        })
                        .tabular_nums()
                        .text_size(px(10.5))
                        .text_color(fg_muted()),
                    ),
            )
            .child(when(is_expanded, move || {
                view().w_full().gap_0p5().child(each(
                    move || group.with(|g| g.entries.clone()),
                    |entry| entry.token.clone(),
                    move |entry, _| sidebar_domain_leaf_row_sig(entry, true, filter),
                ))
            }))
    })
}

fn sidebar_domain_leaf_row(
    entry: session::FacetEntry,
    indented: bool,
    filter: Signal<String>,
) -> impl IntoElement {
    let sig = signal(entry);
    sidebar_domain_leaf_row_sig(sig.read_only(), indented, filter)
}

fn sidebar_domain_leaf_row_sig(
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
        .rounded(px(5.0))
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
                .flex_row()
                .items_center()
                .gap_1p5()
                .cursor(Cursor::Pointer)
                .on_press(move || {
                    let tok = entry.with(|e| e.token.clone());
                    filter.set(toggle_host_facet(&filter.get(), &tok));
                })
                .child(
                    view()
                        .w(px(5.0))
                        .h(px(5.0))
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
                    mono(move || -> SharedString { entry.with(|e| e.label.clone()).into() })
                        .flex_1()
                        .min_w(px(0.0))
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Middle)
                        .text_size(px(11.5))
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
                .child(
                    mono(move || -> SharedString { entry.with(|e| e.count.to_string()).into() })
                        .tabular_nums()
                        .text_size(px(10.5))
                        .text_color(fg_muted()),
                ),
        )
        .child(
            view()
                .w(px(16.0))
                .h(px(16.0))
                .flex_shrink_0()
                .rounded(px(3.0))
                .flex_row()
                .items_center()
                .justify_center()
                .cursor(Cursor::Pointer)
                .hover(|s| s.bg(bg_control_hover()))
                .tooltip(label_tip("屏蔽此域名", "Mute this domain"))
                .on_press(move || {
                    let tok = entry.with(|e| e.token.clone());
                    filter.set(toggle_mute_facet(&filter.get(), &tok));
                })
                .child(
                    text("×")
                        .text_size(px(11.5))
                        .text_color(move || {
                            if is_muted() {
                                status_err_fg()
                            } else {
                                fg_muted()
                            }
                        }),
                ),
        )
}
