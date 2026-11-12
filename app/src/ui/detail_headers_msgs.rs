#[allow(unused_imports)]
use super::*;

pub(crate) fn card_folded(folded: Signal<u8>, bit: u8) -> bool {
    folded.get() & bit != 0
}

pub(crate) fn toggle_card(folded: Signal<u8>, bit: u8) {
    folded.set(folded.get() ^ bit);
}

pub(crate) fn fold_mark(folded: Signal<u8>, bit: u8) -> impl IntoElement {
    glyph(
        move || {
            if card_folded(folded, bit) {
                Glyph::ChevronRight
            } else {
                Glyph::Chevron
            }
        },
        || color(fg_muted()),
    )
}

pub(crate) fn kv_key_width(pane_w: f32) -> f32 {
    if pane_w > 0.0 {
        (pane_w * 0.32).round().clamp(108.0, 176.0)
    } else {
        176.0
    }
}

pub(crate) fn kv_value_cell(line: ReadSignal<HeaderLine>) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(0.0))
        .gap_1()
        .child(
            mono(move || line.with(|line| line.value.clone()))
                .selectable(true)
                .text_size(px(12.0))
                .text_color(fg_primary()),
        )
        .child(when(
            move || line.with(|line| line.hint.is_some()),
            move || {
                view()
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .bg(bg_control())
                    .border_1()
                    .border_color(border_subtle())
                    .child(
                        mono(move || line.with(|line| line.hint.clone().unwrap_or_default()))
                            .selectable(true)
                            .text_size(px(11.0))
                            .text_color(fg_secondary()),
                    )
            },
        ))
}

pub(crate) fn header_section(
    zh: &'static str,
    en: &'static str,
    lines: Signal<Vec<HeaderLine>>,
    pane_width: Signal<f32>,
    folded: Signal<u8>,
    bit: u8,
) -> impl IntoElement {
    view()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .overflow_hidden()
        .child(
            view()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_1p5()
                .px_3p5()
                .py_2()
                .border_b(move || {
                    if card_folded(folded, bit) {
                        px(0.0)
                    } else {
                        px(1.0)
                    }
                })
                .border_b_color(border_subtle())
                .hover(|style| style.bg(bg_control_hover()))
                .active(|style| style.opacity(0.85))
                .cursor(Cursor::Pointer)
                .on_press(move || toggle_card(folded, bit))
                .child(fold_mark(folded, bit))
                .child(section_title(tr(zh, en)))
                .child(
                    view()
                        .px_1p5()
                        .py(px(1.0))
                        .rounded(px(4.0))
                        .bg(bg_control())
                        .child(
                            mono(move || format!("{}", lines.with(Vec::len)))
                                .tabular_nums()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(fg_muted()),
                        ),
                ),
        )
        .child(when(
            move || !card_folded(folded, bit),
            move || {
                when(
                    move || lines.with(Vec::is_empty),
                    || view().px_3p5().py_2p5().child(muted_line(tr("无", "None"))),
                )
                .otherwise(move || {
                    each(
                        lines,
                        |line| line.key,
                        move |line, index| {
                            view()
                                .flex_row()
                                .gap_3()
                                .px_3p5()
                                .py_2()
                                .bg(move || {
                                    if index.get() % 2 == 1 {
                                        bg_code()
                                    } else {
                                        bg_card()
                                    }
                                })
                                .border_t(move || if index.get() > 0 { px(1.0) } else { px(0.0) })
                                .border_t_color(border_subtle())
                                .on_press_up(dismiss_keyboard)
                                .child(
                                    mono(move || line.with(|line| line.name.clone()))
                                        .w(move || px(kv_key_width(pane_width.get())))
                                        .flex_shrink_0()
                                        .selectable(true)
                                        .number_of_lines(1)
                                        .text_overflow(EllipsizeMode::Tail)
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(fg_muted()),
                                )
                                .child(kv_value_cell(line))
                        },
                    )
                })
            },
        ))
}

pub(crate) fn filter_message_lines(
    lines: &[HeaderLine],
    query: &str,
    dir: u8,
    kind: u8,
) -> Vec<HeaderLine> {
    if query.trim().is_empty() && dir == 0 && kind == 0 {
        return lines.to_vec();
    }
    lines
        .iter()
        .filter(|line| present::message_matches_filter(&line.name, &line.value, query, dir, kind))
        .cloned()
        .collect()
}

pub(crate) fn msg_filter_chip(
    state: Signal<u8>,
    val: u8,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    let active = move || state.get() == val;
    view()
        .h(px(22.0))
        .px_2()
        .flex_row()
        .items_center()
        .bg(move || if active() { accent_blue() } else { bg_card() })
        .hover(|style| {
            style.bg(move || {
                if active() {
                    accent_blue_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .on_press(move || state.set(val))
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if active() {
                        AdaptiveColor::rgb(0xffffff, 0xffffff)
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

pub(crate) fn msg_search_box(
    search: Signal<String>,
    lines: Signal<Vec<HeaderLine>>,
    dir: Signal<u8>,
    kind: Signal<u8>,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(
            input()
                .value(search)
                .placeholder(tr("搜索…", "Search…"))
                .placeholder_color(move || color(fg_muted()))
                .w(px(120.0))
                .h(px(22.0))
                .px_2()
                .rounded(px(6.0))
                .bg(bg_code())
                .border_1()
                .border_color(move || {
                    if search.get().trim().is_empty() {
                        border_control()
                    } else {
                        accent_blue_border()
                    }
                })
                .focus(|style| style.border_color(accent_blue()))
                .text_size(px(11.5))
                .text_color(fg_primary())
                .on_change(move |text: &str| search.set(text.to_string())),
        )
        .child(when(
            move || !search.get().trim().is_empty(),
            move || {
                view()
                    .h(px(22.0))
                    .px_1p5()
                    .rounded(px(6.0))
                    .bg(accent_blue_soft())
                    .hover(|style| style.bg(accent_blue_soft_hover()))
                    .active(|style| style.opacity(0.8))
                    .border_1()
                    .border_color(accent_blue_border())
                    .flex_row()
                    .items_center()
                    .cursor(Cursor::Pointer)
                    .tooltip(label_tip("点击清空搜索", "Click to clear search"))
                    .on_press(move || search.set(String::new()))
                    .child(
                        mono(move || {
                            let matched = filter_message_lines(
                                &lines.get(),
                                &search.get(),
                                dir.get(),
                                kind.get(),
                            )
                            .len();
                            format!("{matched} ×")
                        })
                        .tabular_nums()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(accent_blue_text()),
                    )
            },
        ))
}

pub(crate) fn messages_section(
    shown: Shown,
    controls: Arc<Controls>,
    folded: Signal<u8>,
) -> impl IntoElement {
    let lines = shown.messages;
    let search = shown.msg_search;
    let dir = shown.msg_dir;
    let kind = shown.msg_kind;
    let pane_width = shown.pane_width;
    let copy_all_controls = controls.clone();
    let row_controls = controls;
    let has_ws_dirs =
        move || lines.with(|items| items.iter().any(|line| present::is_ws_message(&line.name)));
    view()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .overflow_hidden()
        .child(
            view()
                .flex_shrink_0()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .px_3p5()
                .py_2()
                .border_b(move || {
                    if card_folded(folded, FOLD_MESSAGES) {
                        px(0.0)
                    } else {
                        px(1.0)
                    }
                })
                .border_b_color(border_subtle())
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .px_1()
                        .py_0p5()
                        .rounded(px(5.0))
                        .hover(|style| style.bg(bg_control_hover()))
                        .active(|style| style.opacity(0.85))
                        .cursor(Cursor::Pointer)
                        .on_press(move || toggle_card(folded, FOLD_MESSAGES))
                        .child(fold_mark(folded, FOLD_MESSAGES))
                        .child(section_title(tr("消息", "Messages")))
                        .child(
                            view()
                                .px_1p5()
                                .py(px(1.0))
                                .rounded(px(4.0))
                                .bg(bg_control())
                                .child(
                                    mono(move || {
                                        let total = lines.with(Vec::len);
                                        if search.get().trim().is_empty()
                                            && dir.get() == 0
                                            && kind.get() == 0
                                        {
                                            format!("{total}")
                                        } else {
                                            let matched = filter_message_lines(
                                                &lines.get(),
                                                &search.get(),
                                                dir.get(),
                                                kind.get(),
                                            )
                                            .len();
                                            format!("{matched}/{total}")
                                        }
                                    })
                                    .tabular_nums()
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(fg_muted()),
                                ),
                        ),
                )
                .child(when(
                    move || !card_folded(folded, FOLD_MESSAGES),
                    move || {
                        let copy_all_controls = copy_all_controls.clone();
                        view()
                            .flex_row()
                            .flex_wrap()
                            .items_center()
                            .gap_1p5()
                            .child(when(has_ws_dirs, move || {
                                view()
                                    .flex_row()
                                    .h(px(22.0))
                                    .rounded(px(6.0))
                                    .border_1()
                                    .border_color(border_control())
                                    .overflow_hidden()
                                    .child(msg_filter_chip(dir, 0, "全部", "All"))
                                    .child(msg_filter_chip(dir, 1, "↑ 客户端", "↑ Client"))
                                    .child(msg_filter_chip(dir, 2, "↓ 源站", "↓ Origin"))
                            }))
                            .child(when(has_ws_dirs, move || {
                                view()
                                    .flex_row()
                                    .h(px(22.0))
                                    .rounded(px(6.0))
                                    .border_1()
                                    .border_color(border_control())
                                    .overflow_hidden()
                                    .child(msg_filter_chip(kind, 0, "全部", "All"))
                                    .child(msg_filter_chip(kind, 1, "文本", "Text"))
                                    .child(msg_filter_chip(kind, 2, "二进制", "Binary"))
                                    .child(msg_filter_chip(kind, 3, "控制", "Control"))
                            }))
                            .child(msg_search_box(search, lines, dir, kind))
                            .child(when(
                                move || {
                                    filter_message_lines(
                                        &lines.get(),
                                        &search.get(),
                                        dir.get(),
                                        kind.get(),
                                    )
                                    .iter()
                                    .any(|line| line.name != "…")
                                },
                                move || {
                                    let controls = copy_all_controls.clone();
                                    detail_button(tr("复制全部", "Copy all"), move || {
                                        let matched = filter_message_lines(
                                            &lines.get(),
                                            &search.get(),
                                            dir.get(),
                                            kind.get(),
                                        );
                                        let payload = matched
                                            .into_iter()
                                            .filter(|line| line.name != "…")
                                            .map(|line| format!("[{}]\n{}", line.name, line.value))
                                            .collect::<Vec<_>>()
                                            .join("\n\n");
                                        if !payload.is_empty() {
                                            set_clipboard(payload);
                                            set_status(
                                                &controls,
                                                StatusMsg::CopiedBody {
                                                    side: Side::Response,
                                                    hex: false,
                                                },
                                            );
                                        }
                                    })
                                },
                            ))
                    },
                )),
        )
        .child(when(
            move || !card_folded(folded, FOLD_MESSAGES),
            move || {
                let row_controls = row_controls.clone();
                let filtered = move || {
                    filter_message_lines(&lines.get(), &search.get(), dir.get(), kind.get())
                };
                when(
                    move || filtered().is_empty(),
                    || view().px_3p5().py_2p5().child(muted_line(tr("无", "None"))),
                )
                .otherwise(move || {
                    let row_controls = row_controls.clone();
                    each(
                        filtered,
                        |line| line.key,
                        move |line, index| {
                            let controls = row_controls.clone();
                            view()
                                .flex_row()
                                .items_start()
                                .gap_3()
                                .px_3p5()
                                .py_2()
                                .bg(move || {
                                    if index.get() % 2 == 1 {
                                        bg_code()
                                    } else {
                                        bg_card()
                                    }
                                })
                                .border_t(move || if index.get() > 0 { px(1.0) } else { px(0.0) })
                                .border_t_color(border_subtle())
                                .on_press_up(dismiss_keyboard)
                                .child(
                                    mono(move || line.with(|line| line.name.clone()))
                                        .w(move || px(kv_key_width(pane_width.get())))
                                        .flex_shrink_0()
                                        .selectable(true)
                                        .number_of_lines(1)
                                        .text_overflow(EllipsizeMode::Tail)
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(fg_muted()),
                                )
                                .child(
                                    mono(move || line.with(|line| line.value.clone()))
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .selectable(true)
                                        .text_size(px(12.0))
                                        .text_color(fg_primary()),
                                )
                                .child(when(
                                    move || line.with(|l| l.name != "…"),
                                    move || {
                                        let controls = controls.clone();
                                        view()
                                            .w(px(22.0))
                                            .h(px(22.0))
                                            .flex_shrink_0()
                                            .flex_row()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(5.0))
                                            .border_1()
                                            .border_color(border_control())
                                            .bg(bg_control())
                                            .hover(|style| style.bg(bg_control_hover()))
                                            .active(|style| style.opacity(0.75))
                                            .cursor(Cursor::Pointer)
                                            .label(tr("复制消息", "Copy message"))
                                            .tooltip(label_tip("复制此条消息", "Copy message"))
                                            .on_press(move || {
                                                let payload = line.with(|l| l.value.clone());
                                                set_clipboard(payload);
                                                set_status(
                                                    &controls,
                                                    StatusMsg::CopiedBody {
                                                        side: Side::Response,
                                                        hex: false,
                                                    },
                                                );
                                            })
                                            .child(
                                                canvas(move || {
                                                    Painter::new()
                                                        .line_width(1.3)
                                                        .stroke_color(color(fg_secondary()))
                                                        .stroke_svg(
                                                            "M4.5 4.5 H10.5 V11.0 H4.5 Z M2.5 8.5 H2.0 V2.0 H8.0 V3.0",
                                                        )
                                                })
                                                .w(px(12.5))
                                                .h(px(12.5)),
                                            )
                                    },
                                ))
                        },
                    )
                })
            },
        ))
}

pub(crate) fn diff_card(shown: Shown, folded: Signal<u8>) -> impl IntoElement {
    view()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .overflow_hidden()
        .child(
            view()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_1p5()
                .px_3p5()
                .py_2()
                .border_b(move || {
                    if card_folded(folded, FOLD_DIFF) {
                        px(0.0)
                    } else {
                        px(1.0)
                    }
                })
                .border_b_color(border_subtle())
                .hover(|style| style.bg(bg_control_hover()))
                .active(|style| style.opacity(0.85))
                .cursor(Cursor::Pointer)
                .on_press(move || toggle_card(folded, FOLD_DIFF))
                .child(fold_mark(folded, FOLD_DIFF))
                .child(section_title(tr("对比", "Diff"))),
        )
        .child(when(
            move || !card_folded(folded, FOLD_DIFF),
            move || {
                view()
                    .bg(bg_code())
                    .h(px(320.0))
                    .flex_col()
                    .overflow_hidden()
                    .child(diff_legend())
                    .child(
                        guard_culled_pointer_events(view())
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_hidden()
                            .child(
                                diff_view()
                                    .old_value(move || SharedString::from(shown.diff_old.get()))
                                    .new_value(move || SharedString::from(shown.diff_new.get()))
                                    .layout(DiffLayout::Unified)
                                    .language("plaintext")
                                    .soft_wrap(true)
                                    .font_size(Points(12.0))
                                    .line_height(Points(18.0))
                                    .size_full(),
                            ),
                    )
            },
        ))
}

pub(crate) fn diff_legend() -> impl IntoElement {
    view()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_3()
        .px_3()
        .py_1p5()
        .border_b_1()
        .border_b_color(border_subtle())
        .child(diff_key(baseline_mark(), tr("基准", "Baseline")))
        .child(diff_key(diff_current_mark(), tr("当前", "Current")))
}

pub(crate) fn diff_key(
    color: AdaptiveColor,
    label: impl Fn() -> String + 'static,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(
            view()
                .w(px(2.0))
                .h(px(12.0))
                .rounded(px(1.0))
                .flex_shrink_0()
                .bg(color),
        )
        .child(
            text(label)
                .number_of_lines(1)
                .text_size(px(11.0))
                .text_color(fg_muted()),
        )
}
