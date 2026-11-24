//! Footer Watch Alerts badge and popover panel (E6).

#[allow(unused_imports)]
use super::*;

#[derive(Clone)]
pub(crate) struct WatchPopoverProps {
    pub(crate) open: Signal<bool>,
    pub(crate) rules: Signal<Vec<config::WatchRule>>,
    pub(crate) eval: Signal<crate::watch::WatchEvaluation>,
    pub(crate) effective_filter: Signal<String>,
    pub(crate) list_filter: Signal<String>,
    pub(crate) selected: Signal<Option<u64>>,
    pub(crate) controls: Arc<Controls>,
}

fn persist_watch_rules(
    controls: &Controls,
    rules: Signal<Vec<config::WatchRule>>,
    next: Vec<config::WatchRule>,
) {
    let cleaned = config::sanitize_watch_rules(next);
    rules.set(cleaned.clone());
    if let Err(err) = save_config(|cfg| cfg.watch_rules = cleaned) {
        set_status(
            controls,
            StatusMsg::PrefUnsaved {
                name: PrefName::List,
                err: err.to_string(),
            },
        );
    }
}

fn add_watch_rule(
    controls: &Controls,
    rules: Signal<Vec<config::WatchRule>>,
    name: &str,
    filter: &str,
) -> bool {
    let filter = filter.trim();
    if filter.is_empty() {
        return false;
    }
    let mut next = rules.get_untracked();
    if let Some(existing) = next
        .iter_mut()
        .find(|r| r.filter.eq_ignore_ascii_case(filter))
    {
        if !name.trim().is_empty() {
            existing.name = name.trim().to_string();
        }
        existing.enabled = true;
    } else {
        next.insert(
            0,
            config::WatchRule {
                name: name.trim().to_string(),
                filter: filter.to_string(),
                enabled: true,
            },
        );
    }
    persist_watch_rules(controls, rules, next);
    true
}

pub(crate) fn watch_popover(props: WatchPopoverProps) -> impl IntoElement {
    let open = props.open;
    let eval = props.eval;
    popover()
        .open(move || Some(open.get()))
        .popup_style(ui_popup_style)
        .popup_starting_style(ui_popup_starting_style)
        .placement(Placement::Top)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(8.0))
        .on_open_change(move |event: &nsh_base::PopoverEvent| {
            if let nsh_base::PopoverEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .trigger(watch_trigger(open, eval))
        .child(watch_panel(props))
}

fn watch_trigger(
    open: Signal<bool>,
    eval: Signal<crate::watch::WatchEvaluation>,
) -> impl IntoElement {
    let langs = ui_langs();
    let has_hits = move || eval.with(|e| e.total_hits > 0);
    let has_active = move || eval.with(|e| e.active_rules > 0);
    view()
        .role(AccessibilityRole::Button)
        .h(px(ControlSize::Xs.height()))
        .px(px(ControlSize::Xs.padding_x()))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap(px(ControlSize::Xs.gap()))
        .rounded(px(RADIUS_SM))
        .border_1()
        .border_color(move || {
            if has_hits() {
                status_warn_fg()
            } else if open.get() || has_active() {
                accent_blue_border()
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .bg(move || {
            if has_hits() {
                status_warn_bg()
            } else if open.get() || has_active() {
                accent_blue_soft()
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .hover(|style| {
            style.bg(move || {
                if has_hits() {
                    status_warn_bg()
                } else if open.get() || has_active() {
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
        .label(tr("匹配提醒", "Watch Alerts"))
        .child(glyph_px(
            ControlSize::Xs.icon_size(),
            || Glyph::Alert,
            move || {
                if has_hits() {
                    color(status_warn_fg())
                } else if open.get() || has_active() {
                    color(accent_blue_text())
                } else {
                    color(fg_muted())
                }
            },
        ))
        .child(
            text(move || -> SharedString {
                let lang = ui_lang(langs);
                let (hits, active) = eval.with(|e| (e.total_hits, e.active_rules));
                if hits > 0 {
                    format!(
                        "{} · {}",
                        t(lang, "提醒", "Watch"),
                        present::format_thousands(hits as u64)
                    )
                    .into()
                } else if active > 0 {
                    format!(
                        "{} ({})",
                        t(lang, "提醒", "Watch"),
                        present::format_thousands(active as u64)
                    )
                    .into()
                } else {
                    t(lang, "提醒", "Watch").into()
                }
            })
            .tabular_nums()
            .text_size(px(TEXT_XS))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || {
                if has_hits() {
                    status_warn_fg()
                } else if open.get() || has_active() {
                    accent_blue_text()
                } else {
                    fg_secondary()
                }
            }),
        )
}

fn watch_panel(props: WatchPopoverProps) -> impl IntoElement {
    let draft_filter = signal(String::new());
    let draft_name = signal(String::new());
    let rules = props.rules;
    let eval = props.eval;
    let effective_filter = props.effective_filter;
    let list_filter = props.list_filter;
    let selected = props.selected;
    let open = props.open;
    let controls_add = props.controls.clone();
    let controls_enter = props.controls.clone();
    let controls_save_cur = props.controls.clone();
    let controls_list = props.controls;

    view()
        .w(move || px((window().width.0 - 24.0).clamp(280.0, 380.0)))
        .p_3()
        .gap_2p5()
        .child(watch_panel_header(
            effective_filter,
            rules,
            controls_save_cur,
        ))
        .child(watch_add_form(
            draft_name,
            draft_filter,
            rules,
            controls_add,
            controls_enter,
        ))
        .child(scroll().w_full().max_h(px(250.0)).child(watch_rules_list(
            rules,
            eval,
            list_filter,
            selected,
            open,
            controls_list,
        )))
}

fn watch_panel_header(
    effective_filter: Signal<String>,
    rules: Signal<Vec<config::WatchRule>>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(ui_section_header(tr("匹配提醒 (Watch)", "Watch Alerts")))
        .child(when(
            move || !effective_filter.get().trim().is_empty(),
            move || {
                let controls = controls.clone();
                ui_button(ButtonVariant::Outline, ControlSize::Xs)
                    .border_color(accent_blue_border())
                    .bg(accent_blue_soft())
                    .hover(|s| s.bg(accent_blue_soft_hover()))
                    .active(|s| s.bg(bg_control_active()))
                    .on_press(move || {
                        let cur = effective_filter.get_untracked();
                        add_watch_rule(&controls, rules, "", &cur);
                    })
                    .child(
                        control_text(tr("监听当前过滤", "Watch current filter"))
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(accent_blue_text()),
                    )
            },
        ))
}

fn watch_add_form(
    draft_name: Signal<String>,
    draft_filter: Signal<String>,
    rules: Signal<Vec<config::WatchRule>>,
    controls_add: Arc<Controls>,
    controls_enter: Arc<Controls>,
) -> impl IntoElement {
    view()
        .gap_1p5()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(
                    input()
                        .flex_1()
                        .h(px(ControlSize::Sm.height()))
                        .px_2()
                        .rounded(px(RADIUS_MD))
                        .border_1()
                        .border_color(border_control())
                        .bg(bg_canvas())
                        .text_size(px(TEXT_SM))
                        .text_color(fg_primary())
                        .placeholder(tr(
                            "过滤表达式（如 status:5xx host:api.x.com）",
                            "Filter expr (e.g. status:5xx host:api.x.com)",
                        ))
                        .value(move || draft_filter.get())
                        .on_change(move |v: &str| draft_filter.set(v.to_string()))
                        .on_submit(move |_: &str| {
                            if add_watch_rule(
                                &controls_enter,
                                rules,
                                &draft_name.get_untracked(),
                                &draft_filter.get_untracked(),
                            ) {
                                draft_name.set(String::new());
                                draft_filter.set(String::new());
                            }
                        }),
                )
                .child(
                    ui_button(ButtonVariant::Primary, ControlSize::Sm)
                        .on_press(move || {
                            if add_watch_rule(
                                &controls_add,
                                rules,
                                &draft_name.get_untracked(),
                                &draft_filter.get_untracked(),
                            ) {
                                draft_name.set(String::new());
                                draft_filter.set(String::new());
                            }
                        })
                        .child(
                            control_text(tr("添加", "Add"))
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg_on_accent()),
                        ),
                ),
        )
        .child(
            input()
                .w_full()
                .h(px(ControlSize::Sm.height()))
                .px_2()
                .rounded(px(RADIUS_MD))
                .border_1()
                .border_color(border_control())
                .bg(bg_canvas())
                .text_size(px(TEXT_SM))
                .text_color(fg_secondary())
                .placeholder(tr(
                    "可选备注名（如：支付接口 5xx）",
                    "Optional label (e.g. Checkout 5xx)",
                ))
                .value(move || draft_name.get())
                .on_change(move |v: &str| draft_name.set(v.to_string())),
        )
}

fn watch_rules_list(
    rules: Signal<Vec<config::WatchRule>>,
    eval: Signal<crate::watch::WatchEvaluation>,
    list_filter: Signal<String>,
    selected: Signal<Option<u64>>,
    open: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    when(
        move || rules.with(Vec::is_empty),
        move || {
            view()
                .py_4()
                .px_2()
                .items_center()
                .justify_center()
                .child(
                    text(tr(
                        "保存过滤条件后，新流量命中时将自动弹窗提醒并在此计数",
                        "Save a filter expression to count hits and alert when new matching flows arrive",
                    ))
                    .text_center()
                    .text_size(px(TEXT_SM))
                    .text_color(fg_muted()),
                )
        },
    )
    .otherwise(move || {
        let controls = controls.clone();
        view().w_full().gap_1p5().child(each(
            move || {
                rules
                    .get()
                    .into_iter()
                    .enumerate()
                    .collect::<Vec<_>>()
            },
            |(idx, rule)| format!("{idx}:{}", rule.filter),
            move |item, _| {
                watch_rule_row(item, rules, eval, list_filter, selected, open, controls.clone())
            },
        ))
    })
}

fn watch_rule_row(
    item: ReadSignal<(usize, config::WatchRule)>,
    rules: Signal<Vec<config::WatchRule>>,
    eval: Signal<crate::watch::WatchEvaluation>,
    list_filter: Signal<String>,
    selected: Signal<Option<u64>>,
    open: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let controls_toggle = controls.clone();
    let controls_delete = controls;
    let enabled = move || item.with(|(_, r)| r.enabled);
    let filter_str = move || item.with(|(_, r)| r.filter.clone());
    let hit_info = move || {
        let f = item.with(|(_, r)| r.filter.clone());
        eval.with(|e| {
            e.rules
                .iter()
                .find(|s| s.filter.eq_ignore_ascii_case(&f))
                .map(|s| (s.hit_count, s.recent_ids.first().copied()))
                .unwrap_or((0, None))
        })
    };

    view()
        .w_full()
        .px_2()
        .py_1p5()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border_control())
        .bg(bg_canvas())
        .flex_row()
        .items_center()
        .gap_2()
        .child(
            view()
                .role(AccessibilityRole::Checkbox)
                .w(px(16.0))
                .h(px(16.0))
                .rounded(px(RADIUS_SM))
                .border_1()
                .border_color(move || {
                    if enabled() {
                        accent_blue()
                    } else {
                        border_strong()
                    }
                })
                .bg(move || {
                    if enabled() {
                        accent_blue()
                    } else {
                        bg_control()
                    }
                })
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .on_press(move || {
                    let target_filter = item.with_untracked(|(_, r)| r.filter.clone());
                    let mut next = rules.get_untracked();
                    if let Some(r) = next
                        .iter_mut()
                        .find(|r| r.filter.eq_ignore_ascii_case(&target_filter))
                    {
                        r.enabled = !r.enabled;
                    }
                    persist_watch_rules(&controls_toggle, rules, next);
                })
                .child(when(enabled, || {
                    glyph_px(10.0, || Glyph::Check, || color(fg_on_accent()))
                })),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(1.0))
                .on_press(move || {
                    let f = item.with_untracked(|(_, r)| r.filter.clone());
                    let (_, latest_id) = hit_info();
                    list_filter.set(f);
                    if let Some(id) = latest_id {
                        selected.set(Some(id));
                    }
                    open.set(false);
                })
                .child(
                    text(move || -> SharedString {
                        item.with(|(_, r)| {
                            if r.name.is_empty() {
                                r.filter.clone()
                            } else {
                                r.name.clone()
                            }
                        })
                        .into()
                    })
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail)
                    .text_size(px(TEXT_SM))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || if enabled() { fg_primary() } else { fg_muted() }),
                )
                .child(when(
                    move || item.with(|(_, r)| !r.name.is_empty()),
                    move || {
                        mono(move || -> SharedString { filter_str().into() })
                            .number_of_lines(1)
                            .text_overflow(EllipsizeMode::Tail)
                            .text_size(px(TEXT_XS))
                            .text_color(fg_muted())
                    },
                )),
        )
        .child(
            view()
                .px_1p5()
                .py_0p5()
                .rounded(px(RADIUS_SM))
                .flex_shrink_0()
                .bg(move || {
                    if hit_info().0 > 0 {
                        status_warn_bg()
                    } else {
                        bg_control()
                    }
                })
                .child(
                    mono(move || -> SharedString {
                        present::format_thousands(hit_info().0 as u64).into()
                    })
                    .tabular_nums()
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || {
                        if hit_info().0 > 0 {
                            status_warn_fg()
                        } else {
                            fg_muted()
                        }
                    }),
                ),
        )
        .child(
            ui_icon_button(ButtonVariant::Ghost, ControlSize::Xs)
                .accessibility_label(move || {
                    Some(SharedString::from(tr("删除提醒", "Delete watch")()))
                })
                .on_press(move || {
                    let target_filter = item.with_untracked(|(_, r)| r.filter.clone());
                    let mut next = rules.get_untracked();
                    next.retain(|r| !r.filter.eq_ignore_ascii_case(&target_filter));
                    persist_watch_rules(&controls_delete, rules, next);
                })
                .child(glyph_px(
                    ControlSize::Xs.icon_size(),
                    || Glyph::Close,
                    || color(fg_muted()),
                )),
        )
}
