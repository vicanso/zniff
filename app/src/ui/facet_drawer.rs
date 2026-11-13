#[allow(unused_imports)]
use super::*;

pub(crate) fn column_splitter(width: Signal<f32>, controls: Arc<Controls>) -> impl IntoElement {
    let origin = signal(0.0_f32);
    let width_grant = width;
    let width_move = width;
    let width_release = width;
    let width_cancel = width;
    let controls_release = controls.clone();
    let controls_cancel = controls;
    let hot = signal(false);
    let dragging = signal(false);
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
            origin.set(clamp_list_px_for_window(
                width_grant.get(),
                window().width.0,
            ));
        })
        .on_responder_move(move |event: &ResponderEvent| {
            width_move.set(clamp_list_px_for_window(
                origin.get() + event.gesture.delta.x.0,
                window().width.0,
            ));
        })
        .on_responder_release(move |_event: &ResponderEvent| {
            dragging.set(false);
            store_list_width(
                clamp_list_px_for_window(width_release.get(), window().width.0),
                &controls_release,
            );
        })
        .on_responder_terminate(move |_event: &ResponderEvent| {
            dragging.set(false);
            store_list_width(
                clamp_list_px_for_window(width_cancel.get(), window().width.0),
                &controls_cancel,
            );
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn flow_list(
    rows: Signal<Vec<Arc<RowLine>>>,
    facets: Signal<session::SessionFacets>,
    facets_open: Signal<bool>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
    compose: Compose,
    main_filter: Signal<String>,
    list_filter: Signal<String>,
    effective_filter: Signal<String>,
    rules_text: Signal<String>,
    width: Signal<f32>,
    columns: Signal<ListColumns>,
    sort: Signal<Option<ListSort>>,
    controls: Arc<Controls>,
    shown: Shown,
    baseline: Signal<Option<u64>>,
    filter_focus: ElementRef,
) -> impl IntoElement {
    let row_controls = controls.clone();
    let merge_controls = controls.clone();
    let column_controls = controls;
    let picked = signal(None::<u64>);
    let menu_open = signal(false);
    let menu_at = signal(Point::<Points>::default());
    let facet_tab = signal(0_u8);
    let effective_width = move || clamp_list_px_for_window(width.get(), window().width.0);
    view()
        .w(move || px(effective_width()))
        .flex_shrink_0()
        .overflow_hidden()
        .bg(bg_list())
        .child(
            view()
                .w(move || px(effective_width()))
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
                        .child(
                            input()
                                .element_ref(&filter_focus)
                                .value(list_filter)
                                .placeholder(tr(
                                    "在当前结果中再次过滤 (主机、路径、状态、body:…)",
                                    "Filter current results (host, path, status, body:…)",
                                ))
                                .placeholder_color(move || color(fg_muted()))
                                .w_full()
                                .h(px(30.0))
                                .pl_2p5()
                                .pr(move || {
                                    if list_filter.get().is_empty() {
                                        px(10.0)
                                    } else {
                                        px(28.0)
                                    }
                                })
                                .rounded(px(7.0))
                                .bg(bg_code())
                                .border_1()
                                .border_color(border_control())
                                .focus(|style| style.border_color(accent_blue()))
                                .text_size(px(12.5))
                                .text_color(fg_primary())
                                .on_change(move |text: &str| list_filter.set(text.to_string())),
                        )
                        .child(when(
                            move || !list_filter.get().is_empty(),
                            move || {
                                view()
                                    .absolute()
                                    .right(px(4.0))
                                    .top(px(4.0))
                                    .w(px(22.0))
                                    .h(px(22.0))
                                    .flex_shrink_0()
                                    .flex_row()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(5.0))
                                    .hover(|style| style.bg(bg_control_hover()))
                                    .active(|style| style.opacity(0.75))
                                    .cursor(Cursor::Pointer)
                                    .label(tr("清空二次过滤", "Clear secondary filter"))
                                    .tooltip(label_tip("清空二次过滤", "Clear secondary filter"))
                                    .on_press(move || list_filter.set(String::new()))
                                    .child(
                                        control_text("×")
                                            .text_size(px(14.0))
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
                            .h(px(30.0))
                            .px_2()
                            .flex_shrink_0()
                            .flex_row()
                            .items_center()
                            .justify_center()
                            .rounded(px(CONTROL_RADIUS))
                            .border_1()
                            .border_color(accent_blue_border())
                            .bg(accent_blue_soft())
                            .hover(|style| style.bg(accent_blue_soft_hover()))
                            .active(|style| style.opacity(0.8))
                            .cursor(Cursor::Pointer)
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
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(accent_blue_text()),
                            )
                    },
                ))
                .child(facet_toggle_button(facets_open, list_filter))
                .child(column_menu(columns, column_controls)),
        )
        .child(when(
            move || facets_open.get(),
            move || facet_drawer(facets, facet_tab, list_filter, effective_width),
        ))
        .child(view().flex_1().min_h(px(0.0)).child(list_menu(
            rows,
            selected,
            paused,
            compose,
            columns,
            sort,
            row_controls,
            list_filter,
            effective_filter,
            shown,
            baseline,
            rules_text,
            picked,
            menu_open,
            menu_at,
        )))
}

pub(crate) fn facet_toggle_button(
    facets_open: Signal<bool>,
    filter: Signal<String>,
) -> impl IntoElement {
    let has_facet_filter = computed(move || {
        let f = filter.get();
        !extract_host_or(&f).is_empty()
            || !extract_app_or(&f).is_empty()
            || filter_has_token(&f, SLOW_ALIASES)
            || filter_has_token(&f, ERR_ALIASES)
            || filter_has_token(&f, JSON_ALIASES)
            || filter_has_token(&f, PIN_ALIASES)
    });
    view()
        .w(px(30.0))
        .h(px(30.0))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(move || {
            if facets_open.get() || has_facet_filter.get() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if facets_open.get() || has_facet_filter.get() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if facets_open.get() || has_facet_filter.get() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .label(tr("域名与应用导航", "Domains & Apps"))
        .tooltip(label_tip("域名与应用导航", "Domains & Apps"))
        .on_press(move || facets_open.set(!facets_open.get()))
        .child(glyph(
            || Glyph::List,
            move || {
                if facets_open.get() || has_facet_filter.get() {
                    color(accent_blue_text())
                } else {
                    color(fg_secondary())
                }
            },
        ))
}

pub(crate) fn facet_drawer(
    facets: Signal<session::SessionFacets>,
    facet_tab: Signal<u8>,
    filter: Signal<String>,
    effective_width: impl Fn() -> f32 + Copy + 'static,
) -> impl IntoElement {
    let tab_btn = move |idx: u8,
                        zh: &'static str,
                        en: &'static str,
                        count_fn: fn(&session::SessionFacets) -> usize| {
        let langs = ui_langs();
        view()
            .h(px(22.0))
            .px_2()
            .rounded(px(5.0))
            .flex_row()
            .items_center()
            .gap_1()
            .bg(move || {
                if facet_tab.get() == idx {
                    accent_blue_soft()
                } else {
                    AdaptiveColor::TRANSPARENT
                }
            })
            .border_1()
            .border_color(move || {
                if facet_tab.get() == idx {
                    accent_blue_border()
                } else {
                    AdaptiveColor::TRANSPARENT
                }
            })
            .hover(|style| {
                style.bg(move || {
                    if facet_tab.get() == idx {
                        accent_blue_soft_hover()
                    } else {
                        bg_control_hover()
                    }
                })
            })
            .active(|style| style.opacity(0.8))
            .cursor(Cursor::Pointer)
            .on_press(move || facet_tab.set(idx))
            .child(
                control_text(move || -> SharedString {
                    let count = facets.with(count_fn);
                    format!("{} ({count})", t(ui_lang(langs), zh, en)).into()
                })
                .tabular_nums()
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if facet_tab.get() == idx {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
            )
    };
    let items_list = move || -> Vec<session::FacetEntry> {
        if facet_tab.get() == 0 {
            facets.with(|f| f.domains.clone())
        } else {
            facets.with(|f| f.apps.clone())
        }
    };
    view()
        .w(move || px(effective_width()))
        .flex_shrink_0()
        .bg(bg_chrome())
        .border_b_1()
        .border_b_color(border_strong())
        .pl(px(LIST_INSET))
        .py_1p5()
        .gap_1p5()
        .child(
            view()
                .pr(px(LIST_PAD_END))
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_1p5()
                .child(
                    view()
                        .min_w(px(0.0))
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_1()
                        .child(tab_btn(0, "域名", "Domains", |f| f.domains.len()))
                        .child(tab_btn(1, "应用 / 来源", "Apps / Clients", |f| {
                            f.apps.len()
                        }))
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
                )
                .child(
                    text(move || -> SharedString {
                        if facet_tab.get() == 0 {
                            tr("点击只看 · × 屏蔽", "Click to filter · × to mute")().into()
                        } else {
                            tr("点击按进程/来源筛选", "Click to filter by app/client")().into()
                        }
                    })
                    .text_size(px(11.0))
                    .text_color(fg_muted()),
                ),
        )
        .child(
            when(
                move || items_list().is_empty(),
                move || {
                    view().pr(px(LIST_PAD_END)).py_1p5().child(
                        text(tr(
                            "暂无捕获到的域名或应用",
                            "No captured domains or apps yet",
                        ))
                        .text_size(px(11.5))
                        .text_color(fg_muted()),
                    )
                },
            )
            .otherwise(move || {
                scroll()
                    .w_full()
                    .max_h(move || px((window().height.0 * 0.22).round().clamp(64.0, 132.0)))
                    .child(
                        view()
                            .w_full()
                            .pr(px(LIST_PAD_END))
                            .flex_row()
                            .flex_wrap()
                            .gap_1()
                            .child(each(
                                items_list,
                                move |entry| (facet_tab.get(), entry.token.clone()),
                                move |entry, _index| {
                                    let is_domain = move || facet_tab.get() == 0;
                                    let is_active = move || {
                                        let f = filter.get();
                                        let tok = entry.with(|e| e.token.clone());
                                        if is_domain() {
                                            host_facet_active(&f, &tok)
                                        } else {
                                            app_facet_active(&f, &tok)
                                        }
                                    };
                                    let is_muted = move || {
                                        is_domain()
                                            && mute_facet_active(
                                                &filter.get(),
                                                &entry.with(|e| e.token.clone()),
                                            )
                                    };
                                    view()
                                        .h(px(23.0))
                                        .max_w(move || {
                                            px((effective_width() - LIST_INSET - LIST_PAD_END)
                                                .max(96.0))
                                        })
                                        .rounded(px(5.0))
                                        .border_1()
                                        .border_color(move || {
                                            if is_active() {
                                                accent_blue_border()
                                            } else if is_muted() {
                                                status_err_fg()
                                            } else {
                                                border_control()
                                            }
                                        })
                                        .bg(move || {
                                            if is_active() {
                                                accent_blue_soft()
                                            } else if is_muted() {
                                                status_err_bg()
                                            } else {
                                                bg_control()
                                            }
                                        })
                                        .flex_row()
                                        .items_center()
                                        .overflow_hidden()
                                        .child(
                                            view()
                                                .min_w(px(0.0))
                                                .h_full()
                                                .px_2()
                                                .flex_row()
                                                .items_center()
                                                .gap_1()
                                                .hover(|style| {
                                                    style.bg(move || {
                                                        if is_active() {
                                                            accent_blue_soft_hover()
                                                        } else if is_muted() {
                                                            status_err_bg_hover()
                                                        } else {
                                                            bg_control_hover()
                                                        }
                                                    })
                                                })
                                                .active(|style| style.opacity(0.8))
                                                .cursor(Cursor::Pointer)
                                                .on_press(move || {
                                                    let cur = filter.get();
                                                    let tok = entry.with(|e| e.token.clone());
                                                    let next = if is_domain() {
                                                        toggle_host_facet(&cur, &tok)
                                                    } else {
                                                        toggle_app_facet(&cur, &tok)
                                                    };
                                                    filter.set(next);
                                                })
                                                .child(
                                                    mono(move || {
                                                        let label = entry.with(|e| e.label.clone());
                                                        if is_muted() {
                                                            format!("− {label}")
                                                        } else {
                                                            label
                                                        }
                                                    })
                                                    .min_w(px(0.0))
                                                    .number_of_lines(1)
                                                    .text_overflow(EllipsizeMode::Middle)
                                                    .text_size(px(11.0))
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
                                                    mono(move || {
                                                        entry.with(|e| format!("{}", e.count))
                                                    })
                                                    .flex_shrink_0()
                                                    .tabular_nums()
                                                    .text_size(px(11.0))
                                                    .text_color(move || {
                                                        if entry.with(|e| e.error_count > 0) {
                                                            status_err_fg()
                                                        } else {
                                                            fg_muted()
                                                        }
                                                    }),
                                                ),
                                        )
                                        .child(when(is_domain, move || {
                                            view()
                                                .w(px(24.0))
                                                .h_full()
                                                .flex_shrink_0()
                                                .flex_row()
                                                .items_center()
                                                .justify_center()
                                                .border_l_1()
                                                .border_l_color(border_subtle())
                                                .hover(|style| {
                                                    style.bg(move || {
                                                        if is_muted() {
                                                            status_err_bg_hover()
                                                        } else {
                                                            bg_control_hover()
                                                        }
                                                    })
                                                })
                                                .active(|style| style.opacity(0.8))
                                                .cursor(Cursor::Pointer)
                                                .label(tr("屏蔽域名", "Mute domain"))
                                                .tooltip(label_tip(
                                                    "屏蔽此域名 (mute:)",
                                                    "Mute this domain (mute:)",
                                                ))
                                                .on_press(move || {
                                                    let tok = entry.with(|e| e.token.clone());
                                                    filter.set(toggle_mute_facet(
                                                        &filter.get(),
                                                        &tok,
                                                    ));
                                                })
                                                .child(
                                                    control_text("×")
                                                        .text_size(px(11.5))
                                                        .text_color(move || {
                                                            if is_muted() {
                                                                status_err_fg()
                                                            } else {
                                                                fg_muted()
                                                            }
                                                        }),
                                                )
                                        }))
                                },
                            )),
                    )
            }),
        )
}

#[allow(clippy::too_many_arguments)]
fn drawer_quick_chip(
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
        .h(px(22.0))
        .px_2()
        .rounded(px(5.0))
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
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
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
            control_text(tr(zh, en))
                .text_size(px(11.0))
                .text_color(move || {
                    if is_active.get() {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

pub(crate) const JSON_ALIASES: &[&str] = &["type:json", "ct:json"];
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

pub(crate) fn remember_window_size(controls: &Controls, width: u32, height: u32) {
    let width = config::clamp_window_width(width);
    let height = config::clamp_window_height(height);
    let current = saved_config();
    if current.window_width == width && current.window_height == height {
        return;
    }
    save_config_or_notify(controls, PrefName::Window, |config| {
        config.window_width = width;
        config.window_height = height;
    });
}
