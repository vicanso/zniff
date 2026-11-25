//! Filtered flow summary tile and HTTP status distribution card shown when no row is selected.

#[allow(unused_imports)]
use super::*;

pub(crate) fn filtered_summary_card(
    stats: Signal<session::FilteredStats>,
    selected: Signal<Option<u64>>,
    on_seq: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .p_3p5()
        .gap_3()
        .rounded(px(RADIUS_LG))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .child(section_title(tr("当前列表统计", "Filtered summary")))
                .child(detail_button(tr("生成时序图", "Sequence diagram"), on_seq)),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2p5()
                .child(summary_stat_tile(
                    tr("总请求", "Total requests"),
                    move || stats.with(|s| present::format_thousands(s.total_count as u64)),
                    || None,
                    1.0,
                    92.0,
                ))
                .child(summary_stat_tile(
                    tr("总传输", "Total transfer"),
                    move || stats.with(|s| present::bytes(s.total_bytes)),
                    move || {
                        stats.with(|s| {
                            Some(format!(
                                "↑{} · ↓{}",
                                present::bytes(s.total_req_bytes),
                                present::bytes(s.total_res_bytes)
                            ))
                        })
                    },
                    1.75,
                    148.0,
                ))
                .child(summary_stat_tile(
                    tr("平均耗时", "Avg latency"),
                    move || {
                        stats.with(|s| match s.avg_elapsed_ms {
                            Some(ms) => format!("{ms} ms"),
                            None => "—".to_string(),
                        })
                    },
                    || None,
                    1.0,
                    96.0,
                )),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_1p5()
                .child(status_count_badge(
                    "2xx",
                    move || stats.with(|s| s.s2xx),
                    status_ok_bg(),
                    status_ok_fg(),
                ))
                .child(status_count_badge(
                    "3xx",
                    move || stats.with(|s| s.s3xx),
                    status_warn_bg(),
                    status_warn_fg(),
                ))
                .child(status_count_badge(
                    "4xx",
                    move || stats.with(|s| s.s4xx),
                    status_err_bg(),
                    status_err_fg(),
                ))
                .child(status_count_badge(
                    "5xx",
                    move || stats.with(|s| s.s5xx),
                    status_err_bg(),
                    status_err_fg(),
                ))
                .child(status_count_badge(
                    "ERR",
                    move || stats.with(|s| s.failed),
                    status_err_bg(),
                    status_err_fg(),
                ))
                .child({
                    let langs = ui_langs();
                    view()
                        .h(px(22.0))
                        .px_2()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .rounded(px(RADIUS_SM))
                        .bg(bg_control())
                        .child(
                            mono(move || {
                                format!(
                                    "{} {}",
                                    t(ui_lang(langs), "进行中", "Pending"),
                                    present::format_thousands(stats.with(|s| s.pending) as u64)
                                )
                            })
                            .tabular_nums()
                            .number_of_lines(1)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg_muted()),
                        )
                }),
        )
        .child(when(
            move || stats.with(|s| s.max_elapsed.is_some()),
            move || {
                button()
                    .px_3()
                    .py_2()
                    .rounded(px(RADIUS_MD))
                    .border_1()
                    .border_color(border_subtle())
                    .bg(bg_code())
                    .hover(|style| style.bg(bg_control_hover()))
                    .active(|style| style.bg(bg_control_active()))
                    .transition_colors()
                    .duration(MOTION_INSTANT_MS)
                    .ease_out()
                    .focus_visible(apply_focus_ring)
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .on_press(move |_: &()| {
                        if let Some(id) =
                            stats.with(|s| s.max_elapsed.as_ref().map(|(id, _, _)| *id))
                        {
                            selected.set(Some(id));
                        }
                    })
                    .child(
                        text(tr("最慢请求", "Slowest"))
                            .flex_shrink_0()
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg_muted()),
                    )
                    .child(
                        mono(move || {
                            stats.with(|s| match &s.max_elapsed {
                                Some((id, ms, target)) => format!("#{id} · {ms} ms · {target}"),
                                None => String::new(),
                            })
                        })
                        .tabular_nums()
                        .flex_1()
                        .min_w(px(0.0))
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Middle)
                        .text_size(px(TEXT_SM))
                        .text_color(accent_blue_text()),
                    )
            },
        ))
}

pub(crate) fn summary_stat_tile(
    label: impl IntoReactive<SharedString>,
    value: impl Fn() -> String + 'static,
    sub_value: impl Fn() -> Option<String> + 'static,
    grow: f32,
    min_w: f32,
) -> impl IntoElement {
    let sub_read = Rc::new(sub_value);
    let sub_show = Rc::clone(&sub_read);
    let sub_text = Rc::clone(&sub_read);
    view()
        .flex_grow(grow)
        .flex_shrink(1.0)
        .basis(px(min_w))
        .min_w(px(min_w))
        .px_3()
        .py_2()
        .gap_0p5()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border_subtle())
        .bg(bg_code())
        .child(
            text(label)
                .number_of_lines(1)
                .text_size(px(TEXT_XS))
                .text_color(fg_muted()),
        )
        .child(
            view()
                .w_full()
                .flex_row()
                .flex_wrap()
                .items_baseline()
                .gap_x_2()
                .gap_y_0p5()
                .child(
                    mono(value)
                        .tabular_nums()
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .text_size(px(TEXT_MD))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg_primary()),
                )
                .child(when(
                    move || sub_show().is_some(),
                    move || {
                        let sub = Rc::clone(&sub_text);
                        mono(move || sub().unwrap_or_default())
                            .tabular_nums()
                            .number_of_lines(1)
                            .text_size(px(TEXT_XS))
                            .text_color(fg_muted())
                    },
                )),
        )
}

pub(crate) fn status_count_badge(
    tag: &'static str,
    count: impl Fn() -> usize + 'static,
    bg_color: AdaptiveColor,
    fg_color: AdaptiveColor,
) -> impl IntoElement {
    view()
        .h(px(22.0))
        .px_2()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(RADIUS_SM))
        .bg(bg_color)
        .child(
            mono(move || format!("{tag} {}", present::format_thousands(count() as u64)))
                .tabular_nums()
                .number_of_lines(1)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_color),
        )
}
