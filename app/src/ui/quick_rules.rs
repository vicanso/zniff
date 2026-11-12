#[allow(unused_imports)]
use super::*;

pub(crate) fn config_set(
    bypass: Signal<String>,
    rules: Signal<String>,
    upstream: Signal<String>,
) -> bool {
    !bypass.get().trim().is_empty()
        || !rules.get().trim().is_empty()
        || !upstream.get().trim().is_empty()
}

pub(crate) fn capture_muted(rules_on: Signal<bool>, breaks_on: Signal<bool>) -> bool {
    !rules_on.get() || !breaks_on.get()
}

pub(crate) fn enable_switch(
    on: Signal<bool>,
    zh: &'static str,
    en: &'static str,
    apply: impl Fn(bool) + 'static,
) -> impl IntoElement {
    view()
        .h(px(28.0))
        .px_2()
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .justify_center()
        .gap_1p5()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(move || {
            if on.get() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if on.get() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if on.get() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .on_press(move || {
            let next = !on.get();
            on.set(next);
            apply(next);
        })
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .bg(move || if on.get() { accent_blue() } else { fg_muted() }),
        )
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(12.0))
                .text_color(move || {
                    if on.get() {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

pub(crate) fn rules_editor(
    value: Signal<String>,
    controls: Arc<Controls>,
    import_open: Signal<bool>,
) -> impl IntoElement {
    guard_culled_pointer_events(view())
        .w_full()
        .h(move || {
            px(adaptive_rules_editor_height_for(
                window().height.0,
                import_open.get(),
            ))
        })
        .rounded(px(7.0))
        .border_1()
        .border_color(border_control())
        .bg(bg_code())
        .overflow_hidden()
        .child(
            editor()
                .value(move || SharedString::from(value.get()))
                .language("plaintext")
                .line_numbers(true)
                .folding(false)
                .soft_wrap(true)
                .font_size(Points(12.0))
                .line_height(Points(18.0))
                .scroll_beyond_last_line(false)
                .placeholder(move || {
                    SharedString::from(tr(
                        "一行一条：break example.com/api",
                        "One rule per line: break example.com/api",
                    )())
                })
                .on_change(move |text: &SharedString| {
                    let text = text.to_string();
                    value.set(text.clone());
                    controls.hub.set_rules(&text);
                    if let Err(err) = save_config(|config| config.rules = text) {
                        set_status(
                            &controls,
                            StatusMsg::PrefUnsaved {
                                name: PrefName::Rules,
                                err: err.to_string(),
                            },
                        );
                    }
                })
                .size_full(),
        )
}

pub(crate) fn quick_side_chip(
    response: Signal<bool>,
    target_res: bool,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    view()
        .h(px(26.0))
        .px_2()
        .flex_row()
        .items_center()
        .justify_center()
        .border_r(move || if !target_res { px(1.0) } else { px(0.0) })
        .border_r_color(border_control())
        .bg(move || {
            if response.get() == target_res {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if response.get() == target_res {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .on_press(move || response.set(target_res))
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if response.get() == target_res {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

pub(crate) fn replace_row(
    rules_text: Signal<String>,
    controls: Arc<Controls>,
    host_pattern: Signal<String>,
    path_pattern: Signal<String>,
    op_pattern: Signal<String>,
    from: Signal<String>,
    to: Signal<String>,
    modes: QuickRuleModeSignals,
    kind: Signal<u8>,
) -> impl IntoElement {
    let ready_to_add = move || {
        let kind_now = kind.get();
        let mode_now = modes.mode_for(kind_now);
        let from_raw = from.get();
        let to_raw = to.get();
        let op_raw = op_pattern.get();
        let (eff_from, eff_to) = effective_quick_inputs(kind_now, mode_now, &from_raw, &to_raw);
        let eff_op = effective_quick_op(kind_now, &op_raw);
        let combined = crate::rules::combine_rule_pattern(&host_pattern.get(), &path_pattern.get());
        crate::rules::append_quick_rule_with_op(
            "",
            quick_kind(kind_now),
            &combined,
            mode_now,
            eff_from,
            eff_to,
            eff_op,
        )
        .is_some()
    };
    view()
        .gap_1()
        .w_full()
        .child(
            text(tr("快速添加规则", "Quick Add Rule"))
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_muted()),
        )
        .child(
            view()
                .w_full()
                .p_2()
                .gap_1p5()
                .rounded(px(8.0))
                .border_1()
                .border_color(border_subtle())
                .bg(bg_canvas())
                .child(
                    view()
                        .w_full()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_1()
                        .children((0_u8..14).map(move |idx| {
                            let (zh, en) = quick_kind(idx).label();
                            view()
                                .h(px(26.0))
                                .px_2()
                                .flex_shrink_0()
                                .flex_row()
                                .items_center()
                                .justify_center()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(move || {
                                    if kind.get() == idx {
                                        accent_blue_border()
                                    } else {
                                        border_control()
                                    }
                                })
                                .bg(move || {
                                    if kind.get() == idx {
                                        accent_blue_soft()
                                    } else {
                                        bg_control()
                                    }
                                })
                                .hover(|style| {
                                    style.bg(move || {
                                        if kind.get() == idx {
                                            accent_blue_soft_hover()
                                        } else {
                                            bg_control_hover()
                                        }
                                    })
                                })
                                .active(|style| style.opacity(0.8))
                                .cursor(Cursor::Pointer)
                                .on_press(move || {
                                    kind.set(idx);
                                })
                                .child(
                                    text(tr(zh, en))
                                        .number_of_lines(1)
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(move || {
                                            if kind.get() == idx {
                                                accent_blue_text()
                                            } else {
                                                fg_secondary()
                                            }
                                        }),
                                )
                        })),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(view().flex_1().min_w(px(0.0)).child(side_input(
                            host_pattern,
                            tr("域名 (*)", "Host (*)"),
                            28.0,
                            false,
                            move |text| host_pattern.set(text.to_string()),
                        )))
                        .child(view().flex_1().min_w(px(0.0)).child(side_input(
                            path_pattern,
                            tr("Path (*)", "Path (*)"),
                            28.0,
                            false,
                            move |text| path_pattern.set(text.to_string()),
                        )))
                        .child(when(
                            move || quick_kind(kind.get()).supports_graphql_op(),
                            move || {
                                view().w(px(94.0)).flex_shrink(1.0).min_w(px(0.0)).child(
                                    side_input(
                                        op_pattern,
                                        tr("GraphQL op", "GraphQL op"),
                                        28.0,
                                        false,
                                        move |text| op_pattern.set(text.to_string()),
                                    ),
                                )
                            },
                        ))
                        .child(when(
                            move || matches!(kind.get(), 0 | 2 | 6 | 10 | 11 | 13),
                            move || {
                                view()
                                    .h(px(28.0))
                                    .flex_shrink_0()
                                    .flex_row()
                                    .items_center()
                                    .rounded(px(6.0))
                                    .border_1()
                                    .border_color(border_control())
                                    .overflow_hidden()
                                    .child(when(
                                        move || kind.get() == 10,
                                        move || {
                                            view()
                                                .flex_row()
                                                .items_center()
                                                .child(quick_side_chip(
                                                    modes.mitm_mtls,
                                                    false,
                                                    "MITM",
                                                    "MITM",
                                                ))
                                                .child(quick_side_chip(
                                                    modes.mitm_mtls,
                                                    true,
                                                    "mTLS",
                                                    "mTLS",
                                                ))
                                        },
                                    ))
                                    .child(when(
                                        move || kind.get() == 2,
                                        move || {
                                            view()
                                                .flex_row()
                                                .items_center()
                                                .child(quick_side_chip(
                                                    modes.local_fallback,
                                                    false,
                                                    "本地",
                                                    "Local",
                                                ))
                                                .child(quick_side_chip(
                                                    modes.local_fallback,
                                                    true,
                                                    "回源",
                                                    "Fallback",
                                                ))
                                        },
                                    ))
                                    .child(when(
                                        move || kind.get() == 13,
                                        move || {
                                            view()
                                                .flex_row()
                                                .items_center()
                                                .child(quick_side_chip(
                                                    modes.status_drop,
                                                    false,
                                                    "状态码",
                                                    "Status",
                                                ))
                                                .child(quick_side_chip(
                                                    modes.status_drop,
                                                    true,
                                                    "断开",
                                                    "Drop",
                                                ))
                                        },
                                    ))
                                    .child(when(
                                        move || !matches!(kind.get(), 2 | 10 | 13),
                                        move || {
                                            view()
                                                .flex_row()
                                                .items_center()
                                                .child(quick_side_chip(
                                                    modes.req_res,
                                                    false,
                                                    "请求",
                                                    "Req",
                                                ))
                                                .child(quick_side_chip(
                                                    modes.req_res,
                                                    true,
                                                    "响应",
                                                    "Res",
                                                ))
                                        },
                                    ))
                            },
                        )),
                )
                .child({
                    let langs = ui_langs();
                    let row = view().flex_row().items_center().gap_1().child(when(
                        move || {
                            !matches!(kind.get(), 4 | 5 | 8 | 12)
                                && !(kind.get() == 13 && modes.status_drop.get())
                        },
                        move || {
                            let placeholder = move || {
                                let lang = ui_lang(langs);
                                match kind.get() {
                                    1 => t(
                                        lang,
                                        "目标地址: https://other.test/v1",
                                        "Target URL: https://other.test/v1",
                                    )
                                    .to_string(),
                                    2 => t(
                                        lang,
                                        "本地文件或目录路径 (或留空仅填状态码)",
                                        "Local file or directory (or empty for status)",
                                    )
                                    .to_string(),
                                    3 => t(
                                        lang,
                                        "延迟时长: 2000ms 或 2s",
                                        "Delay duration: 2000ms or 2s",
                                    )
                                    .to_string(),
                                    6 => t(lang, "Header 名称: X-Debug", "Header name: X-Debug")
                                        .to_string(),
                                    7 => t(lang, "参数名: env", "Query param: env").to_string(),
                                    9 => t(
                                        lang,
                                        "下行速率或预设: 3g / 256kbps",
                                        "Down rate or preset: 3g / 256kbps",
                                    )
                                    .to_string(),
                                    10 => t(
                                        lang,
                                        "证书链文件 (.crt / .pem)",
                                        "Certificate chain (.crt / .pem)",
                                    )
                                    .to_string(),
                                    11 => t(
                                        lang,
                                        "JSON 路径: /data/enabled 或 items[0].price",
                                        "JSON path: /data/enabled or items[0].price",
                                    )
                                    .to_string(),
                                    13 => t(
                                        lang,
                                        "状态码 (默认 403): 403 / 404 / 503",
                                        "Status (default 403): 403 / 404 / 503",
                                    )
                                    .to_string(),
                                    _ => t(lang, "查找原文 (from)", "Find text (from)").to_string(),
                                }
                            };
                            view().flex_1().min_w(px(0.0)).child(side_input(
                                from,
                                placeholder,
                                28.0,
                                false,
                                move |text| from.set(text.to_string()),
                            ))
                        },
                    ));
                    let row = desktop_child!(
                        row,
                        when(
                            move || kind.get() == 2 || kind.get() == 10,
                            move || {
                                view()
                                    .h(px(28.0))
                                    .px_2p5()
                                    .flex_shrink_0()
                                    .flex_row()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(6.0))
                                    .border_1()
                                    .border_color(border_control())
                                    .bg(bg_control())
                                    .hover(|style| style.bg(bg_control_hover()))
                                    .active(|style| style.opacity(0.8))
                                    .cursor(Cursor::Pointer)
                                    .on_press(move || {
                                        spawn(async move {
                                            if let Ok(Some(paths)) =
                                                pick_paths(open_dialog_with_default_dir()).await
                                                && let Some(path) = paths.first()
                                            {
                                                from.set(path.display().to_string());
                                            }
                                        });
                                    })
                                    .child(
                                        text(move || {
                                            if kind.get() == 10 {
                                                tr("选择证书", "Cert…")()
                                            } else {
                                                tr("选择文件", "Browse")()
                                            }
                                        })
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(fg_secondary()),
                                    )
                            },
                        )
                    );
                    let row = row.child(when(
                        move || matches!(kind.get(), 0 | 2 | 6 | 7 | 9 | 10 | 11),
                        move || {
                            let placeholder = move || {
                                let lang = ui_lang(langs);
                                match kind.get() {
                                    2 => t(lang, "状态码: 200 (可选)", "Status: 200 (optional)")
                                        .to_string(),
                                    6 => t(lang, "Header 值 (留空删除)", "Value (empty to delete)")
                                        .to_string(),
                                    7 => t(lang, "参数值 (留空删除)", "Value (empty to delete)")
                                        .to_string(),
                                    9 => t(
                                        lang,
                                        "上行速率 (可选): 64kbps",
                                        "Up rate (optional): 64kbps",
                                    )
                                    .to_string(),
                                    10 => t(
                                        lang,
                                        "私钥文件 (.key，合并 PEM 可留空)",
                                        "Private key (.key, optional if combined)",
                                    )
                                    .to_string(),
                                    11 => t(
                                        lang,
                                        "JSON 值: true / 42 / \"text\" (留空删除)",
                                        "JSON value: true / 42 / \"text\" (empty to delete)",
                                    )
                                    .to_string(),
                                    _ => t(lang, "替换成 (to)", "Replace with (to)").to_string(),
                                }
                            };
                            view().flex_1().min_w(px(0.0)).child(side_input(
                                to,
                                placeholder,
                                28.0,
                                false,
                                move |text| to.set(text.to_string()),
                            ))
                        },
                    ));
                    let row = desktop_child!(
                        row,
                        when(
                            move || kind.get() == 10,
                            move || {
                                view()
                                    .h(px(28.0))
                                    .px_2p5()
                                    .flex_shrink_0()
                                    .flex_row()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(6.0))
                                    .border_1()
                                    .border_color(border_control())
                                    .bg(bg_control())
                                    .hover(|style| style.bg(bg_control_hover()))
                                    .active(|style| style.opacity(0.8))
                                    .cursor(Cursor::Pointer)
                                    .on_press(move || {
                                        spawn(async move {
                                            if let Ok(Some(paths)) =
                                                pick_paths(open_dialog_with_default_dir()).await
                                                && let Some(path) = paths.first()
                                            {
                                                to.set(path.display().to_string());
                                            }
                                        });
                                    })
                                    .child(
                                        text(tr("选择私钥", "Key…"))
                                            .text_size(px(11.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(fg_secondary()),
                                    )
                            },
                        )
                    );
                    row.child(
                        view()
                            .h(px(28.0))
                            .px_3()
                            .flex_grow(move || {
                                if matches!(kind.get(), 4 | 5 | 8 | 12)
                                    || (kind.get() == 13 && modes.status_drop.get())
                                {
                                    1.0_f32
                                } else {
                                    0.0_f32
                                }
                            })
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(move || {
                                if ready_to_add() {
                                    accent_blue_border()
                                } else {
                                    border_control()
                                }
                            })
                            .bg(move || {
                                if ready_to_add() {
                                    accent_blue_soft()
                                } else {
                                    bg_control()
                                }
                            })
                            .hover(|style| {
                                style.bg(move || {
                                    if ready_to_add() {
                                        accent_blue_soft_hover()
                                    } else {
                                        bg_control()
                                    }
                                })
                            })
                            .active(|style| {
                                style.opacity(
                                    move || {
                                        if ready_to_add() { 0.8_f32 } else { 1.0_f32 }
                                    },
                                )
                            })
                            .disabled(move || !ready_to_add())
                            .disabled_style(|style| style.cursor(Cursor::Default))
                            .cursor(Cursor::Pointer)
                            .on_press(move || {
                                let kind_now = kind.get();
                                let mode_now = modes.mode_for(kind_now);
                                let from_raw = from.get();
                                let to_raw = to.get();
                                let op_raw = op_pattern.get();
                                let (eff_from, eff_to) =
                                    effective_quick_inputs(kind_now, mode_now, &from_raw, &to_raw);
                                let eff_op = effective_quick_op(kind_now, &op_raw);
                                let combined_pattern = crate::rules::combine_rule_pattern(
                                    &host_pattern.get(),
                                    &path_pattern.get(),
                                );
                                let Some(next) = crate::rules::append_quick_rule_with_op(
                                    &rules_text.get(),
                                    quick_kind(kind_now),
                                    &combined_pattern,
                                    mode_now,
                                    eff_from,
                                    eff_to,
                                    eff_op,
                                ) else {
                                    return;
                                };
                                rules_text.set(next.clone());
                                controls.hub.set_rules(&next);
                                if let Err(err) = save_config(|config| config.rules = next) {
                                    set_status(
                                        &controls,
                                        StatusMsg::PrefUnsaved {
                                            name: PrefName::Rules,
                                            err: err.to_string(),
                                        },
                                    );
                                }
                                host_pattern.set(String::new());
                                path_pattern.set(String::new());
                                op_pattern.set(String::new());
                                from.set(String::new());
                                to.set(String::new());
                            })
                            .child(
                                text(tr("+ 添加规则", "+ Add"))
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(move || {
                                        if ready_to_add() {
                                            accent_blue_text()
                                        } else {
                                            fg_muted()
                                        }
                                    }),
                            ),
                    )
                }),
        )
}

pub(crate) fn config_field(
    zh: &'static str,
    en: &'static str,
    field: impl IntoElement,
) -> impl IntoElement {
    view()
        .gap_1()
        .w_full()
        .child(
            text(tr(zh, en))
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_muted()),
        )
        .child(field)
}
