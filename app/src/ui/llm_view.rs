#[allow(unused_imports)]
use super::*;

pub(crate) fn llm_stream_panel(
    summary: Signal<Option<llm_stream::LlmStreamSummary>>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let thinking_open = signal(false);
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
                let langs = ui_langs();
                llm_badge(move || {
                    let stop_lbl = t(ui_lang(langs), "结束原因", "finish");
                    summary.with(|opt| {
                        opt.as_ref()
                            .and_then(|s| s.finish_reason.as_ref())
                            .map(|r| format!("{stop_lbl}: {r}"))
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
    let (in_lbl, out_lbl, reason_lbl) = match lang {
        Lang::Zh => ("输入", "输出", "思考"),
        Lang::En => ("in", "out", "reason"),
    };
    let mut tok_parts = Vec::new();
    if let Some(v) = u.input_tokens {
        tok_parts.push(format!("{in_lbl}:{v}"));
    }
    if let Some(v) = u.output_tokens {
        tok_parts.push(format!("{out_lbl}:{v}"));
    }
    if let Some(v) = u.reasoning_tokens {
        tok_parts.push(format!("{reason_lbl}:{v}"));
    }
    if !tok_parts.is_empty() {
        let total = u
            .total_tokens
            .or_else(|| match (u.input_tokens, u.output_tokens) {
                (Some(i), Some(o)) => Some(i.saturating_add(o)),
                _ => None,
            });
        if let Some(total) = total {
            parts.push(format!("{} ({total})", tok_parts.join(" ")));
        } else {
            parts.push(tok_parts.join(" "));
        }
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
    let langs = ui_langs();
    view()
        .w_full()
        .rounded(px(RADIUS_MD))
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
                        .duration(move || motion_ms(MOTION_INSTANT_MS))
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .label(tr("展开或收起思考过程", "Toggle thinking"))
                        .on_press(move || open.update(|v| *v = !*v))
                        .child(ui_rotating_chevron(
                            10.0,
                            move || open.get(),
                            || color(fg_secondary()),
                        ))
                        .child(
                            text(move || -> SharedString {
                                let count = summary.with(|opt| {
                                    opt.as_ref().map_or(0, |s| s.thinking.chars().count())
                                });
                                match ui_lang(langs) {
                                    Lang::Zh => format!("思考过程 ({count} 字)").into(),
                                    Lang::En => format!("Thinking ({count} chars)").into(),
                                }
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
                view().w_full().child(ui_divider()).child(
                    guard_culled_pointer_events(view()).w_full().child(
                        scroll().w_full().max_h(px(260.0)).child(
                            view().w_full().p_2p5().child(
                                mono(move || -> SharedString {
                                    summary
                                        .with(|opt| {
                                            opt.as_ref()
                                                .map(|s| s.thinking.clone())
                                                .unwrap_or_default()
                                        })
                                        .into()
                                })
                                .selectable(true)
                                .text_size(move || scaled_px(12.0))
                                .line_height(move || scaled_px(18.0))
                                .text_color(fg_muted()),
                            ),
                        ),
                    ),
                )
            },
        ))
}

fn llm_output_card(
    summary: Signal<Option<llm_stream::LlmStreamSummary>>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let langs = ui_langs();
    view()
        .w_full()
        .rounded(px(RADIUS_MD))
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
        .child(ui_divider())
        .child(
            guard_culled_pointer_events(view()).w_full().child(
                scroll().w_full().max_h(px(260.0)).child(
                    view().w_full().p_2p5().child(
                        mono(move || -> SharedString {
                            let body = summary.with(|opt| {
                                opt.as_ref().map(|s| s.content.clone()).unwrap_or_default()
                            });
                            if body.is_empty() {
                                t(ui_lang(langs), "（无文本输出）", "(empty text output)").into()
                            } else {
                                body.into()
                            }
                        })
                        .selectable(true)
                        .text_size(move || scaled_px(12.0))
                        .line_height(move || scaled_px(18.0))
                        .text_color(move || {
                            if summary
                                .with(|opt| opt.as_ref().is_some_and(|s| s.content.is_empty()))
                            {
                                fg_muted()
                            } else {
                                fg_primary()
                            }
                        }),
                    ),
                ),
            ),
        )
}

fn tool_call_item_key((idx, tc): &(usize, llm_stream::LlmToolCall)) -> String {
    if tc.id.is_empty() {
        format!("idx:{idx}")
    } else {
        format!("{idx}:{}", tc.id)
    }
}

fn llm_tool_calls_card(
    summary: Signal<Option<llm_stream::LlmStreamSummary>>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let items = move || {
        summary.with(|opt| {
            opt.as_ref()
                .map(|s| s.tool_calls.iter().cloned().enumerate().collect::<Vec<_>>())
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
        .child(each(items, tool_call_item_key, move |tc, _| {
            let copy_controls = controls.clone();
            let formatted = move || format_tool_args(&tc.with(|(_, item)| item.arguments.clone()));
            view()
                .w_full()
                .rounded(px(RADIUS_MD))
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
                        .child(
                            mono(move || -> SharedString {
                                tc.with(|(_, item)| {
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
                            let raw = tc.with_untracked(|(_, item)| item.arguments.clone());
                            let pretty = format_tool_args(&raw);
                            set_clipboard(pretty);
                            set_status(&copy_controls, StatusMsg::CopiedToolArgs);
                        })),
                )
                .child(ui_divider())
                .child(
                    guard_culled_pointer_events(view()).w_full().child(
                        scroll().w_full().max_h(px(260.0)).child(
                            view().w_full().p_2p5().child(
                                mono(move || -> SharedString { formatted().into() })
                                    .selectable(true)
                                    .text_size(move || scaled_px(12.0))
                                    .line_height(move || scaled_px(18.0))
                                    .text_color(fg_primary()),
                            ),
                        ),
                    ),
                )
        }))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_call_key_stays_stable_while_arguments_stream() {
        let a = (
            0,
            llm_stream::LlmToolCall {
                id: "call_1".into(),
                name: "get_weather".into(),
                arguments: "{\"city\":\"Sh".into(),
            },
        );
        let b = (
            0,
            llm_stream::LlmToolCall {
                id: "call_1".into(),
                name: "get_weather".into(),
                arguments: "{\"city\":\"Shanghai\"}".into(),
            },
        );
        assert_eq!(tool_call_item_key(&a), tool_call_item_key(&b));
        let empty = (
            1,
            llm_stream::LlmToolCall {
                id: String::new(),
                name: "fn".into(),
                arguments: String::new(),
            },
        );
        assert_eq!(tool_call_item_key(&empty), "idx:1");
    }

    #[test]
    fn metrics_summary_falls_back_or_omits_missing_total_tokens() {
        let mut s = llm_stream::LlmStreamSummary {
            provider: llm_stream::LlmProvider::Anthropic,
            model: Some("claude".into()),
            finish_reason: Some("stop".into()),
            thinking: String::new(),
            content: "hi".into(),
            tool_calls: Vec::new(),
            usage: llm_stream::LlmUsage {
                input_tokens: Some(12),
                output_tokens: Some(8),
                reasoning_tokens: None,
                total_tokens: None,
            },
            chunk_count: 3,
            ttft_ms: Some(100),
            total_ms: Some(500),
        };
        let sum = format_metrics_summary(&s, Lang::En);
        assert!(sum.contains("in:12 out:8 (20)"), "{sum}");

        s.usage.input_tokens = None;
        let partial = format_metrics_summary(&s, Lang::En);
        assert!(partial.contains("out:8"), "{partial}");
        assert!(!partial.contains("(0)"), "{partial}");
    }
}
