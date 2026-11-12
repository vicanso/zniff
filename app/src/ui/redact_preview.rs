#[allow(unused_imports)]
use super::*;

pub(crate) fn begin_redact_preview(
    _controls: &Controls,
    rows: Signal<Vec<Arc<RowLine>>>,
    seq_builder: SeqBuilder,
    compose: Compose,
    selected: Signal<Option<u64>>,
    shown_phase: Signal<Phase>,
) {
    let pick = selected
        .get()
        .or_else(|| rows.with(|list| list.last().map(|r| r.id)));
    seq_builder.redact_flow_id.set(pick);
    reset_compose(compose);
    selected.set(None);
    shown_phase.set(Phase::Empty);
    seq_builder.open.set(false);
    seq_builder.redact_open.set(true);
}

pub(crate) fn redact_preview_panel(
    seq_builder: SeqBuilder,
    controls: Arc<Controls>,
    rows: Signal<Vec<Arc<RowLine>>>,
    filter: Signal<String>,
) -> impl IntoElement {
    let toggle_controls = controls.clone();
    let strict_controls = controls.clone();
    let fields_controls = controls.clone();
    let export_har_controls = controls.clone();
    let export_html_controls = controls.clone();
    let preview_controls = controls;
    let langs = ui_langs();

    let preview_pair = computed(move || {
        let zh = ui_lang(langs) == Lang::Zh;
        let flow_id = seq_builder
            .redact_flow_id
            .get()
            .or_else(|| rows.with(|list| list.last().map(|r| r.id)));
        let extra = seq_builder.redact_fields.get();
        let opts =
            share::RedactOptions::new(true, &extra).with_strict(seq_builder.redact_strict.get());
        let Some(id) = flow_id else {
            return (
                None,
                String::new(),
                String::new(),
                share::RedactReport::default(),
            );
        };
        let fallback_id = rows.with(|list| list.last().map(|r| r.id));
        let Some(flow) = preview_controls.hub.with_session(|session| {
            session.hydrated(id).or_else(|| {
                fallback_id
                    .filter(|&last_id| last_id != id)
                    .and_then(|last_id| session.hydrated(last_id))
            })
        }) else {
            return (
                None,
                String::new(),
                String::new(),
                share::RedactReport::default(),
            );
        };
        let (before, after, report) = share::format_redact_preview(&flow, &opts, zh);
        (
            Some((flow.id, flow.method, flow.url)),
            before,
            after,
            report,
        )
    });

    view()
        .rounded(px(10.0))
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
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(section_title(tr(
                            "导出与分享脱敏设置",
                            "Export & share redaction",
                        )))
                        .child(
                            view()
                                .h(px(24.0))
                                .px_2p5()
                                .rounded(px(5.0))
                                .border_1()
                                .border_color(move || {
                                    if seq_builder.redact_export.get() {
                                        status_ok_fg()
                                    } else {
                                        border_control()
                                    }
                                })
                                .bg(move || {
                                    if seq_builder.redact_export.get() {
                                        status_ok_bg()
                                    } else {
                                        bg_control()
                                    }
                                })
                                .hover(|style| {
                                    style.bg(move || {
                                        if seq_builder.redact_export.get() {
                                            status_ok_bg_hover()
                                        } else {
                                            bg_control_hover()
                                        }
                                    })
                                })
                                .active(|style| style.opacity(0.8))
                                .flex_row()
                                .items_center()
                                .cursor(Cursor::Pointer)
                                .on_press(move || {
                                    let next = !seq_builder.redact_export.get();
                                    seq_builder.redact_export.set(next);
                                    *toggle_controls
                                        .redact_export
                                        .lock()
                                        .unwrap_or_else(|err| err.into_inner()) = next;
                                    save_config_or_notify(
                                        &toggle_controls,
                                        PrefName::Redact,
                                        |cfg| cfg.redact_export = next,
                                    );
                                })
                                .child(
                                    text(move || -> SharedString {
                                        let lang = ui_lang(langs);
                                        if seq_builder.redact_export.get() {
                                            t(lang, "脱敏：已开启", "Redaction: ON").into()
                                        } else {
                                            t(
                                                lang,
                                                "脱敏：已关闭（点击开启）",
                                                "Redaction: OFF (click to enable)",
                                            )
                                            .into()
                                        }
                                    })
                                    .number_of_lines(1)
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || {
                                        if seq_builder.redact_export.get() {
                                            status_ok_fg()
                                        } else {
                                            fg_secondary()
                                        }
                                    }),
                                ),
                        )
                        .child(
                            view()
                                .h(px(24.0))
                                .px_2p5()
                                .rounded(px(5.0))
                                .border_1()
                                .border_color(move || {
                                    if seq_builder.redact_strict.get() {
                                        accent_blue_border()
                                    } else {
                                        border_control()
                                    }
                                })
                                .bg(move || {
                                    if seq_builder.redact_strict.get() {
                                        accent_blue_soft()
                                    } else {
                                        bg_control()
                                    }
                                })
                                .hover(|style| {
                                    style.bg(move || {
                                        if seq_builder.redact_strict.get() {
                                            accent_blue_soft_hover()
                                        } else {
                                            bg_control_hover()
                                        }
                                    })
                                })
                                .active(|style| style.opacity(0.8))
                                .flex_row()
                                .items_center()
                                .cursor(Cursor::Pointer)
                                .label(tr("严格遮罩", "Strict mask"))
                                .tooltip(label_tip(
                                    "开启后将无法按字段脱敏的非结构化或二进制正文替换为占位文本",
                                    "Replace unstructured or binary bodies with a placeholder when redacting",
                                ))
                                .on_press(move || {
                                    let next = !seq_builder.redact_strict.get();
                                    seq_builder.redact_strict.set(next);
                                    *strict_controls
                                        .redact_strict
                                        .lock()
                                        .unwrap_or_else(|err| err.into_inner()) = next;
                                    save_config_or_notify(
                                        &strict_controls,
                                        PrefName::Redact,
                                        |cfg| cfg.redact_strict = next,
                                    );
                                })
                                .child(
                                    text(tr("严格遮罩", "Strict mask"))
                                        .number_of_lines(1)
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || {
                                            if seq_builder.redact_strict.get() {
                                                accent_blue_text()
                                            } else {
                                                fg_secondary()
                                            }
                                        }),
                                ),
                        ),
                )
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_1p5()
                        .child(seq_action_button(
                            tr("导出 HAR", "Export HAR"),
                            || true,
                            move || {
                                export_har_file(export_har_controls.clone(), &filter.get());
                            },
                        ))
                        .child(seq_action_button(
                            tr("导出 HTML", "Export HTML"),
                            || true,
                            move || {
                                export_html_file(
                                    export_html_controls.clone(),
                                    &filter.get(),
                                    ui_lang(langs) == Lang::Zh,
                                );
                            },
                        ))
                        .child(seq_action_button(tr("关闭", "Close"), || true, move || {
                            seq_builder.redact_open.set(false);
                        })),
                ),
        )
        .child(
            view()
                .p_3p5()
                .gap_2p5()
                .child(
                    text(tr(
                        "默认脱敏：Authorization、Proxy-Authorization、Cookie、Set-Cookie、X-Api-Key，以及 URL 查询参数、JSON 和表单中的 password、token、access_token、secret、api_key、session_id、jwt 等字段。仅作用于导出的 HAR/HTML 与实时分享副本，不改动抓包记录或重放数据。",
                        "Defaults: Authorization, Proxy-Authorization, Cookie, Set-Cookie, X-Api-Key, plus password, token, access_token, secret, api_key, session_id, jwt in query/JSON/form. Sanitizes exported/shared copies only without modifying captured flows or replay data.",
                    ))
                    .text_size(px(11.5))
                    .text_color(fg_muted()),
                )
                .child(
                    view()
                        .gap_1()
                        .child(
                            text(tr(
                                "附加敏感字段或请求头名（逗号或空格分隔）",
                                "Extra sensitive field or header names (comma or space separated)",
                            ))
                            .text_size(px(11.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg_secondary()),
                        )
                        .child(
                            input()
                                .value(seq_builder.redact_fields)
                                .placeholder(tr(
                                    "例如: x-custom-token, phone, id_card",
                                    "e.g. x-custom-token, phone, card_number",
                                ))
                                .placeholder_color(move || color(fg_muted()))
                                .h(px(30.0))
                                .px_2p5()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(border_control())
                                .focus(|style| style.border_color(accent_blue()))
                                .bg(bg_code())
                                .text_size(px(12.0))
                                .text_color(fg_primary())
                                .on_change(move |val: &str| {
                                    let text_val = val.to_string();
                                    seq_builder.redact_fields.set(text_val.clone());
                                    *fields_controls
                                        .redact_fields
                                        .lock()
                                        .unwrap_or_else(|err| err.into_inner()) = text_val.clone();
                                    save_config_or_notify(
                                        &fields_controls,
                                        PrefName::Redact,
                                        |cfg| cfg.redact_fields = text_val,
                                    );
                                }),
                        ),
                )
                .child({
                    view()
                        .mt_1()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(border_subtle())
                        .bg(bg_code())
                        .overflow_hidden()
                        .child(
                            view()
                                .flex_row()
                                .flex_wrap()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .px_3()
                                .py_2()
                                .border_b_1()
                                .border_b_color(border_subtle())
                                .child(
                                    mono(move || {
                                        let lang = ui_lang(langs);
                                        preview_pair.with(|(meta, _, _, report)| {
                                            let Some((id, method, url)) = meta else {
                                                return t(
                                                    lang,
                                                    "脱敏预览（当前列表暂无流量）",
                                                    "Redaction preview (no flows in current list)",
                                                )
                                                .to_string();
                                            };
                                            match lang {
                                                Lang::Zh => format!(
                                                    "预览 #{id} {method} {url} · 脱敏头 {} · 参数 {} · 正文字段 {}",
                                                    report.redacted_headers,
                                                    report.redacted_query_params,
                                                    report.redacted_body_fields,
                                                ),
                                                Lang::En => format!(
                                                    "Preview #{id} {method} {url} · headers {} · query {} · body fields {}",
                                                    report.redacted_headers,
                                                    report.redacted_query_params,
                                                    report.redacted_body_fields,
                                                ),
                                            }
                                        })
                                    })
                                    .tabular_nums()
                                    .flex_1()
                                    .min_w(px(160.0))
                                    .number_of_lines(1)
                                    .text_overflow(EllipsizeMode::Middle)
                                    .text_size(px(11.5))
                                    .text_color(fg_secondary()),
                                )
                                .child(when(
                                    move || {
                                        preview_pair.with(|(meta, _, _, report)| {
                                            meta.is_some()
                                                && (report.has_unsanitizable_body()
                                                    || report.has_strict_masked_body())
                                        })
                                    },
                                    move || {
                                        view()
                                            .px_2()
                                            .py_0p5()
                                            .rounded(px(5.0))
                                            .bg(status_warn_bg())
                                            .border_1()
                                            .border_color(status_warn_fg())
                                            .flex_row()
                                            .items_center()
                                            .gap_1()
                                            .child(glyph_px(
                                                12.0,
                                                || Glyph::Alert,
                                                || color(status_warn_fg()),
                                            ))
                                            .child(
                                                text(move || -> SharedString {
                                                    let lang = ui_lang(langs);
                                                    let strict_masked = preview_pair.with(
                                                        |(_, _, _, report)| {
                                                            report.has_strict_masked_body()
                                                        },
                                                    );
                                                    if strict_masked {
                                                        t(
                                                            lang,
                                                            "严格模式已遮罩非结构化/二进制正文",
                                                            "Unstructured/binary body masked by strict mode",
                                                        )
                                                        .into()
                                                    } else {
                                                        t(
                                                            lang,
                                                            "含非结构化/二进制正文（无法按字段脱敏）",
                                                            "Body format cannot be structurally sanitized",
                                                        )
                                                        .into()
                                                    }
                                                })
                                                .number_of_lines(1)
                                                .text_size(px(11.0))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(status_warn_fg()),
                                            )
                                    },
                                )),
                        )
                        .child(
                            guard_culled_pointer_events(view())
                                .h(move || px((window().height.0 - 300.0).clamp(180.0, 480.0)))
                                .overflow_hidden()
                                .child(
                                    diff_view()
                                        .old_value(move || {
                                            preview_pair.with(|(_, before, _, _)| {
                                                SharedString::from(before.clone())
                                            })
                                        })
                                        .new_value(move || {
                                            preview_pair.with(|(_, _, after, _)| {
                                                SharedString::from(after.clone())
                                            })
                                        })
                                        .layout(DiffLayout::Unified)
                                        .language("plaintext")
                                        .soft_wrap(true)
                                        .font_size(Points(12.0))
                                        .line_height(Points(18.0))
                                        .size_full(),
                                ),
                        )
                }),
        )
}
