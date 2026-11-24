#[allow(unused_imports)]
use super::*;

pub(crate) fn llm_stream_panel(
    summary: Signal<Option<llm_stream::LlmStreamSummary>>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let thinking_open = signal(true);
    let content_controls = controls.clone();
    let tools_controls = controls.clone();
    view()
        .w_full()
        .p_3p5()
        .gap_2p5()
        .flex_col()
        .child(llm_meta_strip(summary))
        .child(when(
            move || summary.with(|opt| opt.as_ref().is_some_and(|s| !s.thinking.trim().is_empty())),
            move || llm_thinking_card(summary, thinking_open, controls.clone()),
        ))
        .child(llm_output_card(summary, content_controls))
        .child(when(
            move || summary.with(|opt| opt.as_ref().is_some_and(|s| !s.tool_calls.is_empty())),
            move || llm_tool_calls_card(summary, tools_controls.clone()),
        ))
}

fn llm_meta_strip(summary: Signal<Option<llm_stream::LlmStreamSummary>>) -> impl IntoElement {
    view()
        .w_full()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_1p5()
        .child(
            view()
                .h(px(22.0))
                .px_2()
                .rounded(px(RADIUS_SM))
                .border_1()
                .border_color(accent_blue_border())
                .bg(accent_blue_soft())
                .flex_row()
                .items_center()
                .child(
                    text(move || -> SharedString {
                        summary
                            .with(|opt| opt.as_ref().map(|s| s.provider.label()).unwrap_or("LLM"))
                            .into()
                    })
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(accent_blue_text()),
                ),
        )
        .child(when(
            move || summary.with(|opt| opt.as_ref().and_then(|s| s.model.as_ref()).is_some()),
            move || {
                llm_badge(move || {
                    summary.with(|opt| {
                        opt.as_ref()
                            .and_then(|s| s.model.clone())
                            .unwrap_or_default()
                    })
                })
            },
        ))
        .child(llm_badge(move || {
            let langs = ui_langs();
            summary.with(|opt| {
                let Some(s) = opt.as_ref() else {
                    return String::new();
                };
                format_metrics_summary(s, ui_lang(langs))
            })
        }))
        .child(when(
            move || {
                summary.with(|opt| {
                    opt.as_ref()
                        .and_then(|s| s.finish_reason.as_ref())
                        .is_some()
                })
            },
            move || {
                llm_badge(move || {
                    summary.with(|opt| {
                        opt.as_ref()
                            .and_then(|s| s.finish_reason.as_ref())
                            .map(|r| format!("stop: {r}"))
                            .unwrap_or_default()
                    })
                })
            },
        ))
}

fn llm_badge(label: impl Fn() -> String + 'static) -> impl IntoElement {
    view()
        .h(px(22.0))
        .px_2()
        .rounded(px(RADIUS_SM))
        .border_1()
        .border_color(border_control())
        .bg(bg_code())
        .flex_row()
        .items_center()
        .child(
            mono(move || -> SharedString { label().into() })
                .text_size(px(TEXT_XS))
                .text_color(fg_secondary()),
        )
}

fn format_metrics_summary(s: &llm_stream::LlmStreamSummary, lang: Lang) -> String {
    let mut parts = Vec::new();
    if let Some(ttft) = s.ttft_ms {
        parts.push(format!("TTFT {ttft}ms"));
    }
    if let Some(total) = s.total_ms {
        let lbl = match lang {
            Lang::Zh => "总耗时",
            Lang::En => "Total",
        };
        parts.push(format!("{lbl} {total}ms"));
    }
    let u = &s.usage;
    let mut tok_parts = Vec::new();
    if let Some(v) = u.input_tokens {
        tok_parts.push(format!("in:{v}"));
    }
    if let Some(v) = u.output_tokens {
        tok_parts.push(format!("out:{v}"));
    }
    if let Some(v) = u.reasoning_tokens {
        tok_parts.push(format!("reason:{v}"));
    }
    if !tok_parts.is_empty() {
        parts.push(format!(
            "{} ({})",
            tok_parts.join(" "),
            u.total_tokens.unwrap_or(0)
        ));
    }
    if let Some(tps) = s.tokens_per_sec() {
        parts.push(format!("{tps:.1} tok/s"));
    }
    parts.push(match lang {
        Lang::Zh => format!("{} 帧", s.chunk_count),
        Lang::En => format!("{} chunks", s.chunk_count),
    });
    parts.join(" · ")
}

fn llm_thinking_card(
    summary: Signal<Option<llm_stream::LlmStreamSummary>>,
    open: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    view()
        .w_full()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border_control())
        .bg(bg_code())
        .overflow_hidden()
        .child(
            view()
                .w_full()
                .px_2p5()
                .py_1p5()
                .flex_row()
                .items_center()
                .justify_between()
                .border_b(move || if open.get() { px(1.0) } else { px(0.0) })
                .border_b_color(border_subtle())
                .child(
                    view()
                        .role(AccessibilityRole::Button)
                        .px_1()
                        .py_0p5()
                        .rounded(px(RADIUS_SM))
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .hover(|s| s.bg(bg_control_hover()))
                        .active(|s| s.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .on_press(move || open.update(|v| *v = !*v))
                        .child(
                            text(move || -> SharedString {
                                let marker = if open.get() { "▾" } else { "▸" };
                                let count = summary.with(|opt| {
                                    opt.as_ref().map_or(0, |s| s.thinking.chars().count())
                                });
                                format!("{marker} Thinking ({count} chars)").into()
                            })
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg_secondary()),
                        ),
                )
                .child(detail_button(tr("复制思考", "Copy"), move || {
                    let text = summary.with_untracked(|opt| {
                        opt.as_ref().map(|s| s.thinking.clone()).unwrap_or_default()
                    });
                    if !text.is_empty() {
                        set_clipboard(text);
                        set_status(&controls, StatusMsg::CopiedThinking);
                    }
                })),
        )
        .child(when(
            move || open.get(),
            move || {
                view().w_full().p_2p5().child(
                    mono(move || -> SharedString {
                        summary
                            .with(|opt| {
                                opt.as_ref().map(|s| s.thinking.clone()).unwrap_or_default()
                            })
                            .into()
                    })
                    .selectable(true)
                    .text_size(px(TEXT_SM))
                    .line_height(px(18.0))
                    .text_color(fg_muted()),
                )
            },
        ))
}

fn llm_output_card(
    summary: Signal<Option<llm_stream::LlmStreamSummary>>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    view()
        .w_full()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border_control())
        .bg(bg_code())
        .overflow_hidden()
        .child(
            view()
                .w_full()
                .px_2p5()
                .py_1p5()
                .flex_row()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_b_color(border_subtle())
                .child(
                    text(tr("模型输出", "Assistant Output"))
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg_secondary()),
                )
                .child(when(
                    move || summary.with(|opt| opt.as_ref().is_some_and(|s| !s.content.is_empty())),
                    move || {
                        let copy_controls = controls.clone();
                        detail_button(tr("复制正文", "Copy text"), move || {
                            let text = summary.with_untracked(|opt| {
                                opt.as_ref().map(|s| s.content.clone()).unwrap_or_default()
                            });
                            if !text.is_empty() {
                                set_clipboard(text);
                                set_status(&copy_controls, StatusMsg::CopiedMessage);
                            }
                        })
                    },
                )),
        )
        .child(
            view().w_full().p_2p5().child(
                mono(move || -> SharedString {
                    let body = summary
                        .with(|opt| opt.as_ref().map(|s| s.content.clone()).unwrap_or_default());
                    if body.is_empty() {
                        "(empty text output)".into()
                    } else {
                        body.into()
                    }
                })
                .selectable(true)
                .text_size(px(TEXT_SM))
                .line_height(px(18.0))
                .text_color(move || {
                    if summary.with(|opt| opt.as_ref().is_some_and(|s| s.content.is_empty())) {
                        fg_muted()
                    } else {
                        fg_primary()
                    }
                }),
            ),
        )
}

fn llm_tool_calls_card(
    summary: Signal<Option<llm_stream::LlmStreamSummary>>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let items = move || {
        summary.with(|opt| {
            opt.as_ref()
                .map(|s| s.tool_calls.clone())
                .unwrap_or_default()
        })
    };
    view()
        .w_full()
        .gap_1p5()
        .flex_col()
        .child(
            text(move || -> SharedString {
                let n = items().len();
                let langs = ui_langs();
                match ui_lang(langs) {
                    Lang::Zh => format!("工具调用 ({n})"),
                    Lang::En => format!("Tool Calls ({n})"),
                }
                .into()
            })
            .text_size(px(TEXT_XS))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(fg_secondary()),
        )
        .child(each(
            items,
            |tc| format!("{}:{}:{}", tc.id, tc.name, tc.arguments.len()),
            move |tc, _| {
                let copy_controls = controls.clone();
                let formatted = move || format_tool_args(&tc.with(|item| item.arguments.clone()));
                view()
                    .w_full()
                    .rounded(px(RADIUS_MD))
                    .border_1()
                    .border_color(border_control())
                    .bg(bg_code())
                    .overflow_hidden()
                    .child(
                        view()
                            .w_full()
                            .px_2p5()
                            .py_1p5()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_b_color(border_subtle())
                            .child(
                                mono(move || -> SharedString {
                                    tc.with(|item| {
                                        if item.id.is_empty() {
                                            item.name.clone()
                                        } else {
                                            format!("{} ({})", item.name, item.id)
                                        }
                                    })
                                    .into()
                                })
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(accent_blue_text()),
                            )
                            .child(detail_button(tr("复制参数", "Copy args"), move || {
                                let raw = tc.with_untracked(|item| item.arguments.clone());
                                let pretty = format_tool_args(&raw);
                                set_clipboard(pretty);
                                set_status(&copy_controls, StatusMsg::CopiedToolArgs);
                            })),
                    )
                    .child(
                        view().w_full().p_2p5().child(
                            mono(move || -> SharedString { formatted().into() })
                                .selectable(true)
                                .text_size(px(TEXT_SM))
                                .line_height(px(18.0))
                                .text_color(fg_primary()),
                        ),
                    )
            },
        ))
}

fn format_tool_args(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "{}".to_string();
    }
    serde_json::from_str::<serde_json::Value>(trimmed)
        .and_then(|v| serde_json::to_string_pretty(&v))
        .unwrap_or_else(|_| raw.to_string())
}
