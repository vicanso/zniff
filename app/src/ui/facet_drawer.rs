#[allow(unused_imports)]
use super::*;

pub(crate) fn pane_splitter(
    width: Signal<f32>,
    height: Signal<f32>,
    layout_mode: Signal<config::LayoutMode>,
    sidebar_width: Signal<f32>,
    sidebar_collapsed: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let row_controls = controls.clone();
    let col_controls = controls;
    when(
        move || layout_mode.get() == config::LayoutMode::TopBottom,
        move || row_splitter(height, row_controls.clone()),
    )
    .otherwise(move || {
        column_splitter(
            width,
            layout_mode,
            sidebar_width,
            sidebar_collapsed,
            col_controls.clone(),
        )
    })
}

pub(crate) fn row_splitter(height: Signal<f32>, controls: Arc<Controls>) -> impl IntoElement {
    let origin = signal(0.0_f32);
    let controls_release = controls.clone();
    let controls_cancel = controls;
    let hot = signal(false);
    let dragging = signal(false);
    view()
        .w_full()
        .h(px(6.0))
        .flex_shrink_0()
        .flex_col()
        .justify_center()
        .cursor(Cursor::RowResize)
        .claim_touches(TouchClaim::on_start())
        .keep_touches(true)
        .on_mouse_enter(move |_: &MouseEvent| hot.set(true))
        .on_mouse_leave(move |_: &MouseEvent| hot.set(false))
        .on_responder_grant(move |_event: &ResponderEvent| {
            dragging.set(true);
            origin.set(clamp_list_height_for_window(
                height.get(),
                window().height.0,
            ));
        })
        .on_responder_move(move |event: &ResponderEvent| {
            height.set(clamp_list_height_for_window(
                origin.get() + event.gesture.delta.y.0,
                window().height.0,
            ));
        })
        .on_responder_release(move |_event: &ResponderEvent| {
            dragging.set(false);
            store_list_height(
                clamp_list_height_for_window(height.get(), window().height.0),
                &controls_release,
            );
        })
        .on_responder_terminate(move |_event: &ResponderEvent| {
            dragging.set(false);
            store_list_height(
                clamp_list_height_for_window(height.get(), window().height.0),
                &controls_cancel,
            );
        })
        .child(
            view()
                .w_full()
                .h(move || {
                    if hot.get() || dragging.get() {
                        px(3.0)
                    } else {
                        px(1.0)
                    }
                })
                .bg(move || {
                    if hot.get() || dragging.get() {
                        accent_blue()
                    } else {
                        border_strong()
                    }
                }),
        )
}

pub(crate) fn column_splitter(
    width: Signal<f32>,
    layout_mode: Signal<config::LayoutMode>,
    sidebar_width: Signal<f32>,
    sidebar_collapsed: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let origin = signal(0.0_f32);
    let controls_release = controls.clone();
    let controls_cancel = controls;
    let hot = signal(false);
    let dragging = signal(false);
    let clamped_width = move |target: f32| {
        let mode = layout_mode.get();
        let side = effective_sidebar_px(mode, sidebar_width.get(), sidebar_collapsed.get());
        clamp_list_px_for_layout(target, window().width.0, mode, side)
    };
    view()
        .w(px(6.0))
        .flex_shrink_0()
        .self_stretch()
        .flex_row()
        .justify_center()
        .cursor(Cursor::ColResize)
        .claim_touches(TouchClaim::on_start())
        .keep_touches(true)
        .on_mouse_enter(move |_: &MouseEvent| hot.set(true))
        .on_mouse_leave(move |_: &MouseEvent| hot.set(false))
        .on_responder_grant(move |_event: &ResponderEvent| {
            dragging.set(true);
            origin.set(clamped_width(width.get()));
        })
        .on_responder_move(move |event: &ResponderEvent| {
            width.set(clamped_width(origin.get() + event.gesture.delta.x.0));
        })
        .on_responder_release(move |_event: &ResponderEvent| {
            dragging.set(false);
            store_list_width(clamped_width(width.get()), &controls_release);
        })
        .on_responder_terminate(move |_event: &ResponderEvent| {
            dragging.set(false);
            store_list_width(clamped_width(width.get()), &controls_cancel);
        })
        .child(
            view()
                .w(move || {
                    if hot.get() || dragging.get() {
                        px(3.0)
                    } else {
                        px(1.0)
                    }
                })
                .h_full()
                .bg(move || {
                    if hot.get() || dragging.get() {
                        accent_blue()
                    } else {
                        border_strong()
                    }
                }),
        )
}

pub(crate) fn store_list_width(width: f32, controls: &Controls) {
    let stored = clamp_list_px(width).round() as u32;
    if let Err(err) = save_config(|config| config.list_width = stored) {
        set_status(
            controls,
            StatusMsg::PrefUnsaved {
                name: PrefName::List,
                err: err.to_string(),
            },
        );
    }
}

pub(crate) fn store_list_height(height: f32, controls: &Controls) {
    let stored = clamp_list_height(height).round() as u32;
    if let Err(err) = save_config(|config| config.list_height = stored) {
        set_status(
            controls,
            StatusMsg::PrefUnsaved {
                name: PrefName::List,
                err: err.to_string(),
            },
        );
    }
}

pub(crate) fn merge_secondary_filter(main: &str, secondary: &str) -> String {
    let norm_main = crate::session::normalize_filter_pipes(main);
    let norm_sec = crate::session::normalize_filter_pipes(secondary);
    let mut out: Vec<String> = norm_main
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();
    for tok in norm_sec.split_whitespace().filter(|t| !t.is_empty()) {
        if !out
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(tok))
        {
            out.push(tok.to_string());
        }
    }
    out.join(" ")
}

fn list_clear_button(on_clear: Rc<dyn Fn()>) -> impl IntoElement {
    view()
        .role(AccessibilityRole::Button)
        .w(px(CONTROL_H))
        .h(px(CONTROL_H))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(RADIUS_MD))
        .bg(AdaptiveColor::TRANSPARENT)
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .label(tr("清空会话", "Clear session"))
        .tooltip(label_tip(
            "清空会话 (Cmd/Ctrl+K)",
            "Clear session (Cmd/Ctrl+K)",
        ))
        .on_press(move || on_clear())
        .child(glyph_px(
            ControlSize::Sm.icon_size(),
            || Glyph::Trash,
            || color(fg_secondary()),
        ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn flow_list(
    rows: Signal<Vec<Arc<RowLine>>>,
    facets: Signal<session::SessionFacets>,
    facets_open: Signal<bool>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
    compose: Compose,
    empty_state: EmptyFilterState,
    profiles: ProfileSignals,
    columns: Signal<ListColumns>,
    sort: Signal<Option<ListSort>>,
    controls: Arc<Controls>,
    shown: Shown,
    baseline: Signal<Option<u64>>,
    filter_focus: ElementRef,
    on_clear: Rc<dyn Fn()>,
) -> impl IntoElement {
    let width = profiles.list_width;
    let height = profiles.list_height;
    let layout_mode = profiles.layout_mode;
    let sidebar_width = profiles.sidebar_width;
    let sidebar_collapsed = profiles.sidebar_collapsed;
    let main_filter = empty_state.main_filter;
    let list_filter = empty_state.list_filter;
    let row_controls = controls.clone();
    let merge_controls = controls.clone();
    let column_controls = controls;
    let picked = signal(None::<u64>);
    let menu_open = signal(false);
    let menu_at = signal(Point::<Points>::default());
    let facet_tab = signal(0_u8);
    let effective_width = move || {
        let mode = layout_mode.get();
        let side = effective_sidebar_px(mode, sidebar_width.get(), sidebar_collapsed.get());
        clamp_list_px_for_layout(width.get(), window().width.0, mode, side)
    };
    let effective_height = move || clamp_list_height_for_window(height.get(), window().height.0);
    let is_top_bottom = move || layout_mode.get() == config::LayoutMode::TopBottom;
    view()
        .w(move || -> DefiniteLength {
            if is_top_bottom() {
                relative(1.0)
            } else {
                px(effective_width()).into()
            }
        })
        .h(move || -> DefiniteLength {
            if is_top_bottom() {
                px(effective_height()).into()
            } else {
                relative(1.0)
            }
        })
        .flex_shrink_0()
        .overflow_hidden()
        .bg(bg_list())
        .child(session_tab_bar())
        .child(
            view()
                .w_full()
                .h(px(46.0))
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_1p5()
                .pl(px(LIST_INSET))
                .pr(px(LIST_PAD_END))
                .bg(bg_chrome())
                .border_b_1()
                .border_b_color(border_strong())
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .relative()
                        .flex_row()
                        .items_center()
                        .child(filter_assist_input(
                            list_filter,
                            tr(
                                "在当前结果中再次过滤 (主机、路径、状态、body:…)",
                                "Filter current results (host, path, status, body:…)",
                            ),
                            30.0,
                            28.0,
                            filter_focus,
                        ))
                        .child(when(
                            move || !list_filter.get().is_empty(),
                            move || {
                                view()
                                    .role(AccessibilityRole::Button)
                                    .absolute()
                                    .right(px(4.0))
                                    .top(px(4.0))
                                    .w(px(ControlSize::Xs.height()))
                                    .h(px(ControlSize::Xs.height()))
                                    .flex_shrink_0()
                                    .flex_row()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(RADIUS_SM))
                                    .hover(|style| style.bg(bg_control_hover()))
                                    .active(|style| style.bg(bg_control_active()))
                                    .transition_colors()
                                    .duration(MOTION_INSTANT_MS)
                                    .ease_out()
                                    .focus_visible(apply_focus_ring)
                                    .label(tr("清空二次过滤", "Clear secondary filter"))
                                    .tooltip(label_tip("清空二次过滤", "Clear secondary filter"))
                                    .on_press(move || list_filter.set(String::new()))
                                    .child(
                                        control_text("×")
                                            .text_size(px(TEXT_MD))
                                            .text_color(fg_secondary()),
                                    )
                            },
                        )),
                )
                .child(when(
                    move || !list_filter.get().trim().is_empty(),
                    move || {
                        let merge_controls = merge_controls.clone();
                        view()
                            .role(AccessibilityRole::Button)
                            .h(px(CONTROL_H))
                            .px_2()
                            .flex_shrink_0()
                            .flex_row()
                            .items_center()
                            .justify_center()
                            .rounded(px(RADIUS_MD))
                            .border_1()
                            .border_color(accent_blue_border())
                            .bg(accent_blue_soft())
                            .hover(|style| style.bg(accent_blue_soft_hover()))
                            .active(|style| style.bg(bg_control_active()))
                            .transition_colors()
                            .duration(MOTION_INSTANT_MS)
                            .ease_out()
                            .focus_visible(apply_focus_ring)
                            .label(tr("并入主过滤", "Merge"))
                            .tooltip(label_tip(
                                "将当前二次筛选并入顶栏主过滤条件并可保存进预设",
                                "Merge secondary filter into main filter",
                            ))
                            .on_press(move || {
                                let merged =
                                    merge_secondary_filter(&main_filter.get(), &list_filter.get());
                                main_filter.set(merged.clone());
                                list_filter.set(String::new());
                                save_config_or_notify(&merge_controls, PrefName::Filter, |cfg| {
                                    cfg.filter = merged
                                });
                            })
                            .child(
                                control_text(tr("并入主过滤", "Merge"))
                                    .text_size(px(TEXT_SM))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(accent_blue_text()),
                            )
                    },
                ))
                .child(column_menu(columns, column_controls))
                .child(list_clear_button(on_clear)),
        )
        .child(filter_syntax_error_strip(list_filter))
        .child(when(
            move || layout_mode.get() == config::LayoutMode::TopBottom || sidebar_collapsed.get(),
            move || facet_quick_bar(facets, facets_open, facet_tab, list_filter),
        ))
        .child(tls_failure_banner_from_context())
        .child(
            view()
                .flex_1()
                .min_h(px(0.0))
                .relative()
                .child(list_menu(
                    rows,
                    selected,
                    paused,
                    compose,
                    columns,
                    sort,
                    row_controls,
                    empty_state,
                    shown,
                    baseline,
                    profiles,
                    picked,
                    menu_open,
                    menu_at,
                ))
                .child(when(
                    move || {
                        (layout_mode.get() == config::LayoutMode::TopBottom
                            || sidebar_collapsed.get())
                            && facets_open.get()
                    },
                    move || {
                        view()
                            .absolute()
                            .inset_0()
                            .child(
                                view()
                                    .absolute()
                                    .inset_0()
                                    .bg(backdrop_scrim_soft())
                                    .on_press(move || facets_open.set(false)),
                            )
                            .child(
                                view()
                                    .absolute()
                                    .top(px(0.0))
                                    .left_0()
                                    .right_0()
                                    .bg(bg_card())
                                    .border_b_1()
                                    .border_b_color(border_strong())
                                    .shadow_md()
                                    .child(facet_drawer(
                                        facets,
                                        facets_open,
                                        facet_tab,
                                        list_filter,
                                    )),
                            )
                    },
                )),
        )
}

pub(crate) fn facet_tab_has_filter(filter: &str, tab: u8) -> bool {
    if tab == 0 {
        !extract_host_or(filter).is_empty() || !extract_exclude_or(filter).is_empty()
    } else {
        !extract_app_or(filter).is_empty()
    }
}

pub(crate) fn facet_quick_bar(
    facets: Signal<session::SessionFacets>,
    facets_open: Signal<bool>,
    facet_tab: Signal<u8>,
    filter: Signal<String>,
) -> impl IntoElement {
    let tab_btn = move |idx: u8,
                        zh: &'static str,
                        en: &'static str,
                        count_fn: fn(&session::SessionFacets) -> usize| {
        let langs = ui_langs();
        let open_tab = move || facets_open.get() && facet_tab.get() == idx;
        let highlighted = move || open_tab() || filter.with(|f| facet_tab_has_filter(f, idx));
        view()
            .role(AccessibilityRole::Button)
            .h(px(ControlSize::Xs.height()))
            .px_2()
            .flex_shrink_0()
            .rounded(px(RADIUS_SM))
            .flex_row()
            .items_center()
            .gap_1()
            .bg(move || {
                if highlighted() {
                    accent_blue_soft()
                } else {
                    bg_control()
                }
            })
            .border_1()
            .border_color(move || {
                if highlighted() {
                    accent_blue_border()
                } else {
                    border_control()
                }
            })
            .hover(|style| {
                style.bg(move || {
                    if highlighted() {
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
            .on_press(move || {
                if facets_open.get() && facet_tab.get() == idx {
                    facets_open.set(false);
                } else {
                    facet_tab.set(idx);
                    facets_open.set(true);
                }
            })
            .child(
                control_text(move || -> SharedString {
                    let count = present::format_thousands(facets.with(count_fn) as u64);
                    format!("{} ({count})", t(ui_lang(langs), zh, en)).into()
                })
                .tabular_nums()
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if highlighted() {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
            )
            .child(glyph_px(
                10.0,
                move || {
                    if open_tab() {
                        Glyph::ChevronUp
                    } else {
                        Glyph::Chevron
                    }
                },
                move || {
                    if highlighted() {
                        color(accent_blue_text())
                    } else {
                        color(fg_muted())
                    }
                },
            ))
    };
    view()
        .w_full()
        .h(px(30.0))
        .flex_shrink_0()
        .bg(bg_chrome())
        .border_b_1()
        .border_b_color(border_strong())
        .pl(px(LIST_INSET))
        .pr(px(LIST_PAD_END))
        .flex_row()
        .items_center()
        .gap_1()
        .overflow_hidden()
        .child(tab_btn(0, "域名", "Domains", |f| f.domains.len()))
        .child(tab_btn(1, "应用", "Apps", |f| f.apps.len()))
        .child(view().w(px(1.0)).h(px(14.0)).mx_0p5().bg(border_subtle()))
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
            "Error",
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
            "Star",
            "pin:",
            PIN_ALIASES,
            "只看已星标请求 (pin:)",
            "Show starred requests only (pin:)",
        ))
}

pub(crate) fn facet_drawer(
    facets: Signal<session::SessionFacets>,
    facets_open: Signal<bool>,
    facet_tab: Signal<u8>,
    filter: Signal<String>,
) -> impl IntoElement {
    let drawer_query = signal(String::new());
    let collapsed_roots = signal(std::collections::HashSet::<String>::new());
    let items_list = move || -> Vec<session::FacetEntry> {
        let q = drawer_query.get().trim().to_ascii_lowercase();
        if facet_tab.get() == 0 {
            facets.with(|f| {
                f.domains
                    .iter()
                    .filter(|e| {
                        q.is_empty()
                            || e.label.to_ascii_lowercase().contains(&q)
                            || e.token.to_ascii_lowercase().contains(&q)
                    })
                    .cloned()
                    .collect()
            })
        } else {
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
        }
    };
    let grouped_domains = move || -> Vec<DomainGroup> {
        let q = drawer_query.get();
        facets.with(|f| group_domains_by_root(&f.domains, &q))
    };
    view()
        .w_full()
        .pl(px(LIST_INSET))
        .py_2()
        .gap_1p5()
        .child(
            view()
                .pr(px(LIST_PAD_END))
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .relative()
                        .child(
                            input()
                                .value(drawer_query)
                                .placeholder(tr("筛选域名或应用…", "Filter domains / apps…"))
                                .placeholder_color(move || color(fg_muted()))
                                .w_full()
                                .h(px(24.0))
                                .pl_2()
                                .pr(move || {
                                    if drawer_query.get().is_empty() {
                                        px(6.0)
                                    } else {
                                        px(20.0)
                                    }
                                })
                                .rounded(px(RADIUS_SM))
                                .bg(bg_code())
                                .border_1()
                                .border_color(border_control())
                                .focus(|style| style.border_color(accent_blue()))
                                .text_size(px(TEXT_SM))
                                .text_color(fg_primary())
                                .on_change(move |text: &str| drawer_query.set(text.to_string())),
                        )
                        .child(when(
                            move || !drawer_query.get().is_empty(),
                            move || {
                                view()
                                    .role(AccessibilityRole::Button)
                                    .absolute()
                                    .right(px(2.0))
                                    .top(px(3.0))
                                    .w(px(18.0))
                                    .h(px(18.0))
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
                                    .on_press(move || drawer_query.set(String::new()))
                                    .child(text("×").text_size(px(TEXT_SM)).text_color(fg_muted()))
                            },
                        )),
                )
                .child(
                    text(move || -> SharedString {
                        if facet_tab.get() == 0 {
                            tr("点击只看 · × 屏蔽", "Click to filter · × to mute")().into()
                        } else {
                            tr("点击按进程/来源筛选", "Click to filter by app/client")().into()
                        }
                    })
                    .flex_shrink_0()
                    .text_size(px(TEXT_XS))
                    .text_color(fg_muted()),
                )
                .child(
                    view()
                        .role(AccessibilityRole::Button)
                        .w(px(ControlSize::Xs.height()))
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
                        .on_press(move || facets_open.set(false))
                        .child(glyph_px(10.0, || Glyph::Close, || color(fg_muted()))),
                ),
        )
        .child(
            when(
                move || items_list().is_empty(),
                move || {
                    view().pr(px(LIST_PAD_END)).py_2().child(
                        text(tr(
                            "暂无捕获到的域名或应用",
                            "No captured domains or apps yet",
                        ))
                        .text_size(px(TEXT_SM))
                        .text_color(fg_muted()),
                    )
                },
            )
            .otherwise(move || {
                scroll()
                    .w_full()
                    .max_h(move || px((window().height.0 * 0.45).round().clamp(160.0, 320.0)))
                    .child(
                        view()
                            .w_full()
                            .pr(px(LIST_PAD_END))
                            .gap_0p5()
                            .child(when(
                                move || facet_tab.get() == 0,
                                move || {
                                    view().w_full().gap_0p5().child(each(
                                        grouped_domains,
                                        |group| group.root.clone(),
                                        move |group, _| {
                                            sidebar_domain_group_view(
                                                group,
                                                filter,
                                                drawer_query,
                                                collapsed_roots,
                                            )
                                        },
                                    ))
                                },
                            ))
                            .child(when(
                                move || facet_tab.get() != 0,
                                move || {
                                    view().w_full().gap_0p5().child(each(
                                        items_list,
                                        |entry| entry.token.clone(),
                                        move |entry, _| sidebar_app_row(entry, filter),
                                    ))
                                },
                            )),
                    )
            }),
        )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn drawer_quick_chip(
    filter: Signal<String>,
    zh: &'static str,
    en: &'static str,
    token: &'static str,
    aliases: &'static [&'static str],
    tip_zh: &'static str,
    tip_en: &'static str,
) -> impl IntoElement {
    let is_active = computed(move || filter_has_token(&filter.get(), aliases));
    view()
        .role(AccessibilityRole::Button)
        .h(px(ControlSize::Xs.height()))
        .px_1p5()
        .rounded(px(RADIUS_SM))
        .border_1()
        .border_color(move || {
            if is_active.get() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if is_active.get() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if is_active.get() {
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
        .label(tr(zh, en))
        .tooltip(label_tip(tip_zh, tip_en))
        .flex_row()
        .items_center()
        .gap_1()
        .on_press(move || filter.set(toggle_filter_chip(&filter.get(), token, aliases)))
        .child(view().w(px(5.0)).h(px(5.0)).rounded_full().bg(move || {
            if is_active.get() {
                accent_blue()
            } else {
                fg_muted()
            }
        }))
        .child(
            compact_control_text(tr(zh, en), TEXT_XS).text_color(move || {
                if is_active.get() {
                    accent_blue_text()
                } else {
                    fg_secondary()
                }
            }),
        )
}

pub(crate) const JSON_ALIASES: &[&str] = &["type:json", "ct:json"];
#[cfg(test)]
pub(crate) const FAIL_ALIASES: &[&str] = &["失败", "fail", "err", "error"];
pub(crate) const ERR_ALIASES: &[&str] = &[
    "status:4xx|5xx|err",
    "status:4xx,5xx,err",
    "4xx|5xx|err",
    "4xx,5xx,err",
];
pub(crate) const PIN_ALIASES: &[&str] = &["pin:"];
pub(crate) const SLOW_ALIASES: &[&str] = &["slow:"];

pub(crate) fn remember_filter(controls: &Controls, current: &str) {
    let current = current.trim().to_string();
    if saved_config().filter == current {
        return;
    }
    save_config_or_notify(controls, PrefName::Filter, |config| {
        config.filter = current.clone()
    });
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct WindowPlacementSnapshot {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) x: Option<i32>,
    pub(crate) y: Option<i32>,
    pub(crate) display_id: Option<String>,
    pub(crate) mode: String,
}

pub(crate) fn remember_window_placement(controls: &Controls, snap: &WindowPlacementSnapshot) {
    let width = config::clamp_window_width(snap.width);
    let height = config::clamp_window_height(snap.height);
    let mode = config::normalize_window_mode(&snap.mode);
    let display_id = snap
        .display_id
        .as_deref()
        .and_then(config::parse_window_display_id)
        .map(config::format_window_display_id);
    let (x, y) = match (snap.x, snap.y) {
        (Some(x), Some(y)) => (Some(x), Some(y)),
        _ => (None, None),
    };
    let current = saved_config();
    if current.window_width == width
        && current.window_height == height
        && current.window_x == x
        && current.window_y == y
        && current.window_display_id == display_id
        && current.window_mode == mode
    {
        return;
    }
    save_config_or_notify(controls, PrefName::Window, |config| {
        config.window_width = width;
        config.window_height = height;
        config.window_x = x;
        config.window_y = y;
        config.window_display_id = display_id.clone();
        config.window_mode = mode.clone();
    });
}
