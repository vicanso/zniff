#[allow(unused_imports)]
use super::*;

pub(crate) fn card_folded(folded: Signal<u8>, bit: u8) -> bool {
    folded.get() & bit != 0
}

pub(crate) fn toggle_card(folded: Signal<u8>, bit: u8) {
    folded.set(folded.get() ^ bit);
}

pub(crate) const PERSISTED_FOLD_MASK: u8 = FOLD_REQ_HEADERS
    | FOLD_RES_HEADERS
    | FOLD_QUERY
    | FOLD_REQ_BODY
    | FOLD_RES_BODY
    | FOLD_MESSAGES;

pub(crate) fn folded_bits_from_config(cards: &[String]) -> u8 {
    let mut bits = 0_u8;
    for card in cards {
        match card.trim().to_ascii_lowercase().as_str() {
            "req_headers" => bits |= FOLD_REQ_HEADERS,
            "res_headers" => bits |= FOLD_RES_HEADERS,
            "query" => bits |= FOLD_QUERY,
            "req_body" => bits |= FOLD_REQ_BODY,
            "res_body" => bits |= FOLD_RES_BODY,
            "messages" => bits |= FOLD_MESSAGES,
            _ => {}
        }
    }
    bits
}

pub(crate) fn folded_cards_from_bits(bits: u8) -> Vec<String> {
    let mut out = Vec::new();
    if bits & FOLD_REQ_HEADERS != 0 {
        out.push("req_headers".to_string());
    }
    if bits & FOLD_RES_HEADERS != 0 {
        out.push("res_headers".to_string());
    }
    if bits & FOLD_QUERY != 0 {
        out.push("query".to_string());
    }
    if bits & FOLD_REQ_BODY != 0 {
        out.push("req_body".to_string());
    }
    if bits & FOLD_RES_BODY != 0 {
        out.push("res_body".to_string());
    }
    if bits & FOLD_MESSAGES != 0 {
        out.push("messages".to_string());
    }
    out
}

pub(crate) fn toggle_card_bits(current: u8, bit: u8) -> (u8, Option<Vec<String>>) {
    let next = current ^ bit;
    let persisted = (bit & PERSISTED_FOLD_MASK != 0).then(|| folded_cards_from_bits(next));
    (next, persisted)
}

pub(crate) fn toggle_card_and_persist(folded: Signal<u8>, bit: u8, controls: &Controls) {
    let (next, persisted) = toggle_card_bits(folded.get(), bit);
    folded.set(next);
    if let Some(cards) = persisted {
        save_config_or_notify(controls, PrefName::Pretty, move |cfg| {
            cfg.folded_cards = cards;
        });
    }
}

pub(crate) fn fold_mark(folded: Signal<u8>, bit: u8) -> impl IntoElement {
    ui_rotating_chevron(
        13.0,
        move || !card_folded(folded, bit),
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
                .text_size(move || scaled_px(TEXT_SM))
                .text_color(fg_primary()),
        )
        .child(when(
            move || line.with(|line| line.hint.is_some()),
            move || {
                view()
                    .px_2()
                    .py_1()
                    .rounded(px(RADIUS_SM))
                    .bg(bg_control())
                    .border_1()
                    .border_color(border_subtle())
                    .child(
                        mono(move || line.with(|line| line.hint.clone().unwrap_or_default()))
                            .selectable(true)
                            .text_size(move || scaled_px(TEXT_XS))
                            .text_color(fg_secondary()),
                    )
            },
        ))
}

pub(crate) fn detail_loading_row() -> impl IntoElement {
    view()
        .px_3p5()
        .py_2p5()
        .flex_row()
        .items_center()
        .gap_2()
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded(px(RADIUS_SM))
                .flex_shrink_0()
                .bg(accent_blue())
                .style_with(|s| {
                    if reduce_motion() {
                        s
                    } else {
                        s.animation(ui_pulse_animation(1200.0))
                    }
                }),
        )
        .child(muted_line(tr("加载中…", "Loading…")))
}

pub(crate) fn detail_loading_box() -> impl IntoElement {
    let skeleton_bar = |width: f32| {
        view()
            .w(relative(width))
            .h(px(8.0))
            .rounded(px(RADIUS_SM))
            .bg(bg_control())
            .style_with(|s| {
                if reduce_motion() {
                    s
                } else {
                    s.animation(ui_pulse_animation(1500.0))
                }
            })
    };
    view()
        .w_full()
        .bg(bg_code())
        .px_3p5()
        .py_3()
        .gap_2p5()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2p5()
                .child(
                    progress()
                        .indeterminate(true)
                        .accessibility_label(move || SharedString::from(tr("加载中", "Loading")()))
                        .w(px(120.0))
                        .track_style(|s| {
                            s.w_full()
                                .h(px(4.0))
                                .rounded(px(2.0))
                                .bg(bg_control())
                                .overflow_hidden()
                        })
                        .indicator_style(|s| {
                            let s = s
                                .w(relative(0.42))
                                .h_full()
                                .rounded(px(2.0))
                                .bg(accent_blue());
                            if reduce_motion() {
                                s
                            } else {
                                s.animation(ui_pulse_animation(1100.0))
                            }
                        }),
                )
                .child(muted_line(tr("加载中…", "Loading…"))),
        )
        .child(
            view()
                .gap_1p5()
                .child(skeleton_bar(0.68))
                .child(skeleton_bar(0.42)),
        )
}

pub(crate) struct HeaderSectionProps {
    pub(crate) zh: &'static str,
    pub(crate) en: &'static str,
    pub(crate) lines: Signal<Vec<HeaderLine>>,
    pub(crate) loading: Signal<bool>,
    pub(crate) pane_width: Signal<f32>,
    pub(crate) folded: Signal<u8>,
    pub(crate) bit: u8,
    pub(crate) controls: Arc<Controls>,
}

fn format_header_lines_for_copy(lines: &[HeaderLine], is_query: bool) -> String {
    lines
        .iter()
        .map(|line| {
            if is_query {
                format!("{}={}", line.name, line.value)
            } else {
                format!("{}: {}", line.name, line.value)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn row_copy_button(
    zh_label: &'static str,
    en_label: &'static str,
    zh_tip: &'static str,
    en_tip: &'static str,
    on_copy: impl Fn() + 'static,
) -> impl IntoElement {
    let copied = signal(false);
    let reset_job = stored(None::<Spawned<()>>);
    view()
        .role(AccessibilityRole::Button)
        .w(px(ControlSize::Xs.height()))
        .h(px(ControlSize::Xs.height()))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(RADIUS_SM))
        .bg(move || {
            if copied.get() {
                status_ok_bg()
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .hover(move |style| {
            style.bg(move || {
                if copied.get() {
                    status_ok_bg()
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
        .label(tr(zh_label, en_label))
        .tooltip(label_tip(zh_tip, en_tip))
        .on_press(move || {
            on_copy();
            copied.set(true);
            reset_job.replace(Some(set_timeout(Duration::from_millis(1400), move || {
                copied.set(false)
            })));
        })
        .child(
            canvas(move || {
                if copied.get() {
                    Painter::new()
                        .line_width(1.5)
                        .stroke_color(color(status_ok_fg()))
                        .stroke_svg("M2.5 6.8 L5.2 9.4 L10.2 3.8")
                } else {
                    Painter::new()
                        .line_width(1.3)
                        .stroke_color(color(fg_secondary()))
                        .stroke_svg("M4.5 4.5 H10.5 V11.0 H4.5 Z M2.5 8.5 H2.0 V2.0 H8.0 V3.0")
                }
            })
            .w(px(12.5))
            .h(px(12.5)),
        )
}

fn hoverable_kv_row(index: ReadSignal<usize>) -> View {
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
        .hover(|style| style.bg(bg_control()))
        .border_t(move || if index.get() > 0 { px(1.0) } else { px(0.0) })
        .border_t_color(border_subtle())
}

pub(crate) fn header_section(props: HeaderSectionProps) -> impl IntoElement {
    let HeaderSectionProps {
        zh,
        en,
        lines,
        loading,
        pane_width,
        folded,
        bit,
        controls,
    } = props;
    let is_query = bit == FOLD_QUERY;
    let fold_controls = controls.clone();
    let copy_all_controls = controls;
    view()
        .rounded(px(RADIUS_LG))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .overflow_hidden()
        .child(
            view()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
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
                .child(
                    view()
                        .role(AccessibilityRole::Button)
                        .flex_1()
                        .min_w(px(0.0))
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .px_1()
                        .py_0p5()
                        .rounded(px(RADIUS_SM))
                        .hover(|style| style.bg(bg_control_hover()))
                        .active(|style| style.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .on_press(move || toggle_card_and_persist(folded, bit, &fold_controls))
                        .child(fold_mark(folded, bit))
                        .child(section_title(tr(zh, en)))
                        .child(
                            view()
                                .px_1p5()
                                .py(px(1.0))
                                .rounded(px(RADIUS_SM))
                                .bg(bg_control())
                                .child(
                                    mono(move || {
                                        if loading.get() {
                                            "…".to_string()
                                        } else {
                                            present::format_thousands(lines.with(Vec::len) as u64)
                                        }
                                    })
                                    .tabular_nums()
                                    .text_size(px(TEXT_XS))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(fg_muted()),
                                ),
                        ),
                )
                .child(when(
                    move || {
                        !card_folded(folded, bit) && !loading.get() && !lines.with(Vec::is_empty)
                    },
                    move || {
                        let controls = copy_all_controls.clone();
                        detail_button(tr("复制全部", "Copy all"), move || {
                            let payload =
                                lines.with(|items| format_header_lines_for_copy(items, is_query));
                            if !payload.is_empty() {
                                set_clipboard(payload);
                                set_status(&controls, StatusMsg::CopiedHeaders { zh, en });
                            }
                        })
                    },
                )),
        )
        .child(when(
            move || !card_folded(folded, bit),
            move || {
                when(move || loading.get(), detail_loading_row).otherwise(move || {
                    when(
                        move || lines.with(Vec::is_empty),
                        || view().px_3p5().py_2p5().child(muted_line(tr("无", "None"))),
                    )
                    .otherwise(move || {
                        each(
                            lines,
                            |line| line.key,
                            move |line, index| {
                                hoverable_kv_row(index)
                                    .child(
                                        mono(move || line.with(|line| line.name.clone()))
                                            .w(move || px(kv_key_width(pane_width.get())))
                                            .flex_shrink_0()
                                            .selectable(true)
                                            .number_of_lines(1)
                                            .text_overflow(EllipsizeMode::Tail)
                                            .text_size(move || scaled_px(TEXT_SM))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(fg_muted()),
                                    )
                                    .child(kv_value_cell(line))
                            },
                        )
                    })
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

pub(crate) fn llm_mode_chip(
    current: Signal<bool>,
    target: bool,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    view()
        .role(AccessibilityRole::Button)
        .h(px(ControlSize::Xs.height()))
        .px_2()
        .items_center()
        .justify_center()
        .bg(move || {
            if current.get() == target {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if current.get() == target {
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
        .on_press(move || current.set(target))
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if current.get() == target {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

pub(crate) fn msg_filter_chip(
    state: Signal<u8>,
    val: u8,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    let active = move || state.get() == val;
    view()
        .role(AccessibilityRole::Button)
        .h(px(ControlSize::Xs.height()))
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
        .active(|style| {
            style.bg(move || {
                if active() {
                    accent_blue_active()
                } else {
                    bg_control_active()
                }
            })
        })
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .on_press(move || state.set(val))
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if active() {
                        fg_on_accent()
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
                .h(px(ControlSize::Xs.height()))
                .px_2()
                .rounded(px(RADIUS_MD))
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
                .text_size(px(TEXT_XS))
                .text_color(fg_primary())
                .on_change(move |text: &str| search.set(text.to_string())),
        )
        .child(when(
            move || !search.get().trim().is_empty(),
            move || {
                view()
                    .role(AccessibilityRole::Button)
                    .h(px(ControlSize::Xs.height()))
                    .px_1p5()
                    .rounded(px(RADIUS_MD))
                    .bg(accent_blue_soft())
                    .hover(|style| style.bg(accent_blue_soft_hover()))
                    .active(|style| style.bg(bg_control_active()))
                    .transition_colors()
                    .duration(MOTION_INSTANT_MS)
                    .ease_out()
                    .focus_visible(apply_focus_ring)
                    .border_1()
                    .border_color(accent_blue_border())
                    .flex_row()
                    .items_center()
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
                            format!("{} ×", present::format_thousands(matched as u64))
                        })
                        .tabular_nums()
                        .text_size(px(TEXT_XS))
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
    let llm_stream = shown.llm_stream;
    let msg_llm_view = shown.msg_llm_view;
    let pane_width = shown.pane_width;
    let copy_all_controls = controls.clone();
    let fold_controls = controls.clone();
    let llm_controls = controls;
    let has_ws_dirs =
        move || lines.with(|items| items.iter().any(|line| present::is_ws_message(&line.name)));
    let has_llm_view = move || llm_stream.with(Option::is_some);
    let is_llm_ai_mode = move || has_llm_view() && msg_llm_view.get();
    view()
        .rounded(px(RADIUS_LG))
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
                        .role(AccessibilityRole::Button)
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .px_1()
                        .py_0p5()
                        .rounded(px(RADIUS_SM))
                        .hover(|style| style.bg(bg_control_hover()))
                        .active(|style| style.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .on_press(move || {
                            toggle_card_and_persist(folded, FOLD_MESSAGES, &fold_controls)
                        })
                        .child(fold_mark(folded, FOLD_MESSAGES))
                        .child(section_title(tr("消息", "Messages")))
                        .child(
                            view()
                                .px_1p5()
                                .py(px(1.0))
                                .rounded(px(RADIUS_SM))
                                .bg(bg_control())
                                .child(
                                    mono(move || {
                                        if shown.messages_loading.get() {
                                            return "…".to_string();
                                        }
                                        let total = lines.with(Vec::len);
                                        let total_str = present::format_thousands(total as u64);
                                        if is_llm_ai_mode()
                                            || (search.get().trim().is_empty()
                                                && dir.get() == 0
                                                && kind.get() == 0)
                                        {
                                            total_str
                                        } else {
                                            let matched = filter_message_lines(
                                                &lines.get(),
                                                &search.get(),
                                                dir.get(),
                                                kind.get(),
                                            )
                                            .len();
                                            format!(
                                                "{}/{total_str}",
                                                present::format_thousands(matched as u64)
                                            )
                                        }
                                    })
                                    .tabular_nums()
                                    .text_size(px(TEXT_XS))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(fg_muted()),
                                ),
                        ),
                )
                .child(when(
                    move || !card_folded(folded, FOLD_MESSAGES) && !shown.messages_loading.get(),
                    move || {
                        let copy_all_controls = copy_all_controls.clone();
                        view()
                            .flex_row()
                            .flex_wrap()
                            .items_center()
                            .gap_1p5()
                            .child(when(has_llm_view, move || {
                                view()
                                    .flex_row()
                                    .h(px(ControlSize::Xs.height()))
                                    .rounded(px(RADIUS_MD))
                                    .border_1()
                                    .border_color(border_control())
                                    .overflow_hidden()
                                    .child(llm_mode_chip(
                                        msg_llm_view,
                                        true,
                                        "AI 聚合",
                                        "AI Stream",
                                    ))
                                    .child(llm_mode_chip(msg_llm_view, false, "原始帧", "Raw"))
                            }))
                            .child(when(
                                move || !is_llm_ai_mode() && has_ws_dirs(),
                                move || {
                                    view()
                                        .flex_row()
                                        .h(px(ControlSize::Xs.height()))
                                        .rounded(px(RADIUS_MD))
                                        .border_1()
                                        .border_color(border_control())
                                        .overflow_hidden()
                                        .child(msg_filter_chip(dir, 0, "全部", "All"))
                                        .child(msg_filter_chip(dir, 1, "↑ 客户端", "↑ Client"))
                                        .child(msg_filter_chip(dir, 2, "↓ 源站", "↓ Origin"))
                                },
                            ))
                            .child(when(
                                move || !is_llm_ai_mode() && has_ws_dirs(),
                                move || {
                                    view()
                                        .flex_row()
                                        .h(px(ControlSize::Xs.height()))
                                        .rounded(px(RADIUS_MD))
                                        .border_1()
                                        .border_color(border_control())
                                        .overflow_hidden()
                                        .child(msg_filter_chip(kind, 0, "全部", "All"))
                                        .child(msg_filter_chip(kind, 1, "文本", "Text"))
                                        .child(msg_filter_chip(kind, 2, "二进制", "Binary"))
                                        .child(msg_filter_chip(kind, 3, "控制", "Control"))
                                },
                            ))
                            .child(when(
                                move || !is_llm_ai_mode(),
                                move || msg_search_box(search, lines, dir, kind),
                            ))
                            .child(when(
                                move || {
                                    !is_llm_ai_mode()
                                        && filter_message_lines(
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
                                            set_status(&controls, StatusMsg::CopiedMessage);
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
                let ai_controls = llm_controls.clone();
                let filtered = move || {
                    filter_message_lines(&lines.get(), &search.get(), dir.get(), kind.get())
                };
                when(move || shown.messages_loading.get(), detail_loading_row).otherwise(
                    move || {
                        let ai_controls = ai_controls.clone();
                        when(is_llm_ai_mode, move || {
                            llm_stream_panel(llm_stream, ai_controls.clone())
                        })
                        .otherwise(move || {
                            when(
                                move || filtered().is_empty(),
                                || view().px_3p5().py_2p5().child(muted_line(tr("无", "None"))),
                            )
                            .otherwise(move || {
                                each(
                                    filtered,
                                    |line| line.key,
                                    move |line, index| {
                                        hoverable_kv_row(index)
                                            .child(
                                                mono(move || line.with(|line| line.name.clone()))
                                                    .w(move || px(kv_key_width(pane_width.get())))
                                                    .flex_shrink_0()
                                                    .selectable(true)
                                                    .number_of_lines(1)
                                                    .text_overflow(EllipsizeMode::Tail)
                                                    .text_size(move || scaled_px(TEXT_SM))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(fg_muted()),
                                            )
                                            .child(
                                                mono(move || line.with(|line| line.value.clone()))
                                                    .flex_1()
                                                    .min_w(px(0.0))
                                                    .selectable(true)
                                                    .text_size(move || scaled_px(TEXT_SM))
                                                    .text_color(fg_primary()),
                                            )
                                    },
                                )
                            })
                        })
                    },
                )
            },
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_header_lines_for_copy_supports_headers_and_query() {
        let lines = vec![
            HeaderLine {
                key: 1,
                name: "content-type".into(),
                value: "application/json".into(),
                hint: None,
            },
            HeaderLine {
                key: 2,
                name: "x-trace-id".into(),
                value: "abc-123".into(),
                hint: None,
            },
        ];
        assert_eq!(
            format_header_lines_for_copy(&lines, false),
            "content-type: application/json\nx-trace-id: abc-123"
        );
        assert_eq!(
            format_header_lines_for_copy(&lines, true),
            "content-type=application/json\nx-trace-id=abc-123"
        );
        assert_eq!(
            StatusMsg::CopiedHeaders {
                zh: "响应头",
                en: "Response headers"
            }
            .text(Lang::Zh),
            "已复制全部响应头"
        );
        assert_eq!(
            StatusMsg::CopiedHeaders {
                zh: "响应头",
                en: "Response headers"
            }
            .text(Lang::En),
            "Copied all response headers"
        );
    }
}
