#[allow(unused_imports)]
use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn body_section(
    zh: &'static str,
    en: &'static str,
    body: Signal<String>,
    hex: Signal<Vec<u8>>,
    note: Signal<String>,
    code: Signal<String>,
    doc: Signal<Option<nsh_base::text_view::Document>>,
    form: Signal<Vec<HeaderLine>>,
    search: Signal<String>,
    embed_image: bool,
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    folded: Signal<u8>,
    bit: u8,
) -> impl IntoElement {
    let (preview_flag, json, tree_flag, side) = if embed_image {
        (
            shown.res_doc_preview,
            shown.response_json,
            shown.res_json_tree,
            Side::Response,
        )
    } else {
        (
            shown.req_doc_preview,
            shown.request_json,
            shown.req_json_tree,
            Side::Request,
        )
    };
    let image_on = move || embed_image && shown.pretty.get() && !shown.preview.get().is_empty();
    let doc_on = move || preview_flag.get() && doc.with(Option::is_some);
    let tree_on = move || !doc_on() && tree_flag.get() && json.with(Option::is_some);
    let has_body_below = move || {
        doc_on() || tree_on() || image_on() || !form.with(Vec::is_empty) || !body.get().is_empty()
    };
    let can_search = move || {
        !card_folded(folded, bit)
            && !doc_on()
            && (tree_on() || !form.with(Vec::is_empty) || !body.get().is_empty())
    };
    let pane_width = shown.pane_width;
    let nav_controls = controls.clone();
    let switch_controls = controls.clone();
    let tree_controls = controls;
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
                    if card_folded(folded, bit) {
                        px(0.0)
                    } else {
                        px(1.0)
                    }
                })
                .border_b_color(border_subtle())
                .child(
                    view()
                        .flex_1()
                        .min_w(px(96.0))
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .px_1()
                        .py_0p5()
                        .rounded(px(5.0))
                        .hover(|style| style.bg(bg_control_hover()))
                        .active(|style| style.opacity(0.85))
                        .cursor(Cursor::Pointer)
                        .on_press(move || toggle_card(folded, bit))
                        .child(fold_mark(folded, bit))
                        .child(section_title(tr(zh, en))),
                )
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_1p5()
                        .child(when(
                            move || {
                                if tree_on() {
                                    return false;
                                }
                                let window = if embed_image {
                                    shown.response_window.get()
                                } else {
                                    shown.request_window.get()
                                };
                                window.has_prev() || window.has_more()
                            },
                            {
                                let controls = nav_controls;
                                move || {
                                    body_window_nav(
                                        embed_image,
                                        shown,
                                        controls.clone(),
                                        selected,
                                        baseline,
                                    )
                                }
                            },
                        ))
                        .child(when(can_search, move || {
                            let is_paged = move || {
                                if tree_on() {
                                    return false;
                                }
                                let window = if embed_image {
                                    shown.response_window.get()
                                } else {
                                    shown.request_window.get()
                                };
                                window.has_prev() || window.has_more()
                            };
                            body_search_box(
                                search,
                                body,
                                hex,
                                form,
                                json,
                                tree_flag,
                                shown.pane_width,
                                is_paged,
                            )
                        }))
                        .child(when(
                            move || doc.with(Option::is_some),
                            move || doc_preview_chip(preview_flag),
                        ))
                        .child(pretty_switch(
                            shown,
                            preview_flag,
                            json,
                            tree_flag,
                            switch_controls,
                            selected,
                            baseline,
                        )),
                ),
        )
        .child(when(
            move || !card_folded(folded, bit) && !note.get().is_empty(),
            move || {
                view()
                    .px_3()
                    .pt_3()
                    .pb(move || if has_body_below() { px(0.0) } else { px(12.0) })
                    .child(
                        view()
                            .px_3()
                            .py_2()
                            .rounded(px(7.0))
                            .bg(status_warn_bg())
                            .border_1()
                            .border_color(status_warn_fg())
                            .child(
                                text(move || note.get())
                                    .text_size(px(12.0))
                                    .text_color(status_warn_fg()),
                            ),
                    )
            },
        ))
        .child(when(
            move || !card_folded(folded, bit) && doc_on(),
            move || {
                let divided = !note.get().is_empty();
                view()
                    .w_full()
                    .h(move || px(editor_box_height(&body.get(), 0.0).max(140.0)))
                    .bg(bg_code())
                    .overflow_hidden()
                    .border_t(move || if divided { px(1.0) } else { px(0.0) })
                    .border_t_color(border_subtle())
                    .child(
                        scroll()
                            .size_full()
                            .pl_3p5()
                            .pr_5()
                            .py_2p5()
                            .child(doc_body(doc)),
                    )
            },
        ))
        .child(when(move || !card_folded(folded, bit) && tree_on(), {
            let tree_controls = tree_controls.clone();
            move || {
                let divided = !note.get().is_empty();
                view()
                    .border_t(move || if divided { px(1.0) } else { px(0.0) })
                    .border_t_color(border_subtle())
                    .child(json_tree_body(json, search, side, tree_controls.clone()))
            }
        }))
        .child(when(
            move || !card_folded(folded, bit) && !doc_on() && !tree_on() && image_on(),
            move || {
                view()
                    .px_3()
                    .py_3()
                    .gap_1p5()
                    .items_center()
                    .justify_center()
                    .child(
                        view()
                            .w(move || px(shown.preview_w.get()))
                            .h(move || px(shown.preview_h.get()))
                            .min_w(px(64.0))
                            .min_h(px(64.0))
                            .flex_shrink_0()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(border_strong())
                            .bg(bg_code())
                            .overflow_hidden()
                            .child(
                                image(move || shown.preview.get())
                                    .w_full()
                                    .h_full()
                                    .object_fit(ResizeMode::ScaleDown),
                            ),
                    )
                    .child(when(
                        move || !shown.image_caption.get().is_empty(),
                        move || muted_line(move || shown.image_caption.get()),
                    ))
            },
        ))
        .child(when(
            move || {
                !card_folded(folded, bit)
                    && !doc_on()
                    && !tree_on()
                    && form.with(Vec::is_empty)
                    && !body.get().is_empty()
                    && code.get().is_empty()
            },
            move || {
                let divided = image_on() || !note.get().is_empty();
                let height_body = body;
                let height_hex = hex;
                let height_search = search;
                view()
                    .w_full()
                    .min_w(move || px(hex_area_width(shown.pane_width.get())))
                    .on_layout(move |bounds: &Bounds<Points>| {
                        let pane = bounds.size.width.0 + HEX_AREA_CHROME;
                        if hex_cols_for_pane(pane)
                            != hex_cols_for_pane(shown.pane_width.get_untracked())
                        {
                            shown.pane_width.set(pane);
                        }
                    })
                    .bg(bg_code())
                    .h(move || {
                        let text = filtered_plain_body(
                            &height_body.get(),
                            &height_hex.with(|bytes| bytes.clone()),
                            shown.pane_width.get(),
                            &height_search.get(),
                        );
                        px(body_box_height(body_line_count(&text)))
                    })
                    .overflow_hidden()
                    .border_t(move || if divided { px(1.0) } else { px(0.0) })
                    .border_t_color(border_subtle())
                    .child(
                        scroll().size_full().child(
                            mono(move || {
                                let bytes = hex.with(|bytes| bytes.clone());
                                filtered_plain_body(
                                    &body.get(),
                                    &bytes,
                                    shown.pane_width.get(),
                                    &search.get(),
                                )
                            })
                            .pl_3p5()
                            .pr_5()
                            .py_2()
                            .selectable(true)
                            .on_press_up(dismiss_keyboard)
                            .text_size(px(12.0))
                            .text_color(fg_primary()),
                        ),
                    )
            },
        ))
        .child(when(
            move || {
                !card_folded(folded, bit)
                    && !doc_on()
                    && !tree_on()
                    && form.with(Vec::is_empty)
                    && !body.get().is_empty()
                    && !code.get().is_empty()
            },
            move || {
                let divided = image_on() || !note.get().is_empty();
                view()
                    .border_t(move || if divided { px(1.0) } else { px(0.0) })
                    .border_t_color(border_subtle())
                    .child(code_body(body, code, search))
            },
        ))
        .child(when(
            move || {
                !card_folded(folded, bit) && !doc_on() && !tree_on() && !form.with(Vec::is_empty)
            },
            move || {
                each(
                    move || filtered_form_lines(&form.get(), &search.get()),
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
            },
        ))
}

pub(crate) fn body_window_nav(
    response: bool,
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
) -> impl IntoElement {
    let langs = ui_langs();
    let back_controls = controls.clone();
    let next_controls = controls;
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(when(
            move || {
                let window = if response {
                    shown.response_window.get()
                } else {
                    shown.request_window.get()
                };
                window.has_prev()
            },
            move || {
                let controls = back_controls.clone();
                detail_button(tr("往前", "Previous"), move || {
                    nudge_body_window(
                        shown,
                        &controls,
                        selected,
                        baseline,
                        ui_lang(langs),
                        response,
                        false,
                    )
                })
            },
        ))
        .child(when(
            move || {
                let window = if response {
                    shown.response_window.get()
                } else {
                    shown.request_window.get()
                };
                window.has_more()
            },
            move || {
                let controls = next_controls.clone();
                detail_button(tr("往后", "Next"), move || {
                    nudge_body_window(
                        shown,
                        &controls,
                        selected,
                        baseline,
                        ui_lang(langs),
                        response,
                        true,
                    )
                })
            },
        ))
}

pub(crate) fn nudge_body_window(
    shown: Shown,
    controls: &Controls,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    lang: Lang,
    response: bool,
    forward: bool,
) {
    let window = if response {
        shown.response_window.get()
    } else {
        shown.request_window.get()
    };
    if (forward && !window.has_more()) || (!forward && !window.has_prev()) {
        return;
    }
    let next = if forward {
        window.end
    } else {
        window.offset.saturating_sub(BODY_DISPLAY_CAP)
    };
    if response {
        shown.response_offset.set(next);
    } else {
        shown.request_offset.set(next);
    }
    publish_detail(&shown, &controls.hub, selected.get(), lang, baseline.get());
}

pub(crate) fn reset_body_offsets(shown: Shown) {
    if shown.request_offset.get_untracked() != 0 {
        shown.request_offset.set(0);
    }
    if shown.response_offset.get_untracked() != 0 {
        shown.response_offset.set(0);
    }
}

pub(crate) fn filtered_plain_body(body: &str, hex: &[u8], pane_width: f32, query: &str) -> String {
    let source = if hex.is_empty() {
        body.to_string()
    } else {
        let cols = hex_cols_for_pane(pane_width);
        hex_display_rows(hex, cols)
    };
    if query.trim().is_empty() {
        source
    } else {
        present::filter_body_lines(&source, query).1
    }
}

pub(crate) fn filtered_form_lines(lines: &[HeaderLine], query: &str) -> Vec<HeaderLine> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return lines.to_vec();
    }
    lines
        .iter()
        .filter(|line| {
            line.name.to_ascii_lowercase().contains(&q)
                || line.value.to_ascii_lowercase().contains(&q)
                || line
                    .hint
                    .as_ref()
                    .is_some_and(|h| h.to_ascii_lowercase().contains(&q))
        })
        .cloned()
        .collect()
}

pub(crate) fn body_search_hit_count(
    body: Signal<String>,
    hex: Signal<Vec<u8>>,
    form: Signal<Vec<HeaderLine>>,
    json: Signal<Option<JsonBody>>,
    tree_flag: Signal<bool>,
    pane_width: Signal<f32>,
    query: &str,
) -> usize {
    let q = query.trim();
    if q.is_empty() {
        return 0;
    }
    if tree_flag.get()
        && let Some(jb) = json.get()
    {
        return json_tree_search_hits(&jb, q);
    }
    if !form.with(Vec::is_empty) {
        return filtered_form_lines(&form.get(), q).len();
    }
    let bytes = hex.with(|bytes| bytes.clone());
    let source = if bytes.is_empty() {
        body.get()
    } else {
        hex_display_rows(&bytes, hex_cols_for_pane(pane_width.get()))
    };
    present::filter_body_lines(&source, q).0
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn body_search_box(
    search: Signal<String>,
    body: Signal<String>,
    hex: Signal<Vec<u8>>,
    form: Signal<Vec<HeaderLine>>,
    json: Signal<Option<JsonBody>>,
    tree_flag: Signal<bool>,
    pane_width: Signal<f32>,
    is_paged: impl Fn() -> bool + Copy + 'static,
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
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(when(is_paged, || {
                        view()
                            .h(px(22.0))
                            .px_1p5()
                            .rounded(px(6.0))
                            .bg(status_warn_bg())
                            .border_1()
                            .border_color(status_warn_fg())
                            .flex_row()
                            .items_center()
                            .tooltip(label_tip(
                                "仅在当前 256 KB 分页内匹配与过滤",
                                "Matches within the current 256 KB page only",
                            ))
                            .child(
                                text(tr("当前页命中", "Page matches"))
                                    .number_of_lines(1)
                                    .text_size(px(10.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(status_warn_fg()),
                            )
                    }))
                    .child(
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
                                    format!(
                                        "{} ×",
                                        body_search_hit_count(
                                            body,
                                            hex,
                                            form,
                                            json,
                                            tree_flag,
                                            pane_width,
                                            &search.get()
                                        )
                                    )
                                })
                                .tabular_nums()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(accent_blue_text()),
                            ),
                    )
            },
        ))
}

pub(crate) const BODY_LINE_H: f32 = 18.0;
pub(crate) const BODY_LINES: usize = 15;
pub(crate) const BODY_CHROME: f32 = 16.0;

pub(crate) fn body_line_count(text: &str) -> usize {
    text.lines().count().max(1)
}

/// One line of body grows the card. Past [`BODY_LINES`] the card stays put
/// and the text scrolls inside it.
pub(crate) fn body_box_height(lines: usize) -> f32 {
    let shown = lines.clamp(1, BODY_LINES) as f32;
    shown * BODY_LINE_H + BODY_CHROME
}

/// Width of one 12px monospaced character (Menlo and JetBrains Mono are
/// both about 7.2px).
const EDITOR_CHAR_W: f32 = 7.2;
/// The editor's line-number and fold gutter plus its scroll bar, measured
/// generously so the estimate errs towards one row too many.
const EDITOR_GUTTER_W: f32 = 84.0;
/// Columns assumed before the box has been laid out.
const EDITOR_FALLBACK_COLS: usize = 80;

/// How many characters fit on one soft-wrapped editor row in `width`.
pub(crate) fn editor_wrap_cols(width: f32) -> usize {
    if width <= EDITOR_GUTTER_W {
        return EDITOR_FALLBACK_COLS;
    }
    (((width - EDITOR_GUTTER_W) / EDITOR_CHAR_W).floor() as usize).max(20)
}

/// Rows the soft-wrapping editor draws for `text` at `cols` characters a
/// row, stopping once past [`BODY_LINES`]: minified JS or JSON is one long
/// line but many rows, and should open the box to its full height.
pub(crate) fn wrapped_line_count(text: &str, cols: usize) -> usize {
    let cols = cols.max(1);
    let mut rows = 0usize;
    for line in text.lines() {
        rows += line.chars().count().div_ceil(cols).max(1);
        if rows > BODY_LINES {
            break;
        }
    }
    rows.max(1)
}

pub(crate) fn editor_box_height(text: &str, width: f32) -> f32 {
    body_box_height(wrapped_line_count(text, editor_wrap_cols(width)))
}

pub(crate) fn code_body(
    body: Signal<String>,
    code: Signal<String>,
    search: Signal<String>,
) -> impl IntoElement {
    let shown_text = move || {
        let raw = body.get();
        let q = search.get();
        if q.trim().is_empty() {
            raw
        } else {
            present::filter_body_lines(&raw, &q).1
        }
    };
    let width = signal(0.0_f32);
    guard_culled_pointer_events(view())
        .w_full()
        .h(move || px(editor_box_height(&shown_text(), width.get())))
        .on_layout(move |bounds: &Bounds<Points>| {
            let w = bounds.size.width.0;
            if editor_wrap_cols(w) != editor_wrap_cols(width.get_untracked()) {
                width.set(w);
            }
        })
        .bg(bg_code())
        .overflow_hidden()
        .child(
            editor()
                .value(move || SharedString::from(shown_text()))
                .read_only(true)
                .language(move || {
                    if search.get().trim().is_empty() {
                        SharedString::from(code.get())
                    } else {
                        SharedString::from("plaintext")
                    }
                })
                .line_numbers(true)
                .folding(true)
                .soft_wrap(true)
                .font_size(Points(12.0))
                .line_height(Points(18.0))
                .scroll_beyond_last_line(false)
                .size_full(),
        )
}

pub(crate) fn doc_body(doc: Signal<Option<nsh_base::text_view::Document>>) -> impl IntoElement {
    nsh_base::text_view::text_view()
        .document(move || doc.get())
        .selectable(true)
        .render_image(|img: &nsh_base::text_view::Image| {
            let label = if img.alt.is_empty() {
                format!("[img: {}]", img.src)
            } else {
                format!("[img: {} ({})]", img.alt, img.src)
            };
            mono(label).text_size(px(11.5)).text_color(fg_muted())
        })
}

pub(crate) fn doc_preview_chip(preview: Signal<bool>) -> impl IntoElement {
    view()
        .h(px(22.0))
        .px_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || {
            if preview.get() {
                accent_blue()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if preview.get() {
                accent_blue()
            } else {
                bg_card()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if preview.get() {
                    accent_blue_hover()
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
            preview.set(!preview.get());
        })
        .child(
            text(tr("预览", "Preview"))
                .number_of_lines(1)
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if preview.get() {
                        AdaptiveColor::rgb(0xffffff, 0xffffff)
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

pub(crate) fn pretty_switch(
    shown: Shown,
    preview: Signal<bool>,
    json: Signal<Option<JsonBody>>,
    tree_flag: Signal<bool>,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
) -> impl IntoElement {
    view()
        .flex_row()
        .flex_shrink_0()
        .h(px(22.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(border_control())
        .overflow_hidden()
        .child(pretty_chip(
            shown,
            preview,
            tree_flag,
            controls.clone(),
            selected,
            baseline,
            true,
            "美化",
            "Pretty",
        ))
        .child(when(
            move || json.with(Option::is_some),
            move || tree_chip(preview, tree_flag),
        ))
        .child(pretty_chip(
            shown, preview, tree_flag, controls, selected, baseline, false, "原文", "Raw",
        ))
}

fn body_mode_chip(
    zh: &'static str,
    en: &'static str,
    has_right_border: bool,
    is_active: impl Fn() -> bool + Copy + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .h(px(22.0))
        .px_2()
        .flex_row()
        .items_center()
        .border_r(if has_right_border { px(1.0) } else { px(0.0) })
        .border_r_color(border_control())
        .bg(move || {
            if is_active() {
                accent_blue()
            } else {
                bg_card()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if is_active() {
                    accent_blue_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .on_press(on_press)
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if is_active() {
                        AdaptiveColor::rgb(0xffffff, 0xffffff)
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

fn tree_chip(preview: Signal<bool>, tree_flag: Signal<bool>) -> impl IntoElement {
    body_mode_chip(
        "树形",
        "Tree",
        true,
        move || !preview.get() && tree_flag.get(),
        move || {
            preview.set(false);
            tree_flag.set(true);
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn pretty_chip(
    shown: Shown,
    preview: Signal<bool>,
    tree_flag: Signal<bool>,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    on: bool,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    let langs = ui_langs();
    let is_active = move || !preview.get() && !tree_flag.get() && shown.pretty.get() == on;
    body_mode_chip(zh, en, on, is_active, move || {
        let changed = shown.pretty.get() != on;
        preview.set(false);
        tree_flag.set(false);
        if !changed {
            return;
        }
        shown.pretty.set(on);
        reset_body_offsets(shown);
        publish_detail(
            &shown,
            &controls.hub,
            selected.get(),
            ui_lang(langs),
            baseline.get(),
        );
    })
}
