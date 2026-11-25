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
    ui_switch_row(
        tr(zh, en),
        move || on.get(),
        move |next| {
            on.set(next);
            apply(next);
        },
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
        .rounded(px(RADIUS_MD))
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(move || motion_ms(MOTION_INSTANT_MS))
        .ease_bezier(0.22, 1.0, 0.36, 1.0)
        .focus_visible(apply_focus_ring)
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
                .rounded_full()
                .bg(switch_track_off())
                .checked(|c| c.bg(accent_blue()))
                .transition_colors()
                .duration(move || motion_ms(MOTION_INSTANT_MS))
                .ease_bezier(0.22, 1.0, 0.36, 1.0)
        })
        .thumb_style(|s| {
            s.w(px(10.0))
                .h(px(10.0))
                .rounded_full()
                .bg(switch_thumb())
                .checked(|c| c.translate_x(px(12.0)))
                .transition_transform()
                .duration(move || motion_ms(MOTION_INSTANT_MS))
                .ease_bezier(0.22, 1.0, 0.36, 1.0)
        })
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(TEXT_XS))
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
        .rounded(px(RADIUS_MD))
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
                .font_size(move || scaled_px(12.0))
                .line_height(move || scaled_px(18.0))
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
    button()
        .selected(move || response.get() == target_res)
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
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .on_press(move |_: &()| response.set(target_res))
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(TEXT_XS))
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
    draft.editing_line.set(None);
    draft.host.set(String::new());
    draft.path.set(String::new());
    draft.op.set(String::new());
    draft.from.set(String::new());
    draft.to.set(String::new());
    draft.modes.reset();
}

/// Groups rule kinds that share identical `from`/`to` input semantics (`break`/`break-res`).
pub(crate) fn quick_action_input_group(kind: u8) -> u8 {
    match kind % 16 {
        4 | 5 => 4,
        other => other,
    }
}

/// Returns whether switching from `prev_kind` to `next_kind` keeps the same
/// semantics for the two action inputs (`from` and `to`).
pub(crate) fn quick_kind_shares_action_inputs(prev_kind: u8, next_kind: u8) -> bool {
    quick_action_input_group(prev_kind) == quick_action_input_group(next_kind)
}

/// Switches the quick-rule builder to `next_kind`, keeping match target fields
/// (`host`, `path`, `op`) while clearing incompatible action inputs (`from`, `to`)
/// and ending active field editing so native placeholders refresh cleanly.
pub(crate) fn switch_quick_kind(draft: QuickRuleDraftSignals, next_kind: u8) {
    let prev_kind = draft.kind.get_untracked();
    if !quick_kind_shares_action_inputs(prev_kind, next_kind) {
        dismiss_keyboard();
        draft.from.set(String::new());
        draft.to.set(String::new());
    }
    draft.kind.set(next_kind);
}

pub(crate) fn quick_draft_has_input(draft: QuickRuleDraftSignals) -> bool {
    draft.editing_line.get().is_some()
        || !draft.host.get().trim().is_empty()
        || !draft.path.get().trim().is_empty()
        || !draft.op.get().trim().is_empty()
        || !draft.from.get().trim().is_empty()
        || !draft.to.get().trim().is_empty()
}

pub(crate) fn quick_add_section(
    target: RuleEditTarget,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
    open: Signal<bool>,
) -> impl IntoElement {
    let langs = ui_langs();
    when(
        move || open.get(),
        move || {
            let row2_controls = controls.clone();
            let editing_line = draft.editing_line;
            view()
                .w_full()
                .p_2()
                .gap_1p5()
                .rounded(px(RADIUS_MD))
                .bg(bg_canvas())
                .flex_col()
                .child(
                    view()
                        .w_full()
                        .h(px(22.0))
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_1p5()
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .h_full()
                                .tooltip(label_tip(
                                    "折叠表单并保留当前草稿",
                                    "Collapse form while keeping current draft",
                                ))
                                .child(
                                    button()
                                        .w_full()
                                        .min_w(px(0.0))
                                        .h_full()
                                        .px_1()
                                        .rounded(px(RADIUS_SM))
                                        .flex_row()
                                        .items_center()
                                        .gap_1()
                                        .hover(|style| style.bg(bg_control_hover()))
                                        .active(|style| style.bg(bg_control_active()))
                                        .transition_colors()
                                        .duration(MOTION_INSTANT_MS)
                                        .ease_out()
                                        .focus_visible(apply_focus_ring)
                                        .on_press(move |_: &()| open.set(false))
                                        .child(glyph_px(
                                            9.5,
                                            || Glyph::Chevron,
                                            || color(fg_muted()),
                                        ))
                                        .child(
                                            text(tr("按类型配置规则", "Configure Rule"))
                                                .text_size(px(TEXT_XS))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(fg_muted()),
                                        )
                                        .child(when(
                                            move || editing_line.get().is_some(),
                                            move || {
                                                view()
                                                    .px_1p5()
                                                    .py_0p5()
                                                    .rounded(px(RADIUS_SM))
                                                    .bg(status_warn_bg())
                                                    .child(
                                                        text(move || -> SharedString {
                                                            let n = editing_line
                                                                .get()
                                                                .map(|i| i + 1)
                                                                .unwrap_or(0);
                                                            match ui_lang(langs) {
                                                                Lang::Zh => {
                                                                    format!("编辑第 {n} 行")
                                                                }
                                                                Lang::En => {
                                                                    format!("Editing line {n}")
                                                                }
                                                            }
                                                            .into()
                                                        })
                                                        .text_size(px(TEXT_XS))
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(status_warn_fg()),
                                                    )
                                            },
                                        )),
                                ),
                        )
                        .child(
                            view()
                                .flex_shrink_0()
                                .flex_row()
                                .items_center()
                                .gap_1()
                                .child(when(
                                    move || quick_draft_has_input(draft),
                                    move || {
                                        button()
                                            .h(px(22.0))
                                            .px_1p5()
                                            .rounded(px(RADIUS_SM))
                                            .hover(|style| style.bg(bg_control_hover()))
                                            .active(|style| style.bg(bg_control_active()))
                                            .focus_visible(apply_focus_ring)
                                            .items_center()
                                            .justify_center()
                                            .on_press(move |_: &()| clear_quick_draft(draft))
                                            .child(
                                                text(tr("清空", "Clear"))
                                                    .text_size(px(TEXT_XS))
                                                    .text_color(fg_muted()),
                                            )
                                    },
                                ))
                                .child(
                                    view()
                                        .tooltip(label_tip(
                                            "关闭并清空当前草稿",
                                            "Close form and discard draft",
                                        ))
                                        .child(
                                            button()
                                                .h(px(22.0))
                                                .px_1p5()
                                                .rounded(px(RADIUS_SM))
                                                .hover(|style| style.bg(bg_control_hover()))
                                                .active(|style| style.bg(bg_control_active()))
                                                .focus_visible(apply_focus_ring)
                                                .items_center()
                                                .justify_center()
                                                .on_press(move |_: &()| {
                                                    clear_quick_draft(draft);
                                                    open.set(false);
                                                })
                                                .child(
                                                    text(tr("关闭", "Close"))
                                                        .text_size(px(TEXT_XS))
                                                        .text_color(fg_muted()),
                                                ),
                                        ),
                                ),
                        ),
                )
                .child(quick_add_row_one(target, controls.clone(), draft, open))
                .child(quick_add_row_two(target, row2_controls, draft, open))
                .child(quick_add_validation_hint(draft))
                .child(quick_add_example_row(draft))
        },
    )
}

fn quick_add_example_row(draft: QuickRuleDraftSignals) -> impl IntoElement {
    let kind = draft.kind;
    view()
        .w_full()
        .h(px(ControlSize::Xs.height()))
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
                        .text_size(px(TEXT_XS))
                        .text_color(fg_muted()),
                )
                .child(
                    mono(move || SharedString::from(quick_rule_example(kind.get())))
                        .flex_1()
                        .min_w(px(0.0))
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .text_size(px(TEXT_XS))
                        .text_color(fg_secondary()),
                ),
        )
        .child(
            view()
                .flex_shrink_0()
                .tooltip(label_tip(
                    "将当前类型的常用示例填入上方输入框",
                    "Fill fields above with this rule example",
                ))
                .child(
                    button()
                        .h(px(20.0))
                        .px_1p5()
                        .rounded(px(RADIUS_SM))
                        .flex_row()
                        .items_center()
                        .bg(AdaptiveColor::TRANSPARENT)
                        .hover(|s| s.bg(bg_control_hover()))
                        .active(|s| s.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .on_press(move |_: &()| {
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
                                .text_size(px(TEXT_XS))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(accent_blue_text()),
                        ),
                ),
        )
}

fn quick_add_row_one(
    target: RuleEditTarget,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
    open: Signal<bool>,
) -> impl IntoElement {
    let kind = draft.kind;
    let op_open = signal(false);
    let submit_host = controls.clone();
    let submit_path = controls.clone();
    let submit_op = controls;
    let show_op_input = move || {
        quick_kind(kind.get()).supports_graphql_op()
            && (op_open.get() || !draft.op.get().trim().is_empty())
    };
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(quick_kind_dropdown(draft))
        .child(view().flex_1().min_w(px(0.0)).child(submit_input(
            draft.host,
            tr("域名（*）", "Host (*)"),
            28.0,
            move |text| draft.host.set(text.to_string()),
            move |_| submit_quick_add(target, &submit_host, draft, open),
        )))
        .child(when(
            move || !matches!(kind.get(), 10 | 14),
            move || {
                let sc = submit_path.clone();
                view().flex_1().min_w(px(0.0)).child(submit_input(
                    draft.path,
                    tr("路径（*）", "Path (*)"),
                    28.0,
                    move |text| draft.path.set(text.to_string()),
                    move |_| submit_quick_add(target, &sc, draft, open),
                ))
            },
        ))
        .child(when(show_op_input, move || {
            let sc = submit_op.clone();
            view()
                .w(px(96.0))
                .flex_shrink(1.0)
                .min_w(px(0.0))
                .child(submit_input(
                    draft.op,
                    tr("GraphQL 操作名", "GraphQL op"),
                    28.0,
                    move |text| draft.op.set(text.to_string()),
                    move |_| submit_quick_add(target, &sc, draft, open),
                ))
        }))
        .child(when(
            move || quick_kind(kind.get()).supports_graphql_op() && !show_op_input(),
            move || {
                view()
                    .flex_shrink_0()
                    .tooltip(label_tip(
                        "附加 GraphQL 操作名条件（op=…）",
                        "Filter by GraphQL operation name (op=…)",
                    ))
                    .child(
                        button()
                            .h(px(26.0))
                            .px_1p5()
                            .rounded(px(RADIUS_SM))
                            .items_center()
                            .justify_center()
                            .bg(AdaptiveColor::TRANSPARENT)
                            .hover(|s| s.bg(bg_control_hover()))
                            .active(|s| s.bg(bg_control_active()))
                            .transition_colors()
                            .duration(MOTION_INSTANT_MS)
                            .ease_out()
                            .focus_visible(apply_focus_ring)
                            .on_press(move |_: &()| op_open.set(true))
                            .child(
                                mono("GQL")
                                    .text_size(px(TEXT_XS))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(fg_muted()),
                            ),
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
        .rounded(px(RADIUS_MD))
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
            "本地文件或目录路径（或留空仅填状态码）",
            "Local file or directory (or empty for status)",
        ),
        3 => t(
            lang,
            "延迟时长: 2000ms 或 2s",
            "Delay duration: 2000ms or 2s",
        ),
        4 | 5 => t(
            lang,
            "请求头条件（可选）: X-Debug:1",
            "Header cond (optional): X-Debug:1",
        ),
        6 => t(lang, "请求头名: X-Debug", "Header name: X-Debug"),
        7 => t(lang, "参数名: env", "Query param: env"),
        9 => t(
            lang,
            "下行速率或预设: 3g / 256kbps",
            "Down rate or preset: 3g / 256kbps",
        ),
        10 => t(
            lang,
            "证书链文件（.crt / .pem）",
            "Certificate chain (.crt / .pem)",
        ),
        11 => t(
            lang,
            "JSON 路径: /data/enabled 或 items[0].price",
            "JSON path: /data/enabled or items[0].price",
        ),
        13 => t(
            lang,
            "状态码（默认 403）: 403 / 404 / 503",
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
        _ => t(lang, "查找原文（from）", "Find text (from)"),
    }
}

fn quick_to_placeholder(kind: u8, lang: Lang) -> &'static str {
    match kind {
        2 => t(lang, "状态码: 200（可选）", "Status: 200 (optional)"),
        4 | 5 => t(
            lang,
            "正文子串（可选）: error",
            "Body substr (optional): error",
        ),
        6 => t(lang, "请求头值（留空删除）", "Value (empty to delete)"),
        7 => t(lang, "参数值（留空删除）", "Value (empty to delete)"),
        9 => t(
            lang,
            "上行速率（可选）: 64kbps",
            "Up rate (optional): 64kbps",
        ),
        10 => t(
            lang,
            "私钥文件（.key，合并 PEM 可留空）",
            "Private key (.key, optional if combined)",
        ),
        11 => t(
            lang,
            "JSON 值: true / 42 / \"text\"（留空删除）",
            "JSON value: true / 42 / \"text\" (empty to delete)",
        ),
        14 => t(lang, "端口（可选）: 8080", "Port (optional): 8080"),
        15 => t(
            lang,
            "模式/超时（可选）: preview / both / 3000ms",
            "Mode/timeout (optional): preview / both / 3000ms",
        ),
        _ => t(lang, "替换成（to）", "Replace with (to)"),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn quick_file_picker_button(
    target_input: Signal<String>,
    label: impl Fn() -> String + 'static,
) -> impl IntoElement {
    button()
        .h(px(ControlSize::Sm.height()))
        .px_2p5()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border_control())
        .bg(bg_control())
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .on_press(move |_: &()| {
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
                .text_size(px(TEXT_XS))
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
    let submit_from = controls.clone();
    let submit_to = controls.clone();
    let row = view().flex_row().items_center().gap_1().child(each(
        move || {
            if !matches!(kind.get(), 8 | 12) && !(kind.get() == 13 && modes.status_drop.get()) {
                vec![quick_action_input_group(kind.get())]
            } else {
                Vec::new()
            }
        },
        |group| *group,
        move |_, _| {
            let sc = submit_from.clone();
            view().flex_1().min_w(px(0.0)).child(submit_input(
                from,
                move || quick_from_placeholder(kind.get(), ui_lang(langs)).to_string(),
                28.0,
                move |text| from.set(text.to_string()),
                move |_| submit_quick_add(target, &sc, draft, open),
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
        .child(each(
            move || {
                if matches!(kind.get(), 0 | 2 | 4 | 5 | 6 | 7 | 9 | 10 | 11 | 14 | 15) {
                    vec![quick_action_input_group(kind.get())]
                } else {
                    Vec::new()
                }
            },
            |group| *group,
            move |_, _| {
                let sc = submit_to.clone();
                view().flex_1().min_w(px(0.0)).child(submit_input(
                    to,
                    move || quick_to_placeholder(kind.get(), ui_lang(langs)).to_string(),
                    28.0,
                    move |text| to.set(text.to_string()),
                    move |_| submit_quick_add(target, &sc, draft, open),
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

pub(crate) fn quick_add_validation_error(
    draft: QuickRuleDraftSignals,
    lang: Lang,
) -> Option<&'static str> {
    let host_raw = draft.host.get();
    let host = host_raw.trim();
    if host.is_empty() {
        return Some(t(
            lang,
            "请输入域名模式，如 * 或 api.example.com",
            "Enter a host pattern (e.g. * or api.example.com)",
        ));
    }
    if host.contains(char::is_whitespace) {
        return Some(t(
            lang,
            "域名模式不能包含空格",
            "Host pattern cannot contain spaces",
        ));
    }
    let kind_now = draft.kind.get();
    let path_raw = draft.path.get();
    let eff_path = effective_quick_path(kind_now, &path_raw);
    if eff_path.trim().contains(char::is_whitespace) {
        return Some(t(
            lang,
            "路径前缀不能包含空格",
            "Path prefix cannot contain spaces",
        ));
    }
    let mode_now = draft.modes.mode_for(kind_now);
    let from_raw = draft.from.get();
    let to_raw = draft.to.get();
    let op_raw = draft.op.get();
    let (eff_from, eff_to) = effective_quick_inputs(kind_now, mode_now, &from_raw, &to_raw);
    let eff_op = effective_quick_op(kind_now, &op_raw);
    let combined = crate::rules::combine_rule_pattern(host, eff_path);
    if crate::rules::append_quick_rule_with_op(
        "",
        quick_kind(kind_now),
        &combined,
        mode_now,
        eff_from,
        eff_to,
        eff_op,
    )
    .is_some()
    {
        return None;
    }
    Some(match kind_now % 16 {
        1 => t(
            lang,
            "请输入以 http:// 或 https:// 开头的目标 URL",
            "Enter a target URL starting with http:// or https://",
        ),
        2 => t(
            lang,
            "请填写本地文件路径，或填写有效状态码（100-599）",
            "Provide a local file path or valid status code (100-599)",
        ),
        3 => t(
            lang,
            "请输入合法延迟时长（如 500ms 或 2s）",
            "Enter a valid delay duration (e.g. 500ms or 2s)",
        ),
        9 => t(
            lang,
            "请输入速率或预设（如 3g、slow-3g、256kbps）",
            "Enter a bandwidth rate or preset (e.g. 3g or 256kbps)",
        ),
        10 => t(
            lang,
            "请填写或选择证书文件（.crt / .pem）",
            "Select or enter a certificate file (.crt / .pem)",
        ),
        14 => t(
            lang,
            "请输入合法目标 IP/主机名，且域名不能含路径或端口",
            "Enter a valid target IP/host (source host without path/port)",
        ),
        15 => t(
            lang,
            "Hook 仅支持本机回环 URL（http://127.0.0.1 或 ::1）",
            "Hook requires a local loopback URL (127.0.0.1 or ::1)",
        ),
        _ => t(
            lang,
            "请补全必填字段或检查输入格式",
            "Complete required fields or check syntax",
        ),
    })
}

fn quick_add_ready(draft: QuickRuleDraftSignals) -> bool {
    quick_add_validation_error(draft, Lang::En).is_none()
}

fn quick_add_validation_hint(draft: QuickRuleDraftSignals) -> impl IntoElement {
    let langs = ui_langs();
    let show = move || {
        quick_draft_has_input(draft) && quick_add_validation_error(draft, Lang::En).is_some()
    };
    when(show, move || {
        text(move || -> SharedString {
            quick_add_validation_error(draft, ui_lang(langs))
                .unwrap_or("")
                .into()
        })
        .px_0p5()
        .number_of_lines(1)
        .text_overflow(EllipsizeMode::Tail)
        .text_size(px(TEXT_XS))
        .text_color(status_warn_fg())
    })
}

fn submit_quick_add(
    target: RuleEditTarget,
    controls: &Controls,
    draft: QuickRuleDraftSignals,
    open: Signal<bool>,
) {
    let kind_now = draft.kind.get_untracked();
    let mode_now = draft.modes.mode_for(kind_now);
    let from_raw = draft.from.get_untracked();
    let to_raw = draft.to.get_untracked();
    let op_raw = draft.op.get_untracked();
    let host_raw = draft.host.get_untracked();
    let path_raw = draft.path.get_untracked();
    let (eff_from, eff_to) = effective_quick_inputs(kind_now, mode_now, &from_raw, &to_raw);
    let eff_path = effective_quick_path(kind_now, &path_raw);
    let eff_op = effective_quick_op(kind_now, &op_raw);
    let combined_pattern = crate::rules::combine_rule_pattern(&host_raw, eff_path);
    let current = target.current_text_untracked();
    let next = if let Some(line_idx) = draft.editing_line.get_untracked() {
        let Some(single_line) = crate::rules::append_quick_rule_with_op(
            "",
            quick_kind(kind_now),
            &combined_pattern,
            mode_now,
            eff_from,
            eff_to,
            eff_op,
        ) else {
            return;
        };
        replace_rule_line(&current, line_idx, &single_line)
    } else {
        let Some(appended) = crate::rules::append_quick_rule_with_op(
            &current,
            quick_kind(kind_now),
            &combined_pattern,
            mode_now,
            eff_from,
            eff_to,
            eff_op,
        ) else {
            return;
        };
        appended
    };
    target.commit(controls, next);
    clear_quick_draft(draft);
    open.set(false);
}

fn quick_add_submit_button(
    target: RuleEditTarget,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
    open: Signal<bool>,
) -> impl IntoElement {
    let langs = ui_langs();
    let kind = draft.kind;
    let modes = draft.modes;
    let editing_line = draft.editing_line;
    view()
        .flex_grow(move || {
            if matches!(kind.get(), 8 | 12) || (kind.get() == 13 && modes.status_drop.get()) {
                1.0_f32
            } else {
                0.0_f32
            }
        })
        .flex_shrink_0()
        .tooltip(
            tooltip().child(ui_tooltip_surface(
                text(move || -> SharedString {
                    let lang = ui_lang(langs);
                    if let Some(reason) = quick_add_validation_error(draft, lang) {
                        reason.into()
                    } else if let Some(idx) = editing_line.get() {
                        let n = idx + 1;
                        match lang {
                            Lang::Zh => format!("原地替换第 {n} 行规则（Enter）").into(),
                            Lang::En => format!("Replace rule line {n} in place (Enter)").into(),
                        }
                    } else {
                        t(
                            lang,
                            "添加规则到当前编辑器（Enter）",
                            "Add rule to editor (Enter)",
                        )
                        .into()
                    }
                })
                .text_size(px(TEXT_SM))
                .text_color(fg_primary()),
            )),
        )
        .child(
            button()
                .w_full()
                .h(px(ControlSize::Sm.height()))
                .px_3()
                .items_center()
                .justify_center()
                .rounded(px(RADIUS_MD))
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
                    style.bg(move || {
                        if quick_add_ready(draft) {
                            bg_control_active()
                        } else {
                            bg_control()
                        }
                    })
                })
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .disabled(move || !quick_add_ready(draft))
                .disabled_style(|style| style.opacity(0.55))
                .on_press(move |_: &()| submit_quick_add(target, &controls, draft, open))
                .child(
                    text(move || -> SharedString {
                        let lang = ui_lang(langs);
                        if let Some(idx) = editing_line.get() {
                            let n = idx + 1;
                            match lang {
                                Lang::Zh => format!("更新第 {n} 行").into(),
                                Lang::En => format!("Update line {n}").into(),
                            }
                        } else {
                            t(lang, "确认添加", "Add").into()
                        }
                    })
                    .text_size(px(TEXT_SM))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || {
                        if quick_add_ready(draft) {
                            accent_blue_text()
                        } else {
                            fg_muted()
                        }
                    }),
                ),
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
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_muted()),
        )
        .child(field)
}
