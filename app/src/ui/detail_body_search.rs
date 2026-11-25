#[allow(unused_imports)]
use super::*;

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

pub(crate) fn next_paged_match_offset(
    full_text: &str,
    window: TextWindow,
    query: &str,
) -> Option<usize> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() || full_text.is_empty() {
        return None;
    }
    let lower = full_text.to_ascii_lowercase();
    let end = window.end.min(lower.len());
    let start = window.offset.min(lower.len());
    let hit = lower[end..]
        .find(&q)
        .map(|rel| end + rel)
        .or_else(|| lower[..start].find(&q))?;
    let mut page = (hit / BODY_DISPLAY_CAP) * BODY_DISPLAY_CAP;
    if page != 0 && !full_text.is_char_boundary(page) {
        page = full_text.floor_char_boundary(page);
    }
    Some(page)
}

fn find_next_body_match_page(
    shown: Shown,
    controls: &Controls,
    selected: Signal<Option<u64>>,
    response: bool,
    query: &str,
) -> Option<usize> {
    if query.trim().is_empty() {
        return None;
    }
    let id = selected.get()?;
    let flow = with_active_session(controls, |s| s.hydrated(id))?;
    let (raw, headers, window) = if response {
        (
            &flow.response_body,
            &flow.response_headers,
            shown.response_window.get(),
        )
    } else {
        (
            &flow.request_body,
            &flow.request_headers,
            shown.request_window.get(),
        )
    };
    let utf8 = std::str::from_utf8(raw).ok()?;
    let full = if shown.pretty.get() {
        present::pretty_body(present::content_type(headers), utf8)
    } else {
        utf8.to_string()
    };
    next_paged_match_offset(&full, window, query)
}

fn jump_next_body_match(
    shown: Shown,
    controls: &Controls,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    lang: Lang,
    response: bool,
    query: &str,
) {
    let Some(id) = selected.get() else {
        return;
    };
    if let Some(next_offset) = find_next_body_match_page(shown, controls, selected, response, query)
    {
        if response {
            shown.response_offset.set(next_offset);
        } else {
            shown.request_offset.set(next_offset);
        }
        publish_detail_for_controls(&shown, controls, Some(id), lang, baseline.get());
    }
}

pub(crate) fn body_search_width(pane_width: f32) -> f32 {
    if pane_width > 0.0 && pane_width <= 460.0 {
        76.0
    } else if pane_width > 0.0 && pane_width < 520.0 {
        104.0
    } else {
        120.0
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn body_search_box(
    search: Signal<String>,
    body: Signal<String>,
    hex: Signal<Vec<u8>>,
    form: Signal<Vec<HeaderLine>>,
    json: Signal<Option<JsonBody>>,
    tree_flag: Signal<bool>,
    shown: Shown,
    response: bool,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    is_paged: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    let langs = ui_langs();
    let pane_width = shown.pane_width;
    let check_controls = controls.clone();
    let has_other_page = computed(move || {
        if !is_paged() {
            return false;
        }
        find_next_body_match_page(shown, &check_controls, selected, response, &search.get())
            .is_some()
    });
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(
            ui_input_a11y(input(), tr("搜索正文", "Search body"))
                .value(search)
                .placeholder(tr("搜索…", "Search…"))
                .placeholder_color(move || color(fg_muted()))
                .w(move || px(body_search_width(pane_width.get())))
                .h(px(ControlSize::Xs.height()))
                .px_1p5()
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
                let jump_controls = controls.clone();
                view()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(when(is_paged, move || {
                        let jump_controls = jump_controls.clone();
                        view()
                            .tooltip(tooltip().child(ui_tooltip_surface(
                                text(move || -> SharedString {
                                    let lang = ui_lang(langs);
                                    if has_other_page.get() {
                                        t(
                                            lang,
                                            "仅在当前 256 KB 分页内匹配；点击跳转到其它分页中的命中",
                                            "Matches current 256 KB page; click to jump to match in other pages",
                                        )
                                        .into()
                                    } else {
                                        t(
                                            lang,
                                            "仅在当前 256 KB 分页内匹配（其它分页中暂无更多命中）",
                                            "Matches current 256 KB page (no matches in other pages)",
                                        )
                                        .into()
                                    }
                                })
                                .text_size(px(TEXT_SM))
                                .text_color(fg_primary()),
                            )))
                            .child(
                                button()
                                    .h(px(ControlSize::Xs.height()))
                                    .px_1p5()
                                    .rounded(px(RADIUS_MD))
                                    .bg(status_warn_bg())
                                    .hover(|style| {
                                        style.bg(move || {
                                            if has_other_page.get() {
                                                status_warn_bg_hover()
                                            } else {
                                                status_warn_bg()
                                            }
                                        })
                                    })
                                    .active(|style| style.bg(bg_control_active()))
                                    .transition_colors()
                                    .duration(MOTION_INSTANT_MS)
                                    .ease_out()
                                    .focus_visible(apply_focus_ring)
                                    .disabled(move || !has_other_page.get())
                                    .disabled_style(|s| s.opacity(0.45))
                                    .border_1()
                                    .border_color(status_warn_fg())
                                    .flex_row()
                                    .items_center()
                                    .on_press(move |_: &()| {
                                        jump_next_body_match(
                                            shown,
                                            &jump_controls,
                                            selected,
                                            baseline,
                                            ui_lang(langs),
                                            response,
                                            &search.get(),
                                        );
                                    })
                                    .child(
                                        text(tr("当前页 · 下一处", "Page · Next"))
                                            .number_of_lines(1)
                                            .text_size(px(TEXT_XS))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(status_warn_fg()),
                                    ),
                            )
                    }))
                    .child(
                        view()
                            .tooltip(label_tip("点击清空搜索", "Click to clear search"))
                            .child(
                                button()
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
                                    .on_press(move |_: &()| search.set(String::new()))
                                    .child(
                                        mono(move || {
                                            let hits = body_search_hit_count(
                                                body,
                                                hex,
                                                form,
                                                json,
                                                tree_flag,
                                                pane_width,
                                                &search.get(),
                                            );
                                            format!("{} ×", present::format_thousands(hits as u64))
                                        })
                                        .tabular_nums()
                                        .text_size(px(TEXT_XS))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(accent_blue_text()),
                                    ),
                            ),
                    )
            },
        ))
}

pub(crate) fn body_search_empty_state(search: Signal<String>) -> impl IntoElement {
    let langs = ui_langs();
    view()
        .w_full()
        .px_3p5()
        .py_3()
        .bg(bg_code())
        .border_t_1()
        .border_t_color(border_subtle())
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            text(tr("当前正文无匹配内容", "No matching lines in body"))
                .text_size(px(TEXT_SM))
                .text_color(fg_muted()),
        )
        .child(
            ui_button(ButtonVariant::Outline, ControlSize::Xs)
                .accessibility_label(move || {
                    Some(t(ui_lang(langs), "清空搜索", "Clear search").into())
                })
                .on_press(move || search.set(String::new()))
                .child(control_text(tr("清空搜索", "Clear search")).text_color(fg_secondary())),
        )
}
