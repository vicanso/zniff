#[allow(unused_imports)]
use super::*;

pub(crate) fn dep_kind_prefix(kind: present::DepKind, lang: Lang) -> &'static str {
    match (kind, lang) {
        (present::DepKind::Cookie, Lang::Zh) => "[Cookie] 依赖",
        (present::DepKind::Cookie, Lang::En) => "[Cookie]",
        (present::DepKind::Token, Lang::Zh) => "[Token] 依赖",
        (present::DepKind::Token, Lang::En) => "[Token]",
        (present::DepKind::Redirect, Lang::Zh) => "↪ 重定向依赖",
        (present::DepKind::Redirect, Lang::En) => "↪ Redirect",
        (present::DepKind::QueryParam, Lang::Zh) => "[Query] 参数依赖",
        (present::DepKind::QueryParam, Lang::En) => "[Query]",
        (present::DepKind::BodyOrPath, Lang::Zh) => "[Data] 字段依赖",
        (present::DepKind::BodyOrPath, Lang::En) => "[Data]",
        (present::DepKind::Sequential, Lang::Zh) => "[After] 时序后继",
        (present::DepKind::Sequential, Lang::En) => "[After]",
    }
}

#[derive(Clone, PartialEq, Default)]
pub(crate) struct DepFlatRow {
    pub(crate) stage_no: usize,
    pub(crate) stage_bar_count: usize,
    pub(crate) stage_offset_label: String,
    pub(crate) stage_span_label: String,
    pub(crate) stage_has_hard_dep: bool,
    pub(crate) is_first_in_stage: bool,
    pub(crate) is_last_in_stage: bool,
    pub(crate) bar: present::WaterfallBar,
}

pub(crate) fn build_dep_flat_rows(d: &present::SequenceDiagramData) -> Vec<DepFlatRow> {
    let mut out = Vec::with_capacity(d.bars.len());
    for stage in &d.stages {
        let count = stage.bar_indices.len();
        let has_hard = stage.bar_indices.iter().any(|&idx| {
            d.bars
                .get(idx)
                .is_some_and(|b| b.deps.iter().any(|dep| dep.kind.is_hard()))
        });
        for (pos, &bar_idx) in stage.bar_indices.iter().enumerate() {
            let Some(bar) = d.bars.get(bar_idx).cloned() else {
                continue;
            };
            out.push(DepFlatRow {
                stage_no: stage.stage_no,
                stage_bar_count: count,
                stage_offset_label: stage.offset_label.clone(),
                stage_span_label: stage.span_label.clone(),
                stage_has_hard_dep: has_hard,
                is_first_in_stage: pos == 0,
                is_last_in_stage: pos + 1 == count,
                bar,
            });
        }
    }
    out
}

pub(crate) fn seq_dependency_view(
    seq_builder: SeqBuilder,
    selected: Signal<Option<u64>>,
    compose: Compose,
) -> impl IntoElement {
    let langs = ui_langs();
    let flat_rows = seq_builder.flat_rows;
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
        .child(when(
            move || seq_builder.diagram.with(|d| d.stages.is_empty()),
            || {
                view()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .child(
                        text(tr("请勾选至少一条流量", "Select at least one flow"))
                            .text_size(px(12.0))
                            .text_color(fg_muted()),
                    )
            },
        ))
        .child(when(
            move || seq_builder.diagram.with(|d| !d.stages.is_empty()),
            move || {
                view()
                    .size_full()
                    .child(
                        view()
                            .h(px(30.0))
                            .px_3()
                            .flex_shrink_0()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .border_b_1()
                            .border_b_color(border_subtle())
                            .bg(bg_card())
                            .child(
                                text(move || {
                                    let lang = ui_lang(langs);
                                    seq_builder.diagram.with(|d| {
                                        let stage_cnt = d.stages.len();
                                        let conc_cnt = d
                                            .stages
                                            .iter()
                                            .filter(|s| s.bar_indices.len() > 1)
                                            .count();
                                        let hard_cnt = d.hard_dep_count;
                                        match lang {
                                            Lang::Zh => format!(
                                                "共 {stage_cnt} 个阶段 · {conc_cnt} 组并发 · 识别出 {hard_cnt} 处参数/Cookie 依赖"
                                            ),
                                            Lang::En => format!(
                                                "{stage_cnt} stages · {conc_cnt} concurrent groups · {hard_cnt} data dependencies"
                                            ),
                                        }
                                    })
                                })
                                .tabular_nums()
                                .number_of_lines(1)
                                .text_size(px(11.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg_secondary()),
                            )
                            .child(
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        view()
                                            .px_1p5()
                                            .py(px(1.0))
                                            .rounded(px(4.0))
                                            .bg(accent_blue_soft())
                                            .child(
                                                text(tr("↪ 参数/Cookie 传递", "↪ Data flow"))
                                                    .text_size(px(11.0))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(accent_blue_text()),
                                            ),
                                    )
                                    .child(
                                        view()
                                            .px_1p5()
                                            .py(px(1.0))
                                            .rounded(px(4.0))
                                            .bg(status_warn_bg())
                                            .child(
                                                text(tr("★ 关键路径", "★ Critical path"))
                                                    .text_size(px(11.0))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(status_warn_fg()),
                                            ),
                                    ),
                            ),
                    )
                    .child(
                        virtual_list(
                            move || flat_rows.with(Vec::len),
                            move |at| {
                                seq_dependency_row(
                                    seq_builder,
                                    flat_rows,
                                    selected,
                                    compose,
                                    langs,
                                    at,
                                )
                            },
                        )
                        .keys(move || {
                            flat_rows.with(|rows| {
                                rows.iter().map(|r| r.bar.flow_id).collect::<Vec<_>>()
                            })
                        })
                        .estimated_item_size(px(56.0))
                        .content_padding(px(12.0))
                        .flex_1()
                        .w_full(),
                    )
            },
        ))
}

pub(crate) fn seq_dep_badge(
    row: Memo<DepFlatRow>,
    langs: Signal<Option<Lang>>,
    slot: usize,
) -> impl IntoElement {
    let dep = computed(move || row.with(|r| r.bar.deps.get(slot).cloned()));
    when(
        move || dep.with(Option::is_some),
        move || {
            view()
                .px_2()
                .py(px(1.5))
                .rounded(px(4.0))
                .border_1()
                .border_color(move || {
                    if dep.with(|d| d.as_ref().is_some_and(|d| d.kind.is_hard())) {
                        accent_blue_border()
                    } else {
                        border_subtle()
                    }
                })
                .bg(move || {
                    if dep.with(|d| d.as_ref().is_some_and(|d| d.kind.is_hard())) {
                        accent_blue_soft()
                    } else {
                        bg_control()
                    }
                })
                .child(
                    mono(move || {
                        dep.with(|d| {
                            let Some(d) = d else {
                                return String::new();
                            };
                            let prefix = dep_kind_prefix(d.kind, ui_lang(langs));
                            format!("{prefix} #{} · {}", d.from_flow_id, d.label)
                        })
                    })
                    .tabular_nums()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || {
                        if dep.with(|d| d.as_ref().is_some_and(|d| d.kind.is_hard())) {
                            accent_blue_text()
                        } else {
                            fg_secondary()
                        }
                    }),
                )
        },
    )
}

pub(crate) fn seq_dependency_row(
    seq_builder: SeqBuilder,
    rows: Memo<Vec<DepFlatRow>>,
    selected: Signal<Option<u64>>,
    compose: Compose,
    langs: Signal<Option<Lang>>,
    at: Position,
) -> impl IntoElement {
    let row = computed(move || {
        rows.with(|r| r.get(at.index()).cloned())
            .unwrap_or_default()
    });
    view()
        .w_full()
        .child(when(
            move || row.with(|r| r.is_first_in_stage && r.stage_no > 1),
            move || {
                view()
                    .w_full()
                    .h(px(28.0))
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(
                        mono("↓")
                            .text_size(px(13.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(accent_blue_text()),
                    )
                    .child(
                        view()
                            .px_2()
                            .py(px(1.5))
                            .rounded(px(4.0))
                            .border_1()
                            .border_color(border_subtle())
                            .bg(bg_card())
                            .child(
                                text(move || {
                                    row.with(|r| {
                                        let st_no = r.stage_no;
                                        match (r.stage_has_hard_dep, ui_lang(langs)) {
                                            (true, Lang::Zh) => {
                                                format!("携带前序阶段返回参数进入阶段 {st_no}")
                                            }
                                            (true, Lang::En) => {
                                                format!("Passes response data into Stage {st_no}")
                                            }
                                            (false, Lang::Zh) => {
                                                format!(
                                                    "等待阶段 {} 响应完成后触发",
                                                    st_no.saturating_sub(1)
                                                )
                                            }
                                            (false, Lang::En) => {
                                                format!(
                                                    "Triggered after Stage {} completed",
                                                    st_no.saturating_sub(1)
                                                )
                                            }
                                        }
                                    })
                                })
                                .text_size(px(11.0))
                                .text_color(fg_muted()),
                            ),
                    )
            },
        ))
        .child(when(
            move || row.with(|r| r.is_first_in_stage),
            move || {
                view()
                    .w_full()
                    .h(px(26.0))
                    .px_3()
                    .rounded_t(px(7.0))
                    .border_t_1()
                    .border_l_1()
                    .border_r_1()
                    .border_b_1()
                    .border_color(border_control())
                    .border_b_color(border_subtle())
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .bg(bg_control())
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                text(move || {
                                    let no = row.with(|r| r.stage_no);
                                    match ui_lang(langs) {
                                        Lang::Zh => format!("阶段 {no}"),
                                        Lang::En => format!("Stage {no}"),
                                    }
                                })
                                .tabular_nums()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(fg_primary()),
                            )
                            .child(
                                view()
                                    .px_1p5()
                                    .py(px(0.5))
                                    .rounded(px(4.0))
                                    .bg(move || {
                                        if row.with(|r| r.stage_bar_count > 1) {
                                            accent_blue_soft()
                                        } else {
                                            bg_card()
                                        }
                                    })
                                    .child(
                                        text(move || {
                                            let count = row.with(|r| r.stage_bar_count);
                                            match (count > 1, ui_lang(langs)) {
                                                (true, Lang::Zh) => format!("并发 ×{count}"),
                                                (true, Lang::En) => {
                                                    format!("Concurrent ×{count}")
                                                }
                                                (false, Lang::Zh) => "串行单步".to_string(),
                                                (false, Lang::En) => "Sequential".to_string(),
                                            }
                                        })
                                        .tabular_nums()
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(
                                            move || {
                                                if row.with(|r| r.stage_bar_count > 1) {
                                                    accent_blue_text()
                                                } else {
                                                    fg_muted()
                                                }
                                            },
                                        ),
                                    ),
                            ),
                    )
                    .child(
                        mono(move || {
                            row.with(|r| {
                                format!("[{}] · {}", r.stage_offset_label, r.stage_span_label)
                            })
                        })
                        .tabular_nums()
                        .text_size(px(11.0))
                        .text_color(fg_muted()),
                    )
            },
        ))
        .child(
            view()
                .w_full()
                .px_3()
                .py_2()
                .gap_1p5()
                .bg(bg_card())
                .border_l_1()
                .border_r_1()
                .border_color(border_control())
                .border_t(move || {
                    if row.with(|r| r.is_first_in_stage) {
                        px(0.0)
                    } else {
                        px(1.0)
                    }
                })
                .border_t_color(border_subtle())
                .border_b(move || {
                    if row.with(|r| r.is_last_in_stage) {
                        px(1.0)
                    } else {
                        px(0.0)
                    }
                })
                .border_b_color(border_control())
                .rounded_b(move || {
                    if row.with(|r| r.is_last_in_stage) {
                        px(7.0)
                    } else {
                        px(0.0)
                    }
                })
                .hover(|style| style.bg(bg_control_hover()))
                .active(|style| style.opacity(0.85))
                .cursor(Cursor::Pointer)
                .on_press(move || {
                    jump_to_seq_flow(row.with(|r| r.bar.flow_id), selected, compose, seq_builder);
                })
                .child(
                    view()
                        .w_full()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            view()
                                .w(px(3.0))
                                .h(px(16.0))
                                .rounded_full()
                                .flex_shrink_0()
                                .bg(move || {
                                    row.with(|r| {
                                        let b = &r.bar;
                                        if b.failed || b.status.is_some_and(|c| c >= 400) {
                                            status_err_fg()
                                        } else if b.status.is_some_and(|c| (300..400).contains(&c))
                                        {
                                            status_warn_fg()
                                        } else if b.status.is_some() {
                                            status_ok_fg()
                                        } else {
                                            accent_blue()
                                        }
                                    })
                                }),
                        )
                        .child(
                            mono(move || row.with(|r| format!("#{}", r.bar.flow_id)))
                                .tabular_nums()
                                .min_w(px(36.0))
                                .flex_shrink_0()
                                .text_size(px(11.5))
                                .text_color(fg_muted()),
                        )
                        .child(
                            view()
                                .min_w(px(44.0))
                                .px(px(5.0))
                                .py_0p5()
                                .rounded(px(4.0))
                                .flex_shrink_0()
                                .items_center()
                                .justify_center()
                                .bg(move || row.with(|r| method_bg(&r.bar.method)))
                                .child(
                                    mono(move || row.with(|r| r.bar.method.clone()))
                                        .text_size(px(10.5))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(move || row.with(|r| method_fg(&r.bar.method))),
                                ),
                        )
                        .child(
                            mono(move || {
                                row.with(|r| {
                                    let b = &r.bar;
                                    if b.host.is_empty() {
                                        b.target.clone()
                                    } else if b.target.starts_with('/') {
                                        format!("{}{}", b.host, b.target)
                                    } else {
                                        format!("{}/{}", b.host, b.target)
                                    }
                                })
                            })
                            .flex_1()
                            .min_w(px(0.0))
                            .number_of_lines(1)
                            .text_overflow(EllipsizeMode::Middle)
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg_primary()),
                        )
                        .child(when(
                            move || row.with(|r| r.bar.on_critical_path),
                            || {
                                view()
                                    .px_1p5()
                                    .py(px(0.5))
                                    .rounded(px(3.5))
                                    .flex_shrink_0()
                                    .bg(status_warn_bg())
                                    .child(
                                        text(tr("★ 关键路径", "★ Critical"))
                                            .text_size(px(11.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(status_warn_fg()),
                                    )
                            },
                        ))
                        .child(
                            mono(move || row.with(|r| list_status(r.bar.status, r.bar.failed)))
                                .tabular_nums()
                                .min_w(px(32.0))
                                .text_right()
                                .flex_shrink_0()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || {
                                    row.with(|r| http_status_fg(r.bar.status, r.bar.failed))
                                }),
                        )
                        .child(
                            mono(move || {
                                row.with(|r| {
                                    let b = &r.bar;
                                    if b.size_label.is_empty() {
                                        format!("[{}] · {}", b.offset_label, b.duration_label)
                                    } else {
                                        format!(
                                            "[{}] · {} · {}",
                                            b.offset_label, b.duration_label, b.size_label
                                        )
                                    }
                                })
                            })
                            .tabular_nums()
                            .flex_shrink_0()
                            .text_size(px(11.0))
                            .text_color(fg_muted()),
                        ),
                )
                .child(when(
                    move || row.with(|r| !r.bar.deps.is_empty() || !r.bar.comment.is_empty()),
                    move || {
                        view()
                            .w_full()
                            .pl_3()
                            .flex_row()
                            .flex_wrap()
                            .items_center()
                            .gap_1p5()
                            .child(seq_dep_badge(row, langs, 0))
                            .child(seq_dep_badge(row, langs, 1))
                            .child(seq_dep_badge(row, langs, 2))
                            .child(seq_dep_badge(row, langs, 3))
                            .child(when(
                                move || row.with(|r| !r.bar.comment.is_empty()),
                                move || {
                                    view()
                                        .px_2()
                                        .py(px(1.5))
                                        .rounded(px(4.0))
                                        .max_w(px(200.0))
                                        .bg(accent_blue_soft())
                                        .child(
                                            text(move || {
                                                row.with(|r| format!("# {}", r.bar.comment))
                                            })
                                            .number_of_lines(1)
                                            .text_overflow(EllipsizeMode::Tail)
                                            .text_size(px(11.0))
                                            .text_color(accent_blue_text()),
                                        )
                                },
                            ))
                    },
                )),
        )
}

pub(crate) const SEQ_WF_ROW_H: f32 = 30.0;

pub(crate) fn seq_wf_left_col_width(viewport_w: f32) -> f32 {
    if viewport_w > 0.0 {
        (viewport_w * 0.38).round().clamp(132.0, 260.0)
    } else {
        240.0
    }
}

pub(crate) fn seq_waterfall_view(
    seq_builder: SeqBuilder,
    selected: Signal<Option<u64>>,
    compose: Compose,
) -> impl IntoElement {
    const WF_HEADER_H: f32 = 28.0;
    const WF_RULER_H: f32 = 22.0;
    let langs = ui_langs();
    let track_w = seq_builder.track_w;
    let viewport_w = seq_builder.viewport_w;

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
            move || seq_builder.diagram.with(|d| d.bars.is_empty()),
            || {
                view()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .child(
                        text(tr("请勾选至少一条流量", "Select at least one flow"))
                            .text_size(px(12.0))
                            .text_color(fg_muted()),
                    )
            },
        ))
        .child(when(
            move || seq_builder.diagram.with(|d| !d.bars.is_empty()),
            move || {
                view()
                    .size_full()
                    .child(
                        view()
                            .h(px(WF_HEADER_H))
                            .px_3()
                            .flex_shrink_0()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .border_b_1()
                            .border_b_color(border_subtle())
                            .bg(bg_card())
                            .child(
                                text(tr(
                                    "关键路径瀑布图（折线连接上下游依赖，垂直重叠代表同时并发）",
                                    "Critical-path waterfall (connectors show dependencies; vertical overlap shows concurrency)",
                                ))
                                .flex_1()
                                .min_w(px(0.0))
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Tail)
                                .text_size(px(11.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg_secondary()),
                            )
                            .child(
                                mono(move || {
                                    let hovered = seq_builder.hover_bar.get();
                                    seq_builder.diagram.with(|d| {
                                        if let Some(idx) = hovered
                                            && let Some(bar) = d.bars.get(idx)
                                        {
                                            let dep_note = bar
                                                .deps
                                                .first()
                                                .map(|dep| {
                                                    let prefix =
                                                        t(ui_lang(langs), " · 依赖 #", " · dep #");
                                                    format!(
                                                        "{prefix}{} ({})",
                                                        dep.from_flow_id, dep.label
                                                    )
                                                })
                                                .unwrap_or_default();
                                            return format!(
                                                "#{} {} {} · [+{:.0}ms .. +{:.0}ms] ({}){dep_note}",
                                                bar.flow_id,
                                                bar.method,
                                                bar.target,
                                                bar.start_ms,
                                                bar.end_ms,
                                                bar.duration_label
                                            );
                                        }
                                        let max_ms = d
                                            .bars
                                            .iter()
                                            .map(|b| b.end_ms)
                                            .fold(0.0_f64, f64::max);
                                        format!("0 ms .. {max_ms:.0} ms")
                                    })
                                })
                                .tabular_nums()
                                .flex_shrink_0()
                                .number_of_lines(1)
                                .text_size(px(11.0))
                                .text_color(accent_blue_text()),
                            ),
                    )
                    .child(
                        view()
                            .h(px(WF_RULER_H))
                            .w_full()
                            .flex_shrink_0()
                            .flex_row()
                            .items_center()
                            .border_b_1()
                            .border_b_color(border_subtle())
                            .bg(bg_control())
                            .child(
                                view()
                                    .w(move || px(seq_wf_left_col_width(viewport_w.get())))
                                    .h_full()
                                    .flex_shrink_0()
                                    .px_2p5()
                                    .flex_row()
                                    .items_center()
                                    .border_r_1()
                                    .border_r_color(border_subtle())
                                    .child(
                                        text(tr("阶段 / 接口", "Stage / Endpoint"))
                                            .number_of_lines(1)
                                            .text_overflow(EllipsizeMode::Tail)
                                            .text_size(px(11.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(fg_muted()),
                                    ),
                            )
                            .child(
                                view()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .h_full()
                                    .px_3()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .on_layout(move |bounds: &Bounds<Points>| {
                                        let w = bounds.size.width.0;
                                        if (w - track_w.get_untracked()).abs() > 1.0 {
                                            track_w.set(w);
                                        }
                                    })
                                    .children([0.0_f64, 0.25, 0.5, 0.75, 1.0].into_iter().map(
                                        move |ratio| {
                                            mono(move || {
                                                let tw = track_w.get();
                                                if tw > 0.0
                                                    && tw < 220.0
                                                    && (ratio == 0.25 || ratio == 0.75)
                                                {
                                                    return String::new();
                                                }
                                                let max_ms = seq_builder.diagram.with(|d| {
                                                    d.bars
                                                        .iter()
                                                        .map(|b| b.end_ms)
                                                        .fold(1.0_f64, f64::max)
                                                });
                                                let ms = max_ms * ratio;
                                                if ms >= 1000.0 {
                                                    format!("+{:.1}s", ms / 1000.0)
                                                } else {
                                                    format!("+{ms:.0}ms")
                                                }
                                            })
                                            .tabular_nums()
                                            .text_size(px(11.0))
                                            .text_color(fg_muted())
                                        },
                                    )),
                            ),
                    )
                    .child(
                        virtual_list(
                            move || seq_builder.diagram.with(|d| d.bars.len()),
                            move |at| seq_waterfall_row(seq_builder, track_w, selected, compose, at),
                        )
                        .keys(move || {
                            seq_builder
                                .diagram
                                .with(|d| d.bars.iter().map(|b| b.flow_id).collect::<Vec<_>>())
                        })
                        .estimated_item_size(px(SEQ_WF_ROW_H))
                        .flex_1()
                        .w_full(),
                    )
            },
        ))
}

pub(crate) fn seq_waterfall_row(
    seq_builder: SeqBuilder,
    track_w: Signal<f32>,
    selected: Signal<Option<u64>>,
    compose: Compose,
    at: Position,
) -> impl IntoElement {
    let viewport_w = seq_builder.viewport_w;
    let bar_item = computed(move || {
        seq_builder
            .diagram
            .with(|d| d.bars.get(at.index()).cloned())
            .unwrap_or_default()
    });
    view()
        .w_full()
        .h(px(SEQ_WF_ROW_H))
        .flex_row()
        .items_center()
        .active(|style| style.opacity(0.85))
        .cursor(Cursor::Pointer)
        .on_press(move || {
            jump_to_seq_flow(bar_item.with(|b| b.flow_id), selected, compose, seq_builder);
        })
        .bg(move || {
            if seq_builder.hover_bar.get() == Some(at.index()) {
                bg_control_hover()
            } else if at.index() % 2 == 1 {
                bg_code()
            } else {
                bg_card()
            }
        })
        .border_t(move || if at.index() > 0 { px(1.0) } else { px(0.0) })
        .border_t_color(border_subtle())
        .on_mouse_move(move |_: &MouseEvent| {
            if seq_builder.hover_bar.get_untracked() != Some(at.index()) {
                seq_builder.hover_bar.set(Some(at.index()));
            }
        })
        .on_mouse_leave(move |_: &MouseEvent| {
            if seq_builder.hover_bar.get() == Some(at.index()) {
                seq_builder.hover_bar.set(None);
            }
        })
        .child(
            view()
                .w(move || px(seq_wf_left_col_width(viewport_w.get())))
                .h_full()
                .px_2p5()
                .gap_1p5()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .border_r_1()
                .border_r_color(border_subtle())
                .child(
                    view()
                        .px_1()
                        .py(px(0.5))
                        .rounded(px(3.0))
                        .flex_shrink_0()
                        .bg(bg_control())
                        .child(
                            mono(move || bar_item.with(|b| format!("S{}", b.stage)))
                                .tabular_nums()
                                .text_size(px(10.5))
                                .font_weight(FontWeight::BOLD)
                                .text_color(accent_blue_text()),
                        ),
                )
                .child(
                    mono(move || bar_item.with(|b| format!("#{}", b.flow_id)))
                        .tabular_nums()
                        .flex_shrink_0()
                        .text_size(px(11.0))
                        .text_color(fg_muted()),
                )
                .child(
                    view()
                        .px(px(4.5))
                        .py(px(1.0))
                        .rounded(px(3.5))
                        .flex_shrink_0()
                        .bg(move || bar_item.with(|b| method_bg(&b.method)))
                        .child(
                            mono(move || bar_item.with(|b| b.method.clone()))
                                .text_size(px(10.5))
                                .font_weight(FontWeight::BOLD)
                                .text_color(move || bar_item.with(|b| method_fg(&b.method))),
                        ),
                )
                .child(
                    mono(move || bar_item.with(|b| b.target.clone()))
                        .flex_1()
                        .min_w(px(0.0))
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Middle)
                        .text_size(px(11.0))
                        .text_color(fg_primary()),
                )
                .child(
                    mono(move || bar_item.with(|b| list_status(b.status, b.failed)))
                        .tabular_nums()
                        .flex_shrink_0()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || bar_item.with(|b| http_status_fg(b.status, b.failed))),
                ),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .relative()
                .overflow_hidden()
                .child(
                    canvas(move || {
                        let row_i = at.index();
                        let w = (track_w.get() - 24.0).max(48.0);
                        let pad_x = 12.0_f32;
                        seq_builder.diagram.with(|d| {
                            let bars = &d.bars;
                            let max_ms = bars
                                .iter()
                                .map(|b| b.end_ms)
                                .fold(1.0_f64, f64::max)
                                as f32;
                            let y_mid = SEQ_WF_ROW_H * 0.5;
                            let mut p = Painter::new();

                            // 1. Vertical time grid lines for this row
                            for ratio in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
                                let gx = pad_x + ratio * w;
                                p = p
                                    .line_width(1.0)
                                    .stroke_color(color(border_subtle()))
                                    .stroke_svg(&format!("M{gx:.1} 0 L{gx:.1} {SEQ_WF_ROW_H:.1}"));
                            }

                            // 2. Cross-row dependency connector segments passing through `row_i`
                            for (target_idx, target_bar) in bars.iter().enumerate() {
                                for dep in &target_bar.deps {
                                    let from_idx = dep.from_index;
                                    if from_idx >= target_idx
                                        || row_i < from_idx
                                        || row_i > target_idx
                                    {
                                        continue;
                                    }
                                    let Some(parent) = bars.get(from_idx) else {
                                        continue;
                                    };
                                    let x_parent_end =
                                        pad_x + (parent.end_ms as f32 / max_ms) * w;
                                    let x_target_start =
                                        pad_x + (target_bar.start_ms as f32 / max_ms) * w;
                                    let elbow_x =
                                        x_parent_end.min(x_target_start - 2.0).max(pad_x);
                                    let conn_color = if dep.kind.is_hard() {
                                        color(accent_blue())
                                    } else {
                                        color(status_warn_fg())
                                    };
                                    let lw = if dep.kind.is_hard() { 1.5 } else { 1.1 };
                                    if row_i == from_idx {
                                        p = p.line_width(lw).stroke_color(conn_color).stroke_svg(
                                            &format!(
                                                "M{x_parent_end:.1} {y_mid:.1} L{elbow_x:.1} {y_mid:.1} L{elbow_x:.1} {SEQ_WF_ROW_H:.1}"
                                            ),
                                        );
                                    } else if row_i < target_idx {
                                        p = p.line_width(lw).stroke_color(conn_color).stroke_svg(
                                            &format!(
                                                "M{elbow_x:.1} 0 L{elbow_x:.1} {SEQ_WF_ROW_H:.1}"
                                            ),
                                        );
                                    } else {
                                        p = p.line_width(lw).stroke_color(conn_color).stroke_svg(
                                            &format!(
                                                "M{elbow_x:.1} 0 L{elbow_x:.1} {y_mid:.1} L{x_target_start:.1} {y_mid:.1}"
                                            ),
                                        );
                                        let ax = (x_target_start - 4.2).max(pad_x);
                                        let ay1 = y_mid - 2.6;
                                        let ay2 = y_mid + 2.6;
                                        p = p.line_width(1.4).stroke_color(conn_color).stroke_svg(
                                            &format!(
                                                "M{ax:.1} {ay1:.1} L{x_target_start:.1} {y_mid:.1} L{ax:.1} {ay2:.1}"
                                            ),
                                        );
                                    }
                                }
                            }

                            // 3. Horizontal Span Bar for `row_i`
                            if let Some(bar) = bars.get(row_i) {
                                let x1 = pad_x + (bar.start_ms as f32 / max_ms) * w;
                                let x2 = pad_x + (bar.end_ms as f32 / max_ms) * w;
                                let bw = (x2 - x1).max(5.0);
                                let y_top = 7.0_f32;
                                let bh = SEQ_WF_ROW_H - 14.0;
                                let fill = color(
                                    if bar.failed || bar.status.is_some_and(|c| c >= 400) {
                                        status_err_fg()
                                    } else if bar.status.is_some_and(|c| (300..400).contains(&c)) {
                                        status_warn_fg()
                                    } else if bar.status.is_some() {
                                        status_ok_fg()
                                    } else {
                                        accent_blue()
                                    },
                                );
                                let rect_svg = format!(
                                    "M{x1:.1} {y_top:.1} H{:.1} V{:.1} H{x1:.1} Z",
                                    x1 + bw,
                                    y_top + bh
                                );
                                p = p.fill_color(fill).fill_svg(&rect_svg);
                                if bar.on_critical_path {
                                    p = p
                                        .line_width(1.5)
                                        .stroke_color(color(status_warn_fg()))
                                        .stroke_svg(&rect_svg);
                                }
                            }
                            p
                        })
                    })
                    .size_full(),
                )
                .child(
                    view()
                        .absolute()
                        .left(move || {
                            let w = (track_w.get() - 24.0).max(48.0);
                            let max_ms = seq_builder.diagram.with(|d| {
                                d.bars
                                    .iter()
                                    .map(|b| b.end_ms)
                                    .fold(1.0_f64, f64::max)
                            }) as f32;
                            bar_item.with(|b| {
                                let x1 = 12.0 + (b.start_ms as f32 / max_ms) * w;
                                let x2 = 12.0 + (b.end_ms as f32 / max_ms) * w;
                                let after = x1.max(x2) + 6.0;
                                if after + 104.0 < w + 12.0 {
                                    px(after)
                                } else {
                                    px((x1 + 4.0).min((w - 72.0).max(14.0)).max(14.0))
                                }
                            })
                        })
                        .top(px(5.0))
                        .px_1p5()
                        .py(px(0.5))
                        .rounded(px(3.0))
                        .bg(bg_card())
                        .child(
                            mono(move || {
                                bar_item.with(|b| {
                                    if let Some(dep) = b.deps.first() {
                                        if dep.kind.is_hard() {
                                            format!("{} · dep#{}", b.duration_label, dep.from_flow_id)
                                        } else {
                                            format!(
                                                "{} · after#{}",
                                                b.duration_label, dep.from_flow_id
                                            )
                                        }
                                    } else {
                                        b.duration_label.clone()
                                    }
                                })
                            })
                            .tabular_nums()
                            .number_of_lines(1)
                            .text_size(px(11.0))
                            .text_color(fg_secondary()),
                        ),
                ),
        )
}
