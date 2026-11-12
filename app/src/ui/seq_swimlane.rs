#[allow(unused_imports)]
use super::*;

pub(crate) fn refresh_seq_diagram(controls: &Controls, seq_builder: SeqBuilder) -> String {
    let include_res = seq_builder.include_response.get();
    let include_query = seq_builder.include_query.get();
    let data = controls.hub.with_session(|session| {
        seq_builder.items.update(|items| {
            for item in items.iter_mut() {
                if let Some(f) = session.get(item.id) {
                    item.method = f.method.clone();
                    item.host = present::resolve_flow_host(f);
                    item.path = present::resolve_flow_target(f, true);
                    item.status = f.status;
                    item.failed = f.error.is_some() && f.status.is_none();
                    item.elapsed = present::format_elapsed(f.started, f.finished_at);
                    item.comment = f.comment.clone();
                }
            }
        });
        let mut checked_ids: Vec<u64> = seq_builder.items.with(|items| {
            items
                .iter()
                .filter(|item| item.checked)
                .map(|item| item.id)
                .collect()
        });
        checked_ids.sort_unstable();
        // The waterfall, sequence and dependency previews draw one view per
        // flow (or per arrow), and nsh keeps the grown node pools after the
        // panel closes. Preview only the newest checked flows; the copied
        // Mermaid still covers every checked flow.
        let preview_ids = &checked_ids[checked_ids.len().saturating_sub(SEQ_PREVIEW_CAP)..];
        let hydrated_flows: Vec<crate::session::Flow> = preview_ids
            .iter()
            .filter_map(|id| session.hydrated(*id))
            .collect();
        let flow_refs: Vec<&crate::session::Flow> = hydrated_flows.iter().collect();
        let mut data = present::build_sequence_diagram_data(&flow_refs, include_res, include_query);
        if checked_ids.len() > preview_ids.len() {
            let all: Vec<&crate::session::Flow> = checked_ids
                .iter()
                .filter_map(|id| session.get(*id))
                .collect();
            data.mermaid = present::build_sequence_mermaid(&all, include_res, include_query);
        }
        data
    });
    let mermaid = data.mermaid.clone();
    seq_builder.diagram.set(data);
    seq_builder.output.set(mermaid.clone());
    crate::session::trim_memory();
    mermaid
}

pub(crate) fn seq_candidate_target(item: &SeqCandidate, include_query: bool) -> String {
    let path = if include_query {
        item.path.as_str()
    } else {
        item.path.split(['?', '#']).next().unwrap_or(&item.path)
    };
    let path = if path.is_empty() { "/" } else { path };
    if item.host.is_empty() {
        path.to_string()
    } else if path.starts_with('/') || path.starts_with(':') {
        format!("{}{}", item.host, path)
    } else {
        format!("{}/{}", item.host, path)
    }
}

pub(crate) fn seq_action_button(
    label: impl IntoReactive<SharedString>,
    enabled: impl Fn() -> bool + 'static + Clone,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let enabled_border = enabled.clone();
    let enabled_hover = enabled.clone();
    let enabled_active = enabled.clone();
    let enabled_off = enabled.clone();
    let enabled_text = enabled;
    view()
        .h(px(26.0))
        .px_2p5()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || {
            if enabled_border() {
                border_control()
            } else {
                border_subtle()
            }
        })
        .bg(bg_control())
        .hover(|style| {
            style.bg(move || {
                if enabled_hover() {
                    bg_control_hover()
                } else {
                    bg_control()
                }
            })
        })
        .active(|style| style.opacity(move || if enabled_active() { 0.75_f32 } else { 1.0_f32 }))
        .cursor(Cursor::Pointer)
        .disabled(move || !enabled_off())
        .disabled_style(|style| style.cursor(Cursor::Default))
        .flex_row()
        .items_center()
        .on_press(on_press)
        .child(control_text(label).text_color(move || {
            if enabled_text() {
                fg_secondary()
            } else {
                fg_muted()
            }
        }))
}

pub(crate) fn seq_option_toggle(
    on: Signal<bool>,
    zh: &'static str,
    en: &'static str,
    on_toggle: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .h(px(26.0))
        .px_2p5()
        .rounded(px(6.0))
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
        .flex_row()
        .items_center()
        .gap_1p5()
        .on_press(move || {
            on.set(!on.get());
            on_toggle();
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
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if on.get() {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

pub(crate) fn jump_to_seq_flow(
    flow_id: u64,
    selected: Signal<Option<u64>>,
    compose: Compose,
    seq_builder: SeqBuilder,
) {
    if flow_id == 0 {
        return;
    }
    reset_compose(compose);
    close_sequence_diagram(seq_builder);
    selected.set(Some(flow_id));
}

pub(crate) fn select_seq_mermaid_text<'a>(
    view_mode: u8,
    source_flowchart: bool,
    sequence_mermaid: &'a str,
    flowchart_mermaid: &'a str,
) -> &'a str {
    let want_flowchart = view_mode == 2 || (view_mode == 3 && source_flowchart);
    if want_flowchart && !flowchart_mermaid.is_empty() {
        flowchart_mermaid
    } else {
        sequence_mermaid
    }
}

pub(crate) fn active_seq_mermaid(seq_builder: SeqBuilder) -> String {
    let view_mode = seq_builder.view_mode.get();
    let source_flowchart = seq_builder.source_flowchart.get();
    let seq_text = seq_builder.output.get();
    seq_builder.diagram.with(|d| {
        select_seq_mermaid_text(view_mode, source_flowchart, &seq_text, &d.flowchart_mermaid)
            .to_string()
    })
}

pub(crate) fn seq_mode_tab(
    mode: Signal<u8>,
    source_flowchart: Signal<bool>,
    target: u8,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    let is_active = move || mode.get() == target;
    view()
        .h(px(22.0))
        .px_2p5()
        .rounded(px(5.0))
        .bg(move || if is_active() { bg_card() } else { bg_control() })
        .hover(|style| {
            style.bg(move || {
                if is_active() {
                    bg_card()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.85))
        .border_1()
        .border_color(move || {
            if is_active() {
                border_control()
            } else {
                bg_control()
            }
        })
        .cursor(Cursor::Pointer)
        .flex_row()
        .items_center()
        .on_press(move || {
            if target == 2 {
                source_flowchart.set(true);
            } else if target == 0 || target == 1 {
                source_flowchart.set(false);
            }
            mode.set(target);
        })
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || {
                    if is_active() {
                        fg_primary()
                    } else {
                        fg_muted()
                    }
                }),
        )
}

pub(crate) fn swimlane_pill_width(host: &str) -> f32 {
    ((host.chars().count() + 4) as f32 * 7.2 + 28.0).clamp(136.0, 280.0)
}

pub(crate) fn swimlane_layout(
    diagram: &present::SequenceDiagramData,
    viewport_w: f32,
) -> (Vec<f32>, f32) {
    let hosts_len = diagram.hosts.len();
    let x0 = 58.0_f32;
    if hosts_len == 0 {
        let w = viewport_w.max(520.0);
        return (vec![x0, (w - 80.0).max(280.0)], w);
    }
    let mut xs = Vec::with_capacity(hosts_len + 1);
    xs.push(x0);

    let mut prev_half = 46.0_f32;
    for (i, host) in diagram.hosts.iter().enumerate() {
        let pill_w = swimlane_pill_width(host);
        let half = pill_w * 0.5;
        let min_gap = (prev_half + half + 32.0).max(180.0);

        let max_step_span = diagram
            .steps
            .iter()
            .filter(|s| s.host_idx == i)
            .map(|s| {
                let label_w = s.label.chars().count() as f32 * 7.0;
                let note_w = if s.note.is_empty() {
                    0.0
                } else {
                    (s.note.chars().count().min(24) as f32 * 6.8 + 22.0).min(150.0)
                };
                (42.0 + label_w + note_w + 32.0).clamp(240.0, 760.0)
            })
            .fold(240.0_f32, f32::max);

        let prev_x = *xs.last().unwrap_or(&x0);
        let next_x = (prev_x + min_gap).max(x0 + max_step_span);
        xs.push(next_x);
        prev_half = half;
    }

    let right_pad = (prev_half + 28.0).max(96.0);
    let content_w = xs.last().copied().unwrap_or(x0) + right_pad;
    let total_w = content_w.max(viewport_w.max(520.0));

    if total_w > content_w && hosts_len > 0 {
        let extra = (total_w - content_w).min(240.0 * hosts_len as f32);
        let per_lane = extra / hosts_len as f32;
        for (idx, x) in xs.iter_mut().enumerate().skip(1) {
            *x += per_lane * idx as f32;
        }
    }

    (xs, total_w)
}

pub(crate) fn seq_candidate_max_rows(window_h: f32) -> usize {
    let win_h = if window_h > 0.0 { window_h } else { 760.0 };
    if win_h < 620.0 {
        3
    } else if win_h < 740.0 {
        5
    } else {
        7
    }
}

pub(crate) fn seq_preview_height(window_h: f32, candidate_count: usize) -> f32 {
    let win_h = if window_h > 0.0 { window_h } else { 760.0 };
    let max_rows = seq_candidate_max_rows(win_h);
    let candidate_h = candidate_count.clamp(1, max_rows) as f32 * 32.0;
    let chrome_h = 276.0 + candidate_h;
    (win_h - chrome_h).clamp(180.0, 960.0)
}

pub(crate) const SEQ_SWIMLANE_STEP_H: f32 = 42.0;

pub(crate) fn seq_swimlane_view(
    seq_builder: SeqBuilder,
    selected: Signal<Option<u64>>,
    compose: Compose,
) -> impl IntoElement {
    const HEADER_H: f32 = 42.0;
    let viewport_w = seq_builder.viewport_w;
    let layout = seq_builder.layout;

    view()
        .w_full()
        .h(move || {
            px(seq_preview_height(
                window().height.0,
                seq_builder.items.with(Vec::len),
            ))
        })
        .rounded(px(7.0))
        .border_1()
        .border_color(border_control())
        .bg(bg_code())
        .overflow_hidden()
        .on_layout(move |bounds: &Bounds<Points>| {
            let w = bounds.size.width.0;
            if (w - viewport_w.get_untracked()).abs() > 1.0 {
                viewport_w.set(w);
            }
        })
        .child(when(
            move || seq_builder.diagram.with(|d| d.steps.is_empty()),
            || {
                view().size_full().items_center().justify_center().child(
                    text(tr("请勾选至少一条流量", "Select at least one flow"))
                        .text_size(px(12.0))
                        .text_color(fg_muted()),
                )
            },
        ))
        .child(when(
            move || seq_builder.diagram.with(|d| !d.steps.is_empty()),
            move || {
                scroll().horizontal().size_full().child(
                    uncull_nested_scroll(view())
                        .h_full()
                        .w(move || layout.with(|(_, total_w)| px(*total_w)))
                        .child(
                            view()
                                .w_full()
                                .h(px(HEADER_H))
                                .flex_shrink_0()
                                .relative()
                                .border_b_1()
                                .border_b_color(border_subtle())
                                .bg(bg_card())
                                .child(
                                    view()
                                        .absolute()
                                        .left(move || {
                                            let x0 = layout.with(|(xs, _)| {
                                                xs.first().copied().unwrap_or(58.0)
                                            });
                                            px((x0 - 38.0).max(8.0))
                                        })
                                        .top(px(8.0))
                                        .w(px(76.0))
                                        .h(px(26.0))
                                        .rounded(px(6.0))
                                        .border_1()
                                        .border_color(accent_blue_border())
                                        .bg(accent_blue_soft())
                                        .flex_row()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            mono(tr("客户端", "Client"))
                                                .text_size(px(11.5))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(accent_blue_text()),
                                        ),
                                )
                                .child(each(
                                    move || {
                                        seq_builder.diagram.with(|d| {
                                            d.hosts
                                                .iter()
                                                .cloned()
                                                .enumerate()
                                                .collect::<Vec<(usize, String)>>()
                                        })
                                    },
                                    |(idx, _)| *idx,
                                    move |pair, _| {
                                        view()
                                            .absolute()
                                            .left(move || {
                                                let (idx, host) = pair.with(|p| p.clone());
                                                let x = layout.with(|(xs, _)| {
                                                    xs.get(idx + 1).copied().unwrap_or(300.0)
                                                });
                                                let pill_w = swimlane_pill_width(&host);
                                                px((x - pill_w * 0.5).max(8.0))
                                            })
                                            .top(px(8.0))
                                            .w(move || {
                                                pair.with(|(_, host)| px(swimlane_pill_width(host)))
                                            })
                                            .h(px(26.0))
                                            .px_2p5()
                                            .rounded(px(6.0))
                                            .border_1()
                                            .border_color(border_control())
                                            .bg(bg_control())
                                            .flex_row()
                                            .items_center()
                                            .justify_center()
                                            .gap_1()
                                            .overflow_hidden()
                                            .child(
                                                mono(move || {
                                                    format!("H{}", pair.with(|(i, _)| i + 1))
                                                })
                                                .tabular_nums()
                                                .flex_shrink_0()
                                                .text_size(px(11.0))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(accent_blue_text()),
                                            )
                                            .child(
                                                mono(move || pair.with(|(_, host)| host.clone()))
                                                    .min_w(px(0.0))
                                                    .number_of_lines(1)
                                                    .text_overflow(EllipsizeMode::Middle)
                                                    .text_size(px(11.0))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(fg_primary()),
                                            )
                                    },
                                )),
                        )
                        .child(
                            virtual_list(
                                move || seq_builder.diagram.with(|d| d.steps.len()),
                                move |at| {
                                    seq_swimlane_row(seq_builder, layout, selected, compose, at)
                                },
                            )
                            .keys(move || {
                                seq_builder.diagram.with(|d| {
                                    d.steps
                                        .iter()
                                        .map(|s| (s.step_no, s.flow_id, s.is_request))
                                        .collect::<Vec<_>>()
                                })
                            })
                            .estimated_item_size(px(SEQ_SWIMLANE_STEP_H))
                            .flex_1()
                            .w_full(),
                        ),
                )
            },
        ))
}

pub(crate) fn seq_swimlane_row(
    seq_builder: SeqBuilder,
    layout: Memo<(Vec<f32>, f32)>,
    selected: Signal<Option<u64>>,
    compose: Compose,
    at: Position,
) -> impl IntoElement {
    let step = computed(move || {
        seq_builder
            .diagram
            .with(|d| d.steps.get(at.index()).cloned())
            .unwrap_or_default()
    });
    view()
        .w(move || layout.with(|(_, total_w)| px(*total_w)))
        .h(px(SEQ_SWIMLANE_STEP_H))
        .relative()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.opacity(0.85))
        .cursor(Cursor::Pointer)
        .on_press(move || {
            jump_to_seq_flow(step.with(|s| s.flow_id), selected, compose, seq_builder);
        })
        .child(
            canvas(move || {
                step.with(|s| {
                    layout.with(|(xs, _)| {
                        let mut p = Painter::new();
                        for (k, &x) in xs.iter().enumerate() {
                            p = p
                                .line_width(1.0)
                                .stroke_color(color(border_control()))
                                .stroke_svg(&format!("M{x:.1} 0 L{x:.1} {SEQ_SWIMLANE_STEP_H:.1}"));
                            let active =
                                k == 0 || s.active_hosts.get(k - 1).copied().unwrap_or(false);
                            if active {
                                p = p
                                    .line_width(3.6)
                                    .stroke_color(color(accent_blue()))
                                    .stroke_svg(&format!(
                                        "M{x:.1} 0 L{x:.1} {SEQ_SWIMLANE_STEP_H:.1}"
                                    ));
                            }
                        }
                        let x_client = xs.first().copied().unwrap_or(58.0);
                        let x_host = xs.get(s.host_idx + 1).copied().unwrap_or(x_client + 280.0);
                        let (x_from, x_to) = if s.is_request {
                            (x_client + 3.0, x_host - 3.0)
                        } else {
                            (x_host - 3.0, x_client + 3.0)
                        };
                        let y_arrow = 29.0_f32;
                        let stroke = color(if s.is_error {
                            status_err_fg()
                        } else if s.is_request {
                            accent_blue()
                        } else {
                            status_ok_fg()
                        });
                        if s.is_request {
                            p = p.line_width(1.6).stroke_color(stroke).stroke_svg(&format!(
                                "M{x_from:.1} {y_arrow:.1} L{x_to:.1} {y_arrow:.1}"
                            ));
                        } else {
                            let dir = if x_to >= x_from { 1.0_f32 } else { -1.0_f32 };
                            let span = (x_to - x_from).abs();
                            let mut cur = 0.0_f32;
                            let mut segs = String::new();
                            while cur < span {
                                let next = (cur + 6.0).min(span);
                                let a = x_from + dir * cur;
                                let b = x_from + dir * next;
                                use std::fmt::Write;
                                let _ = write!(segs, "M{a:.1} {y_arrow:.1} L{b:.1} {y_arrow:.1} ");
                                cur += 10.0;
                            }
                            if !segs.is_empty() {
                                p = p.line_width(1.5).stroke_color(stroke).stroke_svg(&segs);
                            }
                        }
                        let dir = if x_to >= x_from { 1.0_f32 } else { -1.0_f32 };
                        let ax = x_to - dir * 6.0;
                        let ay1 = y_arrow - 3.4;
                        let ay2 = y_arrow + 3.4;
                        p.line_width(1.6).stroke_color(stroke).stroke_svg(&format!(
                            "M{ax:.1} {ay1:.1} L{x_to:.1} {y_arrow:.1} L{ax:.1} {ay2:.1}"
                        ))
                    })
                })
            })
            .size_full(),
        )
        .child(
            view()
                .absolute()
                .left(move || {
                    let x0 = layout.with(|(xs, _)| xs.first().copied().unwrap_or(58.0));
                    px(x0 + 8.0)
                })
                .top(px(5.0))
                .max_w(move || {
                    let host_idx = step.with(|s| s.host_idx);
                    layout.with(|(xs, _)| {
                        let x0 = xs.first().copied().unwrap_or(58.0);
                        let x_host = xs.get(host_idx + 1).copied().unwrap_or(x0 + 280.0);
                        px((x_host - x0 - 16.0).max(160.0))
                    })
                })
                .flex_row()
                .items_center()
                .gap_1()
                .overflow_hidden()
                .child(
                    view()
                        .px_1p5()
                        .py(px(0.5))
                        .rounded(px(3.5))
                        .flex_shrink_0()
                        .bg(bg_control())
                        .child(
                            mono(move || step.with(|s| s.step_no.to_string()))
                                .tabular_nums()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(fg_secondary()),
                        ),
                )
                .child(
                    view()
                        .px_1p5()
                        .py(px(0.5))
                        .rounded(px(3.5))
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .bg(bg_card())
                        .child(
                            mono(move || step.with(|s| s.label.clone()))
                                .max_w(move || {
                                    step.with(|s| {
                                        layout.with(|(xs, _)| {
                                            let x0 = xs.first().copied().unwrap_or(58.0);
                                            let x_host = xs
                                                .get(s.host_idx + 1)
                                                .copied()
                                                .unwrap_or(x0 + 280.0);
                                            let reserve =
                                                if !s.note.is_empty() { 180.0 } else { 56.0 };
                                            px((x_host - x0 - reserve).max(120.0))
                                        })
                                    })
                                })
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Middle)
                                .text_size(px(11.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || {
                                    step.with(|s| {
                                        if s.is_error {
                                            status_err_fg()
                                        } else if s.is_request {
                                            fg_primary()
                                        } else {
                                            status_ok_fg()
                                        }
                                    })
                                }),
                        ),
                )
                .child(when(
                    move || step.with(|s| !s.note.is_empty()),
                    move || {
                        view()
                            .px_1p5()
                            .py(px(0.5))
                            .rounded(px(3.5))
                            .max_w(px(140.0))
                            .flex_shrink_0()
                            .bg(accent_blue_soft())
                            .child(
                                text(move || step.with(|s| format!("# {}", s.note)))
                                    .number_of_lines(1)
                                    .text_overflow(EllipsizeMode::Tail)
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(accent_blue_text()),
                            )
                    },
                )),
        )
}
