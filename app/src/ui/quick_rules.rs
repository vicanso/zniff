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

/// Compact pill-track toggle switch for rules panel headers and inline toggles.
pub(crate) fn mini_toggle_switch(
    on: Signal<bool>,
    zh: &'static str,
    en: &'static str,
    apply: impl Fn(bool) + 'static,
) -> impl IntoElement {
    switch()
        .h(px(24.0))
        .px_1p5()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_1p5()
        .rounded(px(5.0))
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.opacity(0.85))
        .cursor(Cursor::Pointer)
        .checked(move || on.get())
        .on_change(move |next: &bool| {
            on.set(*next);
            apply(*next);
        })
        .track_style(|s| {
            s.w(px(26.0))
                .h(px(14.0))
                .p(px(2.0))
                .flex_row()
                .items_center()
                .rounded(px(99.0))
                .bg(switch_track_off())
                .checked(|c| c.bg(accent_blue()))
        })
        .thumb_style(|s| {
            s.w(px(10.0))
                .h(px(10.0))
                .rounded_full()
                .bg(switch_thumb())
                .checked(|c| c.translate_x(px(12.0)))
        })
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || if on.get() { fg_primary() } else { fg_muted() }),
        )
}

pub(crate) fn rules_panel_editor_height(window_h: f32, quick_add_open: bool) -> f32 {
    if window_h.is_finite() && window_h > 0.0 {
        if quick_add_open {
            (window_h * 0.20).round().clamp(96.0, 176.0)
        } else {
            (window_h * 0.28).round().clamp(120.0, 240.0)
        }
    } else {
        140.0
    }
}

pub(crate) fn rules_editor(
    target: RuleEditTarget,
    controls: Arc<Controls>,
    quick_add_open: Signal<bool>,
) -> impl IntoElement {
    guard_culled_pointer_events(view())
        .w_full()
        .h(move || {
            px(rules_panel_editor_height(
                window().height.0,
                quick_add_open.get(),
            ))
        })
        .rounded(px(7.0))
        .border_1()
        .border_color(border_control())
        .bg(bg_code())
        .overflow_hidden()
        .child(
            editor()
                .value(move || SharedString::from(target.current_text()))
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
                    target.commit(&controls, text.to_string());
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

pub(crate) fn clear_quick_draft(draft: QuickRuleDraftSignals) {
    draft.host.set(String::new());
    draft.path.set(String::new());
    draft.op.set(String::new());
    draft.from.set(String::new());
    draft.to.set(String::new());
    draft.modes.reset();
}

pub(crate) fn quick_add_section(
    target: RuleEditTarget,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
    open: Signal<bool>,
) -> impl IntoElement {
    when(
        move || open.get(),
        move || {
            let row2_controls = controls.clone();
            view()
                .w_full()
                .p_2()
                .gap_1p5()
                .rounded(px(8.0))
                .border_1()
                .border_color(border_subtle())
                .bg(bg_canvas())
                .flex_col()
                .child(
                    view()
                        .h(px(20.0))
                        .px_0p5()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .cursor(Cursor::Pointer)
                        .hover(|style| style.opacity(0.85))
                        .on_press(move || {
                            clear_quick_draft(draft);
                            open.set(false);
                        })
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_1()
                                .child(glyph_px(9.5, || Glyph::Chevron, || color(fg_muted())))
                                .child(
                                    text(tr("按类型配置规则", "Configure Rule"))
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(fg_muted()),
                                ),
                        )
                        .child(
                            text(tr("关闭", "Close"))
                                .text_size(px(10.5))
                                .text_color(fg_muted()),
                        ),
                )
                .child(quick_add_row_one(draft))
                .child(quick_add_row_two(target, row2_controls, draft, open))
                .child(quick_add_example_row(draft))
        },
    )
}

fn quick_add_example_row(draft: QuickRuleDraftSignals) -> impl IntoElement {
    let kind = draft.kind;
    view()
        .w_full()
        .h(px(22.0))
        .px_0p5()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(
                    text(tr("示例", "Example"))
                        .flex_shrink_0()
                        .text_size(px(10.5))
                        .text_color(fg_muted()),
                )
                .child(
                    mono(move || SharedString::from(quick_rule_example(kind.get())))
                        .flex_1()
                        .min_w(px(0.0))
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .text_size(px(10.5))
                        .text_color(fg_secondary()),
                ),
        )
        .child(
            view()
                .h(px(20.0))
                .px_1p5()
                .flex_shrink_0()
                .rounded(px(4.0))
                .flex_row()
                .items_center()
                .bg(AdaptiveColor::TRANSPARENT)
                .hover(|s| s.bg(bg_control_hover()))
                .active(|s| s.opacity(0.8))
                .cursor(Cursor::Pointer)
                .tooltip(label_tip(
                    "将当前类型的常用示例填入上方输入框",
                    "Fill fields above with this rule example",
                ))
                .on_press(move || {
                    fill_draft_from_rule_line(
                        quick_rule_example(kind.get_untracked()),
                        draft.kind,
                        draft.host,
                        draft.path,
                        draft.op,
                        draft.from,
                        draft.to,
                        draft.modes,
                    );
                })
                .child(
                    text(tr("填入示例", "Use example"))
                        .number_of_lines(1)
                        .text_size(px(10.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(accent_blue_text()),
                ),
        )
}

fn quick_add_row_one(draft: QuickRuleDraftSignals) -> impl IntoElement {
    let kind = draft.kind;
    let op_open = signal(false);
    let show_op_input = move || {
        quick_kind(kind.get()).supports_graphql_op()
            && (op_open.get() || !draft.op.get().trim().is_empty())
    };
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(quick_kind_dropdown(kind))
        .child(view().flex_1().min_w(px(0.0)).child(side_input(
            draft.host,
            tr("域名 (*)", "Host (*)"),
            28.0,
            false,
            move |text| draft.host.set(text.to_string()),
        )))
        .child(when(
            move || kind.get() != 14,
            move || {
                view().flex_1().min_w(px(0.0)).child(side_input(
                    draft.path,
                    tr("Path (*)", "Path (*)"),
                    28.0,
                    false,
                    move |text| draft.path.set(text.to_string()),
                ))
            },
        ))
        .child(when(show_op_input, move || {
            view()
                .w(px(96.0))
                .flex_shrink(1.0)
                .min_w(px(0.0))
                .child(side_input(
                    draft.op,
                    tr("GraphQL op", "GraphQL op"),
                    28.0,
                    false,
                    move |text| draft.op.set(text.to_string()),
                ))
        }))
        .child(when(
            move || quick_kind(kind.get()).supports_graphql_op() && !show_op_input(),
            move || {
                view()
                    .h(px(26.0))
                    .px_1p5()
                    .flex_shrink_0()
                    .rounded(px(5.0))
                    .items_center()
                    .justify_center()
                    .bg(AdaptiveColor::TRANSPARENT)
                    .hover(|s| s.bg(bg_control_hover()))
                    .active(|s| s.opacity(0.8))
                    .cursor(Cursor::Pointer)
                    .tooltip(label_tip(
                        "附加 GraphQL 操作名条件 (op=...)",
                        "Filter by GraphQL operation name (op=...)",
                    ))
                    .on_press(move || op_open.set(true))
                    .child(
                        mono("GQL")
                            .text_size(px(10.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg_muted()),
                    )
            },
        ))
        .child(when(
            move || matches!(kind.get(), 0 | 2 | 6 | 10 | 11 | 13 | 15),
            move || quick_add_mode_chip(draft),
        ))
}

fn quick_add_mode_chip(draft: QuickRuleDraftSignals) -> impl IntoElement {
    let kind = draft.kind;
    let modes = draft.modes;
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
                    .child(quick_side_chip(modes.mitm_mtls, false, "MITM", "MITM"))
                    .child(quick_side_chip(modes.mitm_mtls, true, "mTLS", "mTLS"))
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
                    .child(quick_side_chip(modes.status_drop, true, "断开", "Drop"))
            },
        ))
        .child(when(
            move || !matches!(kind.get(), 2 | 10 | 13),
            move || {
                view()
                    .flex_row()
                    .items_center()
                    .child(quick_side_chip(modes.req_res, false, "请求", "Req"))
                    .child(quick_side_chip(modes.req_res, true, "响应", "Res"))
            },
        ))
}

fn quick_from_placeholder(kind: u8, lang: Lang) -> &'static str {
    match kind {
        1 => t(
            lang,
            "目标地址: https://other.test/v1",
            "Target URL: https://other.test/v1",
        ),
        2 => t(
            lang,
            "本地文件或目录路径 (或留空仅填状态码)",
            "Local file or directory (or empty for status)",
        ),
        3 => t(
            lang,
            "延迟时长: 2000ms 或 2s",
            "Delay duration: 2000ms or 2s",
        ),
        4 | 5 => t(
            lang,
            "Header 条件 (可选): X-Debug:1",
            "Header cond (optional): X-Debug:1",
        ),
        6 => t(lang, "Header 名称: X-Debug", "Header name: X-Debug"),
        7 => t(lang, "参数名: env", "Query param: env"),
        9 => t(
            lang,
            "下行速率或预设: 3g / 256kbps",
            "Down rate or preset: 3g / 256kbps",
        ),
        10 => t(
            lang,
            "证书链文件 (.crt / .pem)",
            "Certificate chain (.crt / .pem)",
        ),
        11 => t(
            lang,
            "JSON 路径: /data/enabled 或 items[0].price",
            "JSON path: /data/enabled or items[0].price",
        ),
        13 => t(
            lang,
            "状态码 (默认 403): 403 / 404 / 503",
            "Status (default 403): 403 / 404 / 503",
        ),
        14 => t(
            lang,
            "目标 IP 或主机: 127.0.0.1 / ::1",
            "Target IP or host: 127.0.0.1 / ::1",
        ),
        15 => t(
            lang,
            "本地回环 URL: http://127.0.0.1:9000/hook",
            "Loopback URL: http://127.0.0.1:9000/hook",
        ),
        _ => t(lang, "查找原文 (from)", "Find text (from)"),
    }
}

fn quick_to_placeholder(kind: u8, lang: Lang) -> &'static str {
    match kind {
        2 => t(lang, "状态码: 200 (可选)", "Status: 200 (optional)"),
        4 | 5 => t(
            lang,
            "Body 子串 (可选): error",
            "Body substr (optional): error",
        ),
        6 => t(lang, "Header 值 (留空删除)", "Value (empty to delete)"),
        7 => t(lang, "参数值 (留空删除)", "Value (empty to delete)"),
        9 => t(
            lang,
            "上行速率 (可选): 64kbps",
            "Up rate (optional): 64kbps",
        ),
        10 => t(
            lang,
            "私钥文件 (.key，合并 PEM 可留空)",
            "Private key (.key, optional if combined)",
        ),
        11 => t(
            lang,
            "JSON 值: true / 42 / \"text\" (留空删除)",
            "JSON value: true / 42 / \"text\" (empty to delete)",
        ),
        14 => t(lang, "端口 (可选): 8080", "Port (optional): 8080"),
        15 => t(
            lang,
            "模式/超时 (可选): preview / both / 3000ms",
            "Mode/timeout (optional): preview / both / 3000ms",
        ),
        _ => t(lang, "替换成 (to)", "Replace with (to)"),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn quick_file_picker_button(
    target_input: Signal<String>,
    label: impl Fn() -> String + 'static,
) -> impl IntoElement {
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
                if let DialogOutcome::Picked(paths) =
                    pick_paths_after_paint(open_files_dialog()).await
                    && let Some(path) = paths.first()
                {
                    target_input.set(path.display().to_string());
                }
            });
        })
        .child(
            text(label)
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg_secondary()),
        )
}

fn quick_add_row_two(
    target: RuleEditTarget,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
    open: Signal<bool>,
) -> impl IntoElement {
    let langs = ui_langs();
    let kind = draft.kind;
    let modes = draft.modes;
    let from = draft.from;
    let to = draft.to;
    let row = view().flex_row().items_center().gap_1().child(when(
        move || !matches!(kind.get(), 8 | 12) && !(kind.get() == 13 && modes.status_drop.get()),
        move || {
            view().flex_1().min_w(px(0.0)).child(side_input(
                from,
                move || quick_from_placeholder(kind.get(), ui_lang(langs)).to_string(),
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
                quick_file_picker_button(from, move || {
                    if kind.get() == 10 {
                        tr("选择证书", "Cert…")()
                    } else {
                        tr("选择文件", "Browse")()
                    }
                })
            },
        )
    );
    let row = row
        .child(when(
            move || matches!(kind.get(), 0 | 2 | 4 | 5 | 6 | 7 | 9 | 10 | 11 | 14 | 15),
            move || {
                view().flex_1().min_w(px(0.0)).child(side_input(
                    to,
                    move || quick_to_placeholder(kind.get(), ui_lang(langs)).to_string(),
                    28.0,
                    false,
                    move |text| to.set(text.to_string()),
                ))
            },
        ))
        .child(when(
            move || matches!(kind.get(), 0 | 6 | 7 | 11),
            move || quick_var_menu(to),
        ));
    let row = desktop_child!(
        row,
        when(
            move || kind.get() == 10,
            move || quick_file_picker_button(to, tr("选择私钥", "Key…")),
        )
    );
    row.child(quick_add_submit_button(target, controls, draft, open))
}

fn quick_add_ready(draft: QuickRuleDraftSignals) -> bool {
    let kind_now = draft.kind.get();
    let mode_now = draft.modes.mode_for(kind_now);
    let from_raw = draft.from.get();
    let to_raw = draft.to.get();
    let op_raw = draft.op.get();
    let (eff_from, eff_to) = effective_quick_inputs(kind_now, mode_now, &from_raw, &to_raw);
    let path_raw = draft.path.get();
    let eff_path = effective_quick_path(kind_now, &path_raw);
    let eff_op = effective_quick_op(kind_now, &op_raw);
    let combined = crate::rules::combine_rule_pattern(&draft.host.get(), eff_path);
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
}

fn quick_add_submit_button(
    target: RuleEditTarget,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
    open: Signal<bool>,
) -> impl IntoElement {
    let kind = draft.kind;
    let modes = draft.modes;
    view()
        .h(px(28.0))
        .px_3()
        .flex_grow(move || {
            if matches!(kind.get(), 8 | 12) || (kind.get() == 13 && modes.status_drop.get()) {
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
            if quick_add_ready(draft) {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if quick_add_ready(draft) {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if quick_add_ready(draft) {
                    accent_blue_soft_hover()
                } else {
                    bg_control()
                }
            })
        })
        .active(|style| {
            style.opacity(move || {
                if quick_add_ready(draft) {
                    0.8_f32
                } else {
                    1.0_f32
                }
            })
        })
        .disabled(move || !quick_add_ready(draft))
        .disabled_style(|style| style.cursor(Cursor::Default))
        .cursor(Cursor::Pointer)
        .on_press(move || {
            let kind_now = kind.get();
            let mode_now = modes.mode_for(kind_now);
            let from_raw = draft.from.get();
            let to_raw = draft.to.get();
            let op_raw = draft.op.get();
            let (eff_from, eff_to) = effective_quick_inputs(kind_now, mode_now, &from_raw, &to_raw);
            let path_raw = draft.path.get();
            let eff_path = effective_quick_path(kind_now, &path_raw);
            let eff_op = effective_quick_op(kind_now, &op_raw);
            let combined_pattern = crate::rules::combine_rule_pattern(&draft.host.get(), eff_path);
            let Some(next) = crate::rules::append_quick_rule_with_op(
                &target.current_text_untracked(),
                quick_kind(kind_now),
                &combined_pattern,
                mode_now,
                eff_from,
                eff_to,
                eff_op,
            ) else {
                return;
            };
            target.commit(&controls, next);
            clear_quick_draft(draft);
            open.set(false);
        })
        .child(
            text(tr("确认添加", "Add"))
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if quick_add_ready(draft) {
                        accent_blue_text()
                    } else {
                        fg_muted()
                    }
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
