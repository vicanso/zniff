#[allow(unused_imports)]
use super::*;

pub(crate) fn begin_sequence_diagram(
    controls: &Controls,
    rows: Signal<Vec<Arc<RowLine>>>,
    seq_builder: SeqBuilder,
    compose: Compose,
    selected: Signal<Option<u64>>,
    shown_phase: Signal<Phase>,
) {
    let current_rows = rows.get();
    if current_rows.is_empty() {
        return;
    }
    let mut candidates: Vec<SeqCandidate> = with_active_session(controls, |session| {
        current_rows
            .into_iter()
            .map(|row| {
                if let Some(f) = session.get(row.id) {
                    let failed = f.error.is_some() && f.status.is_none();
                    SeqCandidate {
                        id: row.id,
                        method: f.method.clone(),
                        host: present::resolve_flow_host(f),
                        path: present::resolve_flow_target(f, true),
                        status: f.status,
                        failed,
                        elapsed: present::format_elapsed(f.started, f.finished_at),
                        comment: f.comment.clone(),
                        checked: true,
                    }
                } else {
                    SeqCandidate {
                        id: row.id,
                        method: row.method.clone(),
                        host: row.host.clone(),
                        path: row.path.clone(),
                        status: row.status,
                        failed: row.failed,
                        elapsed: row.elapsed.clone(),
                        comment: row.comment.clone(),
                        checked: true,
                    }
                }
            })
            .collect()
    });
    candidates.sort_by_key(|c| c.id);
    seq_builder.resume_ids.set(Vec::new());
    reset_compose(compose);
    selected.set(None);
    shown_phase.set(Phase::Empty);
    seq_builder.redact_open.set(false);
    seq_builder.items.set(candidates);
    seq_builder.hover_bar.set(None);
    refresh_seq_diagram(controls, seq_builder);
    seq_builder.open.set(true);
}

pub(crate) fn resume_sequence_diagram(
    controls: &Controls,
    rows: Signal<Vec<Arc<RowLine>>>,
    seq_builder: SeqBuilder,
    compose: Compose,
    selected: Signal<Option<u64>>,
    shown_phase: Signal<Phase>,
) {
    let saved_ids: std::collections::HashSet<u64> = seq_builder
        .resume_ids
        .with_untracked(|ids| ids.iter().copied().collect());
    let saved_mode = seq_builder.resume_view_mode.get_untracked();
    begin_sequence_diagram(controls, rows, seq_builder, compose, selected, shown_phase);
    if !saved_ids.is_empty() {
        seq_builder.items.update(|items| {
            let any_match = items.iter().any(|item| saved_ids.contains(&item.id));
            if any_match {
                for item in items {
                    item.checked = saved_ids.contains(&item.id);
                }
            }
        });
        seq_builder.view_mode.set(saved_mode);
        refresh_seq_diagram(controls, seq_builder);
    }
}

pub(crate) fn seq_source_kind_chip(
    source_flowchart: Signal<bool>,
    target_flowchart: bool,
    label: &'static str,
) -> impl IntoElement {
    let is_active = move || source_flowchart.get() == target_flowchart;
    button()
        .selected(is_active)
        .h(px(20.0))
        .px_2()
        .rounded(px(RADIUS_SM))
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
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .border_1()
        .border_color(move || {
            if is_active() {
                border_control()
            } else {
                bg_control()
            }
        })
        .flex_row()
        .items_center()
        .accessibility_label(Some(SharedString::from(label)))
        .on_press(move |_: &()| source_flowchart.set(target_flowchart))
        .child(
            mono(label)
                .number_of_lines(1)
                .text_size(px(TEXT_XS))
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

pub(crate) fn seq_mermaid_source_view(
    seq_builder: SeqBuilder,
    controls: Arc<Controls>,
) -> impl IntoElement {
    view()
        .relative()
        .w_full()
        .h(move || {
            px(seq_preview_height(
                window().height.0,
                seq_builder.items.with(Vec::len),
            ))
        })
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border_control())
        .bg(bg_code())
        .overflow_hidden()
        .child(
            guard_culled_pointer_events(view()).size_full().child(
                editor()
                    .value(move || SharedString::from(active_seq_mermaid(seq_builder)))
                    .read_only(true)
                    .language("plaintext")
                    .line_numbers(true)
                    .folding(false)
                    .soft_wrap(true)
                    .font_size(move || scaled_px(12.0))
                    .line_height(move || scaled_px(18.0))
                    .scroll_beyond_last_line(false)
                    .size_full(),
            ),
        )
        .child(when(
            move || !active_seq_mermaid(seq_builder).is_empty(),
            move || {
                let controls = controls.clone();
                view()
                    .absolute()
                    .top(px(6.0))
                    .right(px(8.0))
                    .flex_row()
                    .items_center()
                    .gap_1p5()
                    .child(when(
                        move || {
                            seq_builder
                                .diagram
                                .with(|d| !d.flowchart_mermaid.is_empty())
                        },
                        move || {
                            view()
                                .h(px(24.0))
                                .px(px(2.0))
                                .rounded(px(RADIUS_SM))
                                .border_1()
                                .border_color(border_control())
                                .bg(bg_control())
                                .flex_row()
                                .items_center()
                                .gap(px(2.0))
                                .child(seq_source_kind_chip(
                                    seq_builder.source_flowchart,
                                    false,
                                    "sequenceDiagram",
                                ))
                                .child(seq_source_kind_chip(
                                    seq_builder.source_flowchart,
                                    true,
                                    "flowchart TD",
                                ))
                        },
                    ))
                    .child(
                        button()
                            .h(px(ControlSize::Xs.height()))
                            .px_2()
                            .rounded(px(RADIUS_SM))
                            .border_1()
                            .border_color(border_control())
                            .bg(bg_card())
                            .hover(|style| style.bg(bg_control_hover()))
                            .active(|style| style.bg(bg_control_active()))
                            .transition_colors()
                            .duration(MOTION_INSTANT_MS)
                            .ease_out()
                            .focus_visible(apply_focus_ring)
                            .flex_row()
                            .items_center()
                            .accessibility_label(tr_opt("复制", "Copy"))
                            .on_press(move |_: &()| {
                                set_clipboard(active_seq_mermaid(seq_builder));
                                set_status(&controls, StatusMsg::CopiedMermaid);
                            })
                            .child(
                                text(tr("复制", "Copy"))
                                    .number_of_lines(1)
                                    .text_size(px(TEXT_XS))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(fg_secondary()),
                            ),
                    )
            },
        ))
}

/// Checkbox rows for the sequence diagram. Virtualized: only the rows near the
/// viewport exist, so a long capture does not grow nsh's node pools.
pub(crate) fn seq_candidate_list(
    seq_builder: SeqBuilder,
    controls: Arc<Controls>,
) -> impl IntoElement {
    virtual_list(
        move || seq_builder.items.with(Vec::len),
        move |at| seq_candidate_row(seq_builder, controls.clone(), at),
    )
    .keys(move || {
        seq_builder
            .items
            .with(|items| items.iter().map(|item| item.id).collect::<Vec<_>>())
    })
    .estimated_item_size(px(32.0))
    .size_full()
}

pub(crate) fn seq_candidate_row(
    seq_builder: SeqBuilder,
    controls: Arc<Controls>,
    at: Position,
) -> impl IntoElement {
    let item = computed(move || {
        seq_builder
            .items
            .with(|items| items.get(at.index()).cloned())
            .unwrap_or_default()
    });
    checkbox()
        .checked(move || item.with(|i| i.checked))
        .w_full()
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .flex_row()
        .items_center()
        .bg(move || {
            if at.index() % 2 == 1 {
                bg_code()
            } else {
                bg_card()
            }
        })
        .border_t(move || if at.index() > 0 { px(1.0) } else { px(0.0) })
        .border_t_color(border_subtle())
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .on_change(move |next: &bool| {
            let index = at.index();
            seq_builder.items.update(|items| {
                if let Some(target) = items.get_mut(index) {
                    target.checked = *next;
                }
            });
            refresh_seq_diagram(&controls, seq_builder);
        })
        .indicator_style(|s| {
            s.w(px(14.0))
                .h(px(14.0))
                .flex_shrink_0()
                .rounded(px(RADIUS_SM))
                .border_1()
                .border_color(border_control())
                .bg(bg_card())
                .items_center()
                .justify_center()
                .checked(|c| c.border_color(accent_blue()).bg(accent_blue()))
        })
        .indicator(when(
            move || item.with(|i| i.checked),
            || {
                canvas(|| {
                    Painter::new()
                        .line_width(1.8)
                        .stroke_color(fg_on_accent().light)
                        .stroke_svg("M2.2 5.2 L4.4 7.4 L8.0 3.0")
                })
                .w(px(10.0))
                .h(px(10.0))
            },
        ))
        .child(
            mono(move || item.with(|i| format!("#{}", i.id)))
                .tabular_nums()
                .min_w(px(36.0))
                .flex_shrink_0()
                .text_size(px(TEXT_XS))
                .text_color(fg_muted()),
        )
        .child(
            view()
                .min_w(px(44.0))
                .px(px(5.0))
                .py_0p5()
                .rounded(px(RADIUS_SM))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .bg(move || item.with(|i| method_bg(&i.method)))
                .child(
                    mono(move || item.with(|i| i.method.clone()))
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::BOLD)
                        .text_color(move || item.with(|i| method_fg(&i.method))),
                ),
        )
        .child(
            mono(move || item.with(|i| seq_candidate_target(i, seq_builder.include_query.get())))
                .flex_1()
                .min_w(px(0.0))
                .number_of_lines(1)
                .text_overflow(EllipsizeMode::Middle)
                .text_size(px(TEXT_SM))
                .text_color(fg_primary()),
        )
        .child(when(
            move || item.with(|i| !i.comment.is_empty()),
            move || {
                view()
                    .px_1p5()
                    .py(px(1.0))
                    .rounded(px(RADIUS_SM))
                    .max_w(px(140.0))
                    .flex_shrink_0()
                    .bg(accent_blue_soft())
                    .child(
                        text(move || item.with(|i| format!("# {}", i.comment)))
                            .number_of_lines(1)
                            .text_overflow(EllipsizeMode::Tail)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(accent_blue_text()),
                    )
            },
        ))
        .child(
            mono(move || item.with(|i| list_status(i.status, i.failed)))
                .tabular_nums()
                .min_w(px(32.0))
                .text_right()
                .flex_shrink_0()
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || item.with(|i| http_status_fg(i.status, i.failed))),
        )
        .child(
            mono(move || item.with(|i| i.elapsed.clone()))
                .tabular_nums()
                .min_w(px(52.0))
                .text_right()
                .flex_shrink_0()
                .text_size(px(TEXT_XS))
                .text_color(fg_muted()),
        )
}

pub(crate) fn sequence_diagram_panel(
    seq_builder: SeqBuilder,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    compose: Compose,
) -> impl IntoElement {
    let select_all_controls = controls.clone();
    let clear_controls = controls.clone();
    let invert_controls = controls.clone();
    let res_controls = controls.clone();
    let query_controls = controls.clone();
    let row_controls = controls.clone();
    let copy_mermaid_controls = controls.clone();
    let copy_md_controls = controls.clone();
    let source_controls = controls;
    let langs = ui_langs();
    let has_checked = move || {
        seq_builder
            .items
            .with(|items| items.iter().any(|item| item.checked))
    };
    view()
        .rounded(px(RADIUS_LG))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .overflow_hidden()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .px_3p5()
                .py_2()
                .border_b_1()
                .border_b_color(border_subtle())
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(section_title(tr("生成时序图", "Sequence diagram")))
                        .child(
                            view()
                                .px_2()
                                .py_0p5()
                                .rounded(px(RADIUS_SM))
                                .bg(accent_blue_soft())
                                .child(
                                    mono({
                                        let langs = ui_langs();
                                        move || {
                                            let (checked, total) =
                                                seq_builder.items.with(|items| {
                                                    let c = items
                                                        .iter()
                                                        .filter(|item| item.checked)
                                                        .count();
                                                    (c, items.len())
                                                });
                                            let checked_str =
                                                present::format_thousands(checked as u64);
                                            let total_str = present::format_thousands(total as u64);
                                            match ui_lang(langs) {
                                                Lang::Zh => format!("已选 {checked_str} / {total_str}"),
                                                Lang::En => {
                                                    format!("{checked_str} / {total_str} selected")
                                                }
                                            }
                                        }
                                    })
                                    .tabular_nums()
                                    .text_size(px(TEXT_XS))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(accent_blue_text()),
                                ),
                        ),
                )
                .child(detail_button(tr("关闭", "Close"), move || {
                    seq_builder.resume_ids.set(Vec::new());
                    close_sequence_diagram(seq_builder);
                })),
        )
        .child(
            view()
                .p_3p5()
                .gap_3()
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .child(detail_button(tr("全选", "Select all"), move || {
                                    seq_builder.items.update(|items| {
                                        for item in items {
                                            item.checked = true;
                                        }
                                    });
                                    refresh_seq_diagram(&select_all_controls, seq_builder);
                                }))
                                .child(detail_button(tr("全不选", "Clear"), move || {
                                    seq_builder.items.update(|items| {
                                        for item in items {
                                            item.checked = false;
                                        }
                                    });
                                    refresh_seq_diagram(&clear_controls, seq_builder);
                                }))
                                .child(detail_button(tr("反选", "Invert"), move || {
                                    seq_builder.items.update(|items| {
                                        for item in items {
                                            item.checked = !item.checked;
                                        }
                                    });
                                    refresh_seq_diagram(&invert_controls, seq_builder);
                                })),
                        )
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .child(seq_option_toggle(
                                    seq_builder.include_response,
                                    "含响应",
                                    "Responses",
                                    move || {
                                        refresh_seq_diagram(&res_controls, seq_builder);
                                    },
                                ))
                                .child(seq_option_toggle(
                                    seq_builder.include_query,
                                    "含查询参数",
                                    "Query string",
                                    move || {
                                        refresh_seq_diagram(&query_controls, seq_builder);
                                    },
                                )),
                        ),
                )
                .child(
                    view()
                        .w_full()
                        .h(move || {
                            let max_rows = seq_candidate_max_rows(window().height.0);
                            let rows = seq_builder.items.with(Vec::len).clamp(1, max_rows) as f32;
                            px(rows * 32.0)
                        })
                        .rounded(px(RADIUS_MD))
                        .border_1()
                        .border_color(border_subtle())
                        .bg(bg_code())
                        .overflow_hidden()
                        .child(seq_candidate_list(seq_builder, row_controls)),
                )
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .child(
                                    button()
                                        .h(px(CONTROL_H))
                                        .px_3()
                                        .rounded(px(RADIUS_MD))
                                        .bg(move || {
                                            if has_checked() {
                                                accent_blue()
                                            } else {
                                                bg_control()
                                            }
                                        })
                                        .hover(|style| {
                                            style.bg(move || {
                                                if has_checked() {
                                                    accent_blue_hover()
                                                } else {
                                                    bg_control()
                                                }
                                            })
                                        })
                                        .active(|style| {
                                            style.bg(move || {
                                                if has_checked() {
                                                    accent_blue_active()
                                                } else {
                                                    bg_control()
                                                }
                                            })
                                        })
                                        .transition_colors()
                                        .duration(MOTION_INSTANT_MS)
                                        .ease_out()
                                        .focus_visible(apply_focus_ring)
                                        .disabled(move || !has_checked())
                                        .flex_row()
                                        .items_center()
                                        .on_press(move |_: &()| {
                                            let count = seq_builder.items.with(|items| {
                                                items.iter().filter(|i| i.checked).count()
                                            });
                                            if count == 0 {
                                                return;
                                            }
                                            let _ = refresh_seq_diagram(
                                                &copy_mermaid_controls,
                                                seq_builder,
                                            );
                                            set_clipboard(active_seq_mermaid(seq_builder));
                                            set_status(
                                                &copy_mermaid_controls,
                                                StatusMsg::SequenceGenerated { count },
                                            );
                                        })
                                        .child(
                                            text(move || {
                                                let lang = ui_lang(langs);
                                                let is_flow = seq_builder.view_mode.get() == 2
                                                    || (seq_builder.view_mode.get() == 3
                                                        && seq_builder.source_flowchart.get());
                                                if is_flow {
                                                    t(
                                                        lang,
                                                        "复制 Mermaid (流程图)",
                                                        "Copy Mermaid (Flowchart)",
                                                    )
                                                } else {
                                                    t(
                                                        lang,
                                                        "复制 Mermaid (时序图)",
                                                        "Copy Mermaid (Sequence)",
                                                    )
                                                }
                                            })
                                            .number_of_lines(1)
                                            .text_size(px(TEXT_SM))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || {
                                                if has_checked() {
                                                    fg_on_accent()
                                                } else {
                                                    fg_muted()
                                                }
                                            }),
                                        ),
                                )
                                .child(seq_action_button(
                                    tr("复制 Markdown", "Copy Markdown"),
                                    has_checked,
                                    move || {
                                        let count = seq_builder.items.with(|items| {
                                            items.iter().filter(|i| i.checked).count()
                                        });
                                        if count == 0 {
                                            return;
                                        }
                                        let _ = refresh_seq_diagram(&copy_md_controls, seq_builder);
                                        let diagram = active_seq_mermaid(seq_builder);
                                        set_clipboard(present::mermaid_markdown_block(&diagram));
                                        set_status(
                                            &copy_md_controls,
                                            StatusMsg::CopiedMermaidMarkdown,
                                        );
                                    },
                                )),
                        )
                        .child(
                            tabs()
                                .orientation(Axis::Horizontal)
                                .accessibility_label(tr_opt("时序视图", "Sequence view"))
                                .h(px(CONTROL_H))
                                .px(px(2.0))
                                .rounded(px(RADIUS_MD))
                                .border_1()
                                .border_color(border_subtle())
                                .bg(bg_control())
                                .flex_row()
                                .items_center()
                                .gap(px(2.0))
                                .child(seq_mode_tab(
                                    seq_builder.view_mode,
                                    seq_builder.source_flowchart,
                                    0,
                                    "瀑布图",
                                    "Waterfall",
                                ))
                                .child(seq_mode_tab(
                                    seq_builder.view_mode,
                                    seq_builder.source_flowchart,
                                    1,
                                    "时序图",
                                    "Sequence",
                                ))
                                .child(seq_mode_tab(
                                    seq_builder.view_mode,
                                    seq_builder.source_flowchart,
                                    2,
                                    "依赖图",
                                    "Dependency",
                                ))
                                .child(seq_mode_tab(
                                    seq_builder.view_mode,
                                    seq_builder.source_flowchart,
                                    3,
                                    "源码",
                                    "Mermaid",
                                )),
                        ),
                )
                .child(when(
                    move || {
                        seq_builder
                            .items
                            .with(|items| items.iter().filter(|i| i.checked).count())
                            > SEQ_PREVIEW_CAP
                    },
                    move || {
                        let langs = ui_langs();
                        view()
                            .w_full()
                            .px_3()
                            .py_1p5()
                            .rounded(px(RADIUS_MD))
                            .bg(status_warn_bg())
                            .border_1()
                            .border_color(status_warn_fg())
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(glyph_px(13.0, || Glyph::Alert, || color(status_warn_fg())))
                            .child(
                                text(move || {
                                    let checked = present::format_thousands(
                                        seq_builder
                                            .items
                                            .with(|items| items.iter().filter(|i| i.checked).count())
                                            as u64,
                                    );
                                    let is_flow = seq_builder.view_mode.get() == 2
                                        || (seq_builder.view_mode.get() == 3
                                            && seq_builder.source_flowchart.get());
                                    match (ui_lang(langs), is_flow) {
                                        (Lang::Zh, true) => format!(
                                            "依赖图 / 流程图仅覆盖最近 {SEQ_PREVIEW_CAP} 条；切回「时序图」可复制全部 {checked} 条"
                                        ),
                                        (Lang::En, true) => format!(
                                            "Dependency / Flowchart covers newest {SEQ_PREVIEW_CAP} flows; switch to Sequence to copy all {checked}"
                                        ),
                                        (Lang::Zh, false) => format!(
                                            "仅预览最近 {SEQ_PREVIEW_CAP} 条；复制 Mermaid / Markdown 仍包含全部 {checked} 条"
                                        ),
                                        (Lang::En, false) => format!(
                                            "Previewing newest {SEQ_PREVIEW_CAP} flows; Copy Mermaid / Markdown still includes all {checked}"
                                        ),
                                    }
                                })
                                .flex_1()
                                .min_w(px(0.0))
                                .tabular_nums()
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(status_warn_fg()),
                            )
                    },
                ))
                .child(when(
                    move || seq_builder.view_mode.get() == 0,
                    move || seq_waterfall_view(seq_builder, selected, compose),
                ))
                .child(when(
                    move || seq_builder.view_mode.get() == 1,
                    move || seq_swimlane_view(seq_builder, selected, compose),
                ))
                .child(when(
                    move || seq_builder.view_mode.get() == 2,
                    move || seq_dependency_view(seq_builder, selected, compose),
                ))
                .child(when(
                    move || seq_builder.view_mode.get() == 3,
                    move || seq_mermaid_source_view(seq_builder, source_controls.clone()),
                )),
        )
}
