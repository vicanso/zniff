#[allow(unused_imports)]
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Empty,
    Gone,
    Ready,
}

#[derive(Clone, PartialEq)]
pub(crate) struct HeaderLine {
    pub(crate) key: u32,
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) hint: Option<String>,
}

#[derive(Clone, Copy)]
pub(crate) struct Shown {
    pub(crate) phase: Signal<Phase>,
    pub(crate) method: Signal<String>,
    pub(crate) url: Signal<String>,
    pub(crate) protocol: Signal<String>,
    pub(crate) rules: Signal<String>,
    pub(crate) rule_hits: Signal<Vec<session::RuleHit>>,
    pub(crate) gql_badge: Signal<String>,
    pub(crate) status: Signal<Option<u16>>,
    pub(crate) error: Signal<String>,
    pub(crate) request_headers: Signal<Vec<HeaderLine>>,
    pub(crate) response_headers: Signal<Vec<HeaderLine>>,
    pub(crate) request_body: Signal<String>,
    pub(crate) response_body: Signal<String>,
    pub(crate) request_body_bytes: Signal<usize>,
    pub(crate) response_body_bytes: Signal<usize>,
    pub(crate) request_hex: Signal<Vec<u8>>,
    pub(crate) response_hex: Signal<Vec<u8>>,
    /// Detail-pane viewport width. The hex line uses it after the pane is shown.
    pub(crate) pane_width: Signal<f32>,
    pub(crate) request_language: Signal<String>,
    pub(crate) response_language: Signal<String>,
    pub(crate) request_doc: Signal<Option<nsh_base::text_view::Document>>,
    pub(crate) response_doc: Signal<Option<nsh_base::text_view::Document>>,
    pub(crate) req_doc_preview: Signal<bool>,
    pub(crate) res_doc_preview: Signal<bool>,
    pub(crate) request_json: Signal<Option<JsonBody>>,
    pub(crate) response_json: Signal<Option<JsonBody>>,
    pub(crate) req_json_tree: Signal<bool>,
    pub(crate) res_json_tree: Signal<bool>,
    pub(crate) request_form: Signal<Vec<HeaderLine>>,
    pub(crate) request_note: Signal<String>,
    pub(crate) response_note: Signal<String>,
    pub(crate) metrics: Signal<String>,
    pub(crate) timing_bar: Signal<present::TimingBreakdown>,
    pub(crate) query: Signal<Vec<HeaderLine>>,
    pub(crate) preview: Signal<String>,
    pub(crate) preview_w: Signal<f32>,
    pub(crate) preview_h: Signal<f32>,
    pub(crate) image_caption: Signal<String>,
    pub(crate) pretty: Signal<bool>,
    pub(crate) default_json_tree: Signal<bool>,
    pub(crate) messages: Signal<Vec<HeaderLine>>,
    pub(crate) llm_stream: Signal<Option<llm_stream::LlmStreamSummary>>,
    pub(crate) msg_llm_view: Signal<bool>,
    pub(crate) diff: Signal<String>,
    pub(crate) diff_old: Signal<String>,
    pub(crate) diff_new: Signal<String>,
    pub(crate) diff_baseline_label: Signal<String>,
    pub(crate) diff_scope: Signal<u8>,
    pub(crate) pinned: Signal<bool>,
    pub(crate) is_marker: Signal<bool>,
    pub(crate) comment: Signal<String>,
    pub(crate) editing_comment: Signal<bool>,
    pub(crate) repeating: Signal<bool>,
    pub(crate) repeat_count: Signal<String>,
    pub(crate) repeat_interval: Signal<String>,
    pub(crate) repeat_concurrency: Signal<String>,
    pub(crate) req_search: Signal<String>,
    pub(crate) res_search: Signal<String>,
    pub(crate) msg_search: Signal<String>,
    pub(crate) msg_dir: Signal<u8>,
    pub(crate) msg_kind: Signal<u8>,
    /// Eight hex characters of the share that pushed the selected row. Empty locally.
    pub(crate) relay: Signal<String>,
    pub(crate) request_offset: Signal<usize>,
    pub(crate) response_offset: Signal<usize>,
    pub(crate) request_window: Signal<TextWindow>,
    pub(crate) response_window: Signal<TextWindow>,
    pub(crate) req_headers_loading: Signal<bool>,
    pub(crate) res_headers_loading: Signal<bool>,
    pub(crate) req_body_loading: Signal<bool>,
    pub(crate) res_body_loading: Signal<bool>,
    pub(crate) messages_loading: Signal<bool>,
    pub(crate) diff_loading: Signal<bool>,
}

#[derive(Clone, Copy)]
pub(crate) enum CopyPart {
    Url,
    Curl,
    Request,
    Response,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn detail_pane(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
    compose: Compose,
    seq_builder: SeqBuilder,
    rows: Signal<Vec<Arc<RowLine>>>,
    list_filter: Signal<String>,
    effective_filter: Signal<String>,
    baseline: Signal<Option<u64>>,
    folded: Signal<u8>,
    pane_px: Rc<Cell<f32>>,
    filtered_stats: Signal<session::FilteredStats>,
    detail_scroll: ElementRef,
    layout_mode: Signal<config::LayoutMode>,
    onboarding: OnboardingProps,
) -> impl IntoElement {
    let ready_controls = controls.clone();
    let ready_profiles = onboarding.profiles;
    let standalone_summary = use_context::<StandaloneSummaryContext>().map(|ctx| ctx.0);
    let _ = layout_mode;
    scroll()
        .element_ref(&detail_scroll)
        .flex_1()
        .min_w(px(0.0))
        .bg(bg_canvas())
        .on_layout(move |bounds: &Bounds<Points>| {
            let width = bounds.size.width.0;
            pane_px.set(width);
            if width > 0.0 && (width - shown.pane_width.get_untracked()).abs() > 1.0 {
                shown.pane_width.set(width);
            }
        })
        .child(
            when(
                move || shown.phase.get() == Phase::Ready,
                move || {
                    let marker_controls = ready_controls.clone();
                    let flow_controls = ready_controls.clone();
                    when(
                        move || shown.is_marker.get(),
                        move || {
                            marker_detail_card(
                                shown,
                                marker_controls.clone(),
                                selected,
                                list_filter,
                            )
                        },
                    )
                    .otherwise(move || {
                        ready_detail(
                            shown,
                            flow_controls.clone(),
                            selected,
                            paused,
                            compose,
                            list_filter,
                            baseline,
                            folded,
                            ready_profiles,
                        )
                    })
                },
            )
            .otherwise({
                let onboarding_compose = onboarding.clone();
                move || {
                    let compose_controls = controls.clone();
                    let seq_controls = controls.clone();
                    let summary_controls = controls.clone();
                    let onboarding_seq = onboarding_compose.clone();
                    when(
                        move || compose.composing.get(),
                        move || {
                            uncull_nested_scroll(view())
                                .pl_3p5()
                                .pr_5()
                                .py_3p5()
                                .gap_3p5()
                                .child(compose_box(
                                    compose,
                                    compose_controls.clone(),
                                    selected,
                                    paused,
                                ))
                        },
                    )
                    .otherwise(move || {
                        let seq_controls = seq_controls.clone();
                        let summary_controls = summary_controls.clone();
                        let onboarding_redact = onboarding_seq.clone();
                        when(
                            move || seq_builder.open.get(),
                            move || {
                                uncull_nested_scroll(view())
                                    .pl_3p5()
                                    .pr_5()
                                    .py_3p5()
                                    .gap_3p5()
                                    .child(sequence_diagram_panel(
                                        seq_builder,
                                        seq_controls.clone(),
                                        selected,
                                        compose,
                                    ))
                            },
                        )
                        .otherwise(move || {
                            let redact_controls = summary_controls.clone();
                            let summary_controls = summary_controls.clone();
                            let onboarding_empty = onboarding_redact.clone();
                            when(
                                move || seq_builder.redact_open.get(),
                                move || {
                                    uncull_nested_scroll(view())
                                        .pl_3p5()
                                        .pr_5()
                                        .py_3p5()
                                        .gap_3p5()
                                        .child(redact_preview_panel(
                                            seq_builder,
                                            redact_controls.clone(),
                                            rows,
                                            effective_filter,
                                        ))
                                },
                            )
                            .otherwise(move || {
                                let summary_controls = summary_controls.clone();
                                let onboarding_now = onboarding_empty.clone();
                                let onboarding_flow_total = onboarding_empty.flow_total;
                            uncull_nested_scroll(view())
                                .flex_grow(1.0)
                                .flex_shrink_0()
                                .pl_3p5()
                                .pr_5()
                                .py_3p5()
                                .gap_3p5()
                                .child(when(
                                    move || {
                                        !standalone_summary.is_some_and(|s| s.get())
                                            && filtered_stats.with(|s| s.total_count > 0)
                                    },
                                    move || {
                                        let controls = summary_controls.clone();
                                        filtered_summary_card(filtered_stats, selected, move || {
                                            begin_sequence_diagram(
                                                &controls,
                                                rows,
                                                seq_builder,
                                                compose,
                                                selected,
                                                shown.phase,
                                            );
                                        })
                                    },
                                ))
                                .child(
                                    when(
                                        move || {
                                            should_show_onboarding(
                                                shown.phase.get(),
                                                onboarding_flow_total.get(),
                                            )
                                        },
                                        move || {
                                            view()
                                                .w_full()
                                                .flex_grow(1.0)
                                                .flex_shrink_0()
                                                .flex_col()
                                                .items_center()
                                                .px_2()
                                                .child(
                                                    view()
                                                        .min_h(px(12.0))
                                                        .max_h(px(64.0))
                                                        .flex_grow(0.35),
                                                )
                                                .child(onboarding_card(
                                                    onboarding_now.clone(),
                                                    shown.pane_width,
                                                ))
                                                .child(view().min_h(px(16.0)).flex_grow(1.0))
                                        },
                                    )
                                    .otherwise(move || {
                                        view()
                                            .flex_grow(1.0)
                                            .flex_shrink_0()
                                            .items_center()
                                            .justify_center()
                                            .gap_1p5()
                                            .px_6()
                                            .py_8()
                                            .child(glyph_px(
                                                28.0,
                                                || Glyph::List,
                                                || color(fg_muted()),
                                            ))
                                            .child(
                                                text({
                                                    let langs = ui_langs();
                                                    move || -> SharedString {
                                                        let lang = ui_lang(langs);
                                                        match shown.phase.get() {
                                                            Phase::Gone => t(
                                                                lang,
                                                                "这条记录已经不在了",
                                                                "That flow is gone",
                                                            )
                                                            .into(),
                                                            _ => t(
                                                                lang,
                                                                "还没有选择",
                                                                "Nothing selected",
                                                            )
                                                            .into(),
                                                        }
                                                    }
                                                })
                                                .text_center()
                                                .text_size(px(TEXT_MD))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(fg_secondary()),
                                            )
                                            .child(
                                                text({
                                                    let langs = ui_langs();
                                                    move || -> SharedString {
                                                        let lang = ui_lang(langs);
                                                        match shown.phase.get() {
                                                            Phase::Gone => t(
                                                                lang,
                                                                "它因为内存上限被丢掉了。",
                                                                "It was dropped because the session hit its memory cap.",
                                                            )
                                                            .into(),
                                                            _ => t(
                                                                lang,
                                                                "从左边选一条流量，查看请求头、响应头和 body。",
                                                                "Pick a flow on the left to see headers and the body.",
                                                            )
                                                            .into(),
                                                        }
                                                    }
                                                })
                                                .text_center()
                                                .text_size(px(TEXT_SM))
                                                .text_color(fg_muted()),
                                            )
                                            .child(
                                                button()
                                                    .mt_1p5()
                                                    .h(px(CONTROL_H))
                                                    .px_3()
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
                                                    .flex_row()
                                                    .items_center()
                                                    .gap_1p5()
                                                    .on_press(move |_: &()| {
                                                        begin_new_request(compose, selected)
                                                    })
                                                    .child(glyph_px(
                                                        12.0,
                                                        || Glyph::Prompt,
                                                        || color(fg_secondary()),
                                                    ))
                                                    .child(
                                                        text(tr("构造请求", "New request"))
                                                            .number_of_lines(1)
                                                            .text_size(px(TEXT_SM))
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .text_color(fg_primary()),
                                                    ),
                                            )
                                    }),
                                )
                            })
                        })
                    })
                }
            }),
        )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn ready_detail(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
    compose: Compose,
    filter: Signal<String>,
    baseline: Signal<Option<u64>>,
    folded: Signal<u8>,
    profiles: ProfileSignals,
) -> impl IntoElement {
    let side_by_side = move || profiles.layout_mode.get() == config::LayoutMode::TopBottom;
    let req_controls_split = controls.clone();
    let res_controls_split = controls.clone();
    let req_controls_stack = controls.clone();
    let res_controls_stack = controls.clone();
    uncull_nested_scroll(view())
        .pl_3p5()
        .pr_5()
        .py_3p5()
        .gap_3p5()
        .child(summary(
            shown,
            controls.clone(),
            selected,
            paused,
            compose,
            filter,
            baseline,
            profiles,
        ))
        .child(when(move || compose.composing.get(), {
            let controls = controls.clone();
            move || compose_box(compose, controls.clone(), selected, paused)
        }))
        .child(when(move || compose.inline_mock.open.get(), {
            let controls = controls.clone();
            move || inline_mock_box(compose, controls.clone(), profiles)
        }))
        .child(
            when(side_by_side, move || {
                view()
                    .w_full()
                    .flex_row()
                    .items_start()
                    .gap_3p5()
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .gap_3p5()
                            .child(request_cards(
                                shown,
                                req_controls_split.clone(),
                                selected,
                                baseline,
                                folded,
                            )),
                    )
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .gap_3p5()
                            .child(response_cards(
                                shown,
                                res_controls_split.clone(),
                                selected,
                                baseline,
                                folded,
                            )),
                    )
            })
            .otherwise(move || {
                view()
                    .w_full()
                    .gap_3p5()
                    .child(response_cards(
                        shown,
                        res_controls_stack.clone(),
                        selected,
                        baseline,
                        folded,
                    ))
                    .child(request_cards(
                        shown,
                        req_controls_stack.clone(),
                        selected,
                        baseline,
                        folded,
                    ))
            }),
        )
        .child(when(
            move || shown.diff_loading.get() || !shown.diff.get().is_empty(),
            {
                let diff_controls = controls.clone();
                move || diff_card(shown, diff_controls.clone(), selected, baseline, folded)
            },
        ))
        .child(when(
            move || {
                shown.messages_loading.get()
                    || !shown.messages.with(Vec::is_empty)
                    || shown.llm_stream.with(Option::is_some)
            },
            move || messages_section(shown, controls.clone(), folded),
        ))
}

fn response_cards(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    folded: Signal<u8>,
) -> impl IntoElement {
    let no_form = signal(Vec::<HeaderLine>::new());
    view()
        .w_full()
        .gap_3p5()
        .child(when(
            move || {
                shown.res_body_loading.get()
                    || !shown.response_body.get().is_empty()
                    || !shown.response_note.get().is_empty()
                    || (shown.pretty.get() && !shown.preview.get().is_empty())
                    || shown.response_json.with(Option::is_some)
            },
            {
                let controls = controls.clone();
                move || {
                    body_section(
                        "响应正文",
                        "Response body",
                        shown.response_body,
                        shown.response_hex,
                        shown.response_note,
                        shown.response_language,
                        shown.response_doc,
                        no_form,
                        shown.res_search,
                        true,
                        shown,
                        controls.clone(),
                        selected,
                        baseline,
                        folded,
                        FOLD_RES_BODY,
                        shown.res_body_loading,
                    )
                }
            },
        ))
        .child(header_section(HeaderSectionProps {
            zh: "响应头",
            en: "Response headers",
            lines: shown.response_headers,
            loading: shown.res_headers_loading,
            pane_width: shown.pane_width,
            folded,
            bit: FOLD_RES_HEADERS,
            controls: controls.clone(),
        }))
}

fn request_cards(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    folded: Signal<u8>,
) -> impl IntoElement {
    let no_loading = signal(false);
    view()
        .w_full()
        .gap_3p5()
        .child(when(
            move || {
                shown.req_body_loading.get()
                    || !shown.request_body.get().is_empty()
                    || !shown.request_note.get().is_empty()
                    || !shown.request_form.with(Vec::is_empty)
                    || shown.request_json.with(Option::is_some)
            },
            {
                let controls = controls.clone();
                move || {
                    body_section(
                        "请求正文",
                        "Request body",
                        shown.request_body,
                        shown.request_hex,
                        shown.request_note,
                        shown.request_language,
                        shown.request_doc,
                        shown.request_form,
                        shown.req_search,
                        false,
                        shown,
                        controls.clone(),
                        selected,
                        baseline,
                        folded,
                        FOLD_REQ_BODY,
                        shown.req_body_loading,
                    )
                }
            },
        ))
        .child(when(move || !shown.query.with(Vec::is_empty), {
            let controls = controls.clone();
            move || {
                header_section(HeaderSectionProps {
                    zh: "查询参数",
                    en: "Query",
                    lines: shown.query,
                    loading: no_loading,
                    pane_width: shown.pane_width,
                    folded,
                    bit: FOLD_QUERY,
                    controls: controls.clone(),
                })
            }
        }))
        .child(header_section(HeaderSectionProps {
            zh: "请求头",
            en: "Request headers",
            lines: shown.request_headers,
            loading: shown.req_headers_loading,
            pane_width: shown.pane_width,
            folded,
            bit: FOLD_REQ_HEADERS,
            controls: controls.clone(),
        }))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn summary(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
    compose: Compose,
    filter: Signal<String>,
    baseline: Signal<Option<u64>>,
    profiles: ProfileSignals,
) -> impl IntoElement {
    let is_compact = move || profiles.layout_mode.get() == config::LayoutMode::TopBottom;
    let compact_controls = controls.clone();
    let full_controls = controls.clone();
    view()
        .px_3p5()
        .py(move || if is_compact() { px(9.0) } else { px(14.0) })
        .gap(move || if is_compact() { px(7.0) } else { px(10.0) })
        .rounded(px(RADIUS_LG))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .child(
            view()
                .w_full()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(
                    view()
                        .px_2()
                        .py_0p5()
                        .rounded(px(RADIUS_SM))
                        .bg(move || method_bg(&shown.method.get()))
                        .child(
                            mono(move || shown.method.get())
                                .text_size(px(TEXT_XS))
                                .font_weight(FontWeight::BOLD)
                                .text_color(move || method_fg(&shown.method.get())),
                        ),
                )
                .child(
                    view()
                        .px_2()
                        .py_0p5()
                        .rounded(px(RADIUS_SM))
                        .bg(move || {
                            if shown.status.get().is_none() && is_tunnel_notice(&shown.error.get())
                            {
                                bg_control()
                            } else {
                                http_status_bg(
                                    shown.status.get(),
                                    !shown.error.get().is_empty() && shown.status.get().is_none(),
                                )
                            }
                        })
                        .child(
                            mono({
                                let langs = ui_langs();
                                move || {
                                    let err = shown.error.get();
                                    match shown.status.get() {
                                        Some(code) => code.to_string(),
                                        None if !err.is_empty() && is_tunnel_notice(&err) => {
                                            t(ui_lang(langs), "隧道", "TUNNEL").into()
                                        }
                                        None if !err.is_empty() => "ERR".to_string(),
                                        None => t(ui_lang(langs), "等待响应", "Waiting").into(),
                                    }
                                }
                            })
                            .tabular_nums()
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || {
                                if shown.status.get().is_none()
                                    && is_tunnel_notice(&shown.error.get())
                                {
                                    fg_secondary()
                                } else {
                                    http_status_fg(
                                        shown.status.get(),
                                        !shown.error.get().is_empty()
                                            && shown.status.get().is_none(),
                                    )
                                }
                            }),
                        ),
                )
                .child(
                    view()
                        .px_2()
                        .py_0p5()
                        .rounded(px(RADIUS_SM))
                        .bg(bg_control())
                        .child(
                            mono(move || shown.protocol.get())
                                .text_size(px(TEXT_XS))
                                .text_color(fg_muted()),
                        ),
                )
                .child(detail_rule_badges(shown, profiles, controls.clone()))
                .child(when(
                    move || !shown.comment.get().is_empty() && !shown.editing_comment.get(),
                    move || {
                        button()
                            .max_w(px(180.0))
                            .overflow_hidden()
                            .px_2()
                            .py_0p5()
                            .rounded(px(RADIUS_SM))
                            .border_1()
                            .border_color(accent_blue_border())
                            .bg(accent_blue_soft())
                            .hover(|style| style.bg(accent_blue_soft_hover()))
                            .active(|style| style.bg(bg_control_active()))
                            .transition_colors()
                            .duration(MOTION_INSTANT_MS)
                            .ease_out()
                            .focus_visible(apply_focus_ring)
                            .on_press(move |_: &()| {
                                shown.repeating.set(false);
                                shown.editing_comment.set(true);
                            })
                            .child(
                                text(move || format!("# {}", shown.comment.get()))
                                    .number_of_lines(1)
                                    .text_overflow(EllipsizeMode::Tail)
                                    .text_size(px(TEXT_XS))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(accent_blue_text()),
                            )
                    },
                ))
                .child(view().flex_1())
                .child({
                    let diagram_ctx = use_context::<SeqBuilder>();
                    let diagram_rows = use_context::<DiagramResumeRows>();
                    let resume_controls = controls.clone();
                    when(
                        move || {
                            diagram_ctx.is_some_and(|sb| {
                                !sb.open.get() && !sb.resume_ids.with(Vec::is_empty)
                            }) && diagram_rows.is_some()
                        },
                        move || {
                            let sb = diagram_ctx.expect("seq builder context");
                            let rows_ctx = diagram_rows.expect("diagram rows context");
                            let c = resume_controls.clone();
                            let langs = ui_langs();
                            detail_button(
                                move || -> SharedString {
                                    let count = sb.resume_ids.with(Vec::len);
                                    match ui_lang(langs) {
                                        Lang::Zh => format!("返回图表（{count}）").into(),
                                        Lang::En => format!("Diagram ({count})").into(),
                                    }
                                },
                                move || {
                                    resume_sequence_diagram(
                                        &c,
                                        rows_ctx.0,
                                        sb,
                                        compose,
                                        selected,
                                        shown.phase,
                                    );
                                },
                            )
                        },
                    )
                })
                .child(summary_primary_actions(shown, controls.clone(), selected))
                .child(inline_mock_trigger_button(
                    shown,
                    controls.clone(),
                    selected,
                    compose,
                ))
                .child(detail_actions(
                    shown,
                    controls.clone(),
                    selected,
                    paused,
                    compose,
                    filter,
                    profiles,
                    baseline,
                )),
        )
        .child(when(
            move || !shown.relay.get().is_empty(),
            move || {
                let langs = ui_langs();
                text(move || {
                    let short = shown.relay.get();
                    format!(
                        "{} {} {short}",
                        t(ui_lang(langs), "来源", "From"),
                        t(ui_lang(langs), "中继", "relay")
                    )
                })
                .number_of_lines(1)
                .text_size(px(TEXT_SM))
                .text_color(fg_secondary())
            },
        ))
        .child(
            when(is_compact, move || {
                view()
                    .w_full()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_2p5()
                    .child(view().flex_1().min_w(px(220.0)).child(summary_url_box(
                        shown,
                        compact_controls.clone(),
                        selected,
                        true,
                    )))
                    .child(summary_metrics_text(shown))
            })
            .otherwise(move || {
                view()
                    .w_full()
                    .gap_2p5()
                    .child(summary_url_box(
                        shown,
                        full_controls.clone(),
                        selected,
                        false,
                    ))
                    .child(summary_metrics_text(shown))
            }),
        )
        .child(when(move || shown.repeating.get(), {
            let controls = controls.clone();
            move || repeat_inline_bar(shown, controls.clone(), selected)
        }))
        .child(when(move || shown.editing_comment.get(), {
            let controls = controls.clone();
            move || {
                let save_controls = controls.clone();
                let submit_controls = controls.clone();
                let clear_controls = controls.clone();
                let draft = signal(shown.comment.get_untracked());
                let confirm_del = signal(false);
                let del_job = stored(None::<Spawned<()>>);
                let has_saved_comment = move || !shown.comment.with(|c| c.trim().is_empty());
                let langs = ui_langs();
                view()
                    .flex_row()
                    .items_center()
                    .gap_1p5()
                    .child(
                        view().flex_1().min_w(px(0.0)).child(
                            compose_input_on(
                                draft,
                                tr("添加备注…", "Add a note…"),
                                26.0,
                                false,
                                move |text| {
                                    draft.set(text.to_string());
                                    confirm_del.set(false);
                                },
                            )
                            .on_submit(move |text: &str| {
                                draft.set(text.to_string());
                                shown.comment.set(text.to_string());
                                save_flow_comment(&submit_controls, selected.get(), shown);
                            }),
                        ),
                    )
                    .child(detail_button(tr("保存", "Save"), move || {
                        shown.comment.set(draft.get_untracked());
                        save_flow_comment(&save_controls, selected.get(), shown);
                    }))
                    .child(detail_button(tr("取消", "Cancel"), move || {
                        del_job.replace(None);
                        shown.editing_comment.set(false);
                    }))
                    .child(when(has_saved_comment, move || {
                        let clear_controls = clear_controls.clone();
                        detail_button(
                            move || -> SharedString {
                                if confirm_del.get() {
                                    t(ui_lang(langs), "确认删除？", "Confirm?").into()
                                } else {
                                    t(ui_lang(langs), "删除", "Delete").into()
                                }
                            },
                            move || {
                                if confirm_del.get_untracked() {
                                    del_job.replace(None);
                                    confirm_del.set(false);
                                    shown.comment.set(String::new());
                                    save_flow_comment(&clear_controls, selected.get(), shown);
                                } else {
                                    confirm_del.set(true);
                                    del_job.replace(Some(set_timeout(
                                        Duration::from_millis(3500),
                                        move || confirm_del.set(false),
                                    )));
                                }
                            },
                        )
                    }))
            }
        }))
        .child(when(
            move || shown.timing_bar.with(present::TimingBreakdown::has_content),
            move || summary_timing_bar(shown.timing_bar),
        ))
        .child(error_summary_banner(shown, controls, profiles))
}

fn summary_url_box(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    compact: bool,
) -> impl IntoElement {
    let hovered = signal(false);
    let url_text = mono(move || shown.url.get())
        .selectable(true)
        .style_with(move |s| {
            if compact {
                s.number_of_lines(1).text_overflow(EllipsizeMode::Middle)
            } else {
                s
            }
        })
        .text_size(move || scaled_px(TEXT_SM))
        .text_color(fg_primary());
    let url_slot = if compact {
        view()
            .flex_1()
            .min_w(px(0.0))
            .tooltip(
                tooltip().child(ui_tooltip_surface(
                    mono(move || shown.url.get())
                        .text_size(px(TEXT_XS))
                        .text_color(fg_primary()),
                )),
            )
            .child(url_text)
    } else {
        view().flex_1().min_w(px(0.0)).child(url_text)
    };
    view()
        .w_full()
        .overflow_hidden()
        .px_2p5()
        .py(if compact { px(5.0) } else { px(8.0) })
        .rounded(px(RADIUS_MD))
        .bg(bg_code())
        .border_1()
        .border_color(border_subtle())
        .flex_row()
        .items_center()
        .gap_1p5()
        .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
        .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
        .on_press_up(dismiss_keyboard)
        .child(url_slot)
        .child(
            view()
                .w(px(ControlSize::Xs.height()))
                .h(px(ControlSize::Xs.height()))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .opacity(move || {
                    if compact || hovered.get() {
                        1.0_f32
                    } else {
                        0.0_f32
                    }
                })
                .transition_opacity()
                .duration(move || motion_ms(MOTION_INSTANT_MS))
                .ease_out()
                .child({
                    let copy_controls = controls.clone();
                    row_copy_button(
                        "复制 URL",
                        "Copy URL",
                        "复制完整 URL",
                        "Copy full URL",
                        move || {
                            copy_part(&copy_controls, selected.get(), CopyPart::Url);
                        },
                    )
                }),
        )
}

fn summary_metrics_text(shown: Shown) -> impl IntoElement {
    mono(move || shown.metrics.get())
        .tabular_nums()
        .selectable(true)
        .on_press_up(dismiss_keyboard)
        .number_of_lines(1)
        .text_overflow(EllipsizeMode::Tail)
        .text_size(move || scaled_px(TEXT_XS))
        .text_color(fg_muted())
}
