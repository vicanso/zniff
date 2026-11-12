#[allow(unused_imports)]
use super::*;

pub(crate) fn phase_color_dns() -> AdaptiveColor {
    AdaptiveColor::rgb(0x0284c7, 0x38bdf8)
}

pub(crate) fn phase_color_tcp() -> AdaptiveColor {
    AdaptiveColor::rgb(0xd97706, 0xfbbf24)
}

pub(crate) fn phase_color_tls() -> AdaptiveColor {
    AdaptiveColor::rgb(0x7c3aed, 0xa78bfa)
}

pub(crate) fn phase_color_wait() -> AdaptiveColor {
    AdaptiveColor::rgb(0x059669, 0x34d399)
}

pub(crate) fn phase_color_download() -> AdaptiveColor {
    AdaptiveColor::rgb(0x2563eb, 0x60a5fa)
}

pub(crate) fn summary_timing_bar(timing: Signal<present::TimingBreakdown>) -> impl IntoElement {
    view()
        .px_2p5()
        .py_2()
        .gap_1p5()
        .rounded(px(7.0))
        .bg(bg_code())
        .border_1()
        .border_color(border_subtle())
        .child(
            view()
                .w_full()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_1p5()
                        .child(when(
                            move || timing.with(|tb| tb.reused),
                            || {
                                view()
                                    .px_1p5()
                                    .py(px(1.5))
                                    .rounded(px(4.0))
                                    .bg(status_ok_bg())
                                    .child(
                                        text(tr("复用连接", "Reused"))
                                            .text_size(px(10.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(status_ok_fg()),
                                    )
                            },
                        ))
                        .child(when(
                            move || {
                                timing.with(|tb| {
                                    !tb.reused
                                        && (tb.dns_ms.is_some()
                                            || tb.tcp_ms.is_some()
                                            || tb.tls_ms.is_some())
                                })
                            },
                            || {
                                view()
                                    .px_1p5()
                                    .py(px(1.5))
                                    .rounded(px(4.0))
                                    .bg(accent_blue_soft())
                                    .child(
                                        text(tr("新建连接", "New conn"))
                                            .text_size(px(10.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(accent_blue_text()),
                                    )
                            },
                        ))
                        .child(when(
                            move || timing.with(|tb| !tb.tls_version.is_empty()),
                            move || {
                                view()
                                    .px_1p5()
                                    .py(px(1.5))
                                    .rounded(px(4.0))
                                    .bg(bg_control())
                                    .child(
                                        mono(move || timing.with(|tb| tb.tls_version.clone()))
                                            .text_size(px(10.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(fg_secondary()),
                                    )
                            },
                        ))
                        .child(when(
                            move || timing.with(|tb| !tb.client_cert.is_empty()),
                            move || {
                                view()
                                    .px_1p5()
                                    .py(px(1.5))
                                    .rounded(px(4.0))
                                    .bg(accent_blue_soft())
                                    .tooltip(
                                        tooltip().child(
                                            view()
                                                .px_2()
                                                .py_1()
                                                .rounded(px(6.0))
                                                .bg(bg_card())
                                                .border_1()
                                                .border_color(border_strong())
                                                .child(
                                                    mono(move || {
                                                        timing.with(|tb| tb.client_cert.clone())
                                                    })
                                                    .text_size(px(11.5))
                                                    .text_color(fg_primary()),
                                                ),
                                        ),
                                    )
                                    .child(
                                        mono(move || {
                                            let path = timing.with(|tb| tb.client_cert.clone());
                                            format!("mTLS {}", cert_display_name(&path))
                                        })
                                        .selectable(true)
                                        .text_size(px(10.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(accent_blue_text()),
                                    )
                            },
                        ))
                        .child(when(
                            move || timing.with(|tb| !tb.server_addr.is_empty()),
                            move || {
                                view()
                                    .px_1p5()
                                    .py(px(1.5))
                                    .rounded(px(4.0))
                                    .bg(bg_control())
                                    .child(
                                        mono({
                                            let langs = ui_langs();
                                            move || {
                                                let addr = timing.with(|tb| tb.server_addr.clone());
                                                format!(
                                                    "{} {addr}",
                                                    t(ui_lang(langs), "远端", "Peer")
                                                )
                                            }
                                        })
                                        .selectable(true)
                                        .text_size(px(10.5))
                                        .text_color(fg_muted()),
                                    )
                            },
                        )),
                )
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_2p5()
                        .child(timing_phase_chip(
                            "DNS",
                            "DNS",
                            phase_color_dns(),
                            move || timing.with(|tb| tb.dns_ms),
                        ))
                        .child(timing_phase_chip(
                            "TCP",
                            "TCP",
                            phase_color_tcp(),
                            move || timing.with(|tb| tb.tcp_ms),
                        ))
                        .child(timing_phase_chip(
                            "TLS",
                            "TLS",
                            phase_color_tls(),
                            move || timing.with(|tb| tb.tls_ms),
                        ))
                        .child(timing_phase_chip(
                            "等待",
                            "Wait",
                            phase_color_wait(),
                            move || timing.with(|tb| tb.wait_ms),
                        ))
                        .child(timing_phase_chip(
                            "下载",
                            "Download",
                            phase_color_download(),
                            move || timing.with(|tb| tb.download_ms),
                        )),
                ),
        )
        .child(when(
            move || timing.with(present::TimingBreakdown::has_phases),
            move || {
                view()
                    .w_full()
                    .h(px(5.0))
                    .rounded(px(2.5))
                    .bg(bg_control())
                    .overflow_hidden()
                    .flex_row()
                    .gap(px(1.0))
                    .child(timing_phase_segment(phase_color_dns(), move || {
                        timing.with(|tb| tb.dns_ms)
                    }))
                    .child(timing_phase_segment(phase_color_tcp(), move || {
                        timing.with(|tb| tb.tcp_ms)
                    }))
                    .child(timing_phase_segment(phase_color_tls(), move || {
                        timing.with(|tb| tb.tls_ms)
                    }))
                    .child(timing_phase_segment(phase_color_wait(), move || {
                        timing.with(|tb| tb.wait_ms)
                    }))
                    .child(timing_phase_segment(phase_color_download(), move || {
                        timing.with(|tb| tb.download_ms)
                    }))
            },
        ))
}

pub(crate) fn timing_phase_chip(
    zh: &'static str,
    en: &'static str,
    color: AdaptiveColor,
    get_ms: impl Fn() -> Option<u64> + Copy + 'static,
) -> impl IntoElement {
    when(
        move || get_ms().is_some(),
        move || {
            let langs = ui_langs();
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(view().w(px(6.5)).h(px(6.5)).rounded_full().bg(color))
                .child(
                    mono(move || {
                        let ms = get_ms().unwrap_or(0);
                        format!("{} {ms} ms", t(ui_lang(langs), zh, en))
                    })
                    .tabular_nums()
                    .text_size(px(11.0))
                    .text_color(fg_secondary()),
                )
        },
    )
}

pub(crate) fn timing_phase_segment(
    color: AdaptiveColor,
    get_ms: impl Fn() -> Option<u64> + Copy + 'static,
) -> impl IntoElement {
    when(
        move || get_ms().is_some(),
        move || {
            view()
                .h_full()
                .min_w(px(3.0))
                .flex_grow(move || (get_ms().unwrap_or(0).max(1)) as f32)
                .bg(color)
        },
    )
}

pub(crate) fn repeat_inline_bar(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
) -> impl IntoElement {
    let submit_count_controls = controls.clone();
    let submit_interval_controls = controls.clone();
    let submit_concurrency_controls = controls.clone();
    let run_controls = controls;
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_1p5()
        .px_3()
        .py_2()
        .rounded(px(7.0))
        .bg(bg_code())
        .border_1()
        .border_color(border_subtle())
        .child(
            text(tr("次数", "Count"))
                .number_of_lines(1)
                .text_size(px(11.5))
                .text_color(fg_secondary()),
        )
        .child(
            view().w(px(56.0)).flex_shrink_0().child(
                compose_input_on(shown.repeat_count, "5", 26.0, false, move |text| {
                    shown.repeat_count.set(text.to_string())
                })
                .on_submit(move |text: &str| {
                    shown.repeat_count.set(text.to_string());
                    repeat_flow_batch(&submit_count_controls, selected.get(), shown);
                }),
            ),
        )
        .child(repeat_preset_chip(shown.repeat_count, "5", "5×"))
        .child(repeat_preset_chip(shown.repeat_count, "10", "10×"))
        .child(repeat_preset_chip(shown.repeat_count, "20", "20×"))
        .child(
            text(tr("并发", "Concurrency"))
                .pl_1()
                .number_of_lines(1)
                .text_size(px(11.5))
                .text_color(fg_secondary()),
        )
        .child(
            view().w(px(48.0)).flex_shrink_0().child(
                compose_input_on(shown.repeat_concurrency, "1", 26.0, false, move |text| {
                    shown.repeat_concurrency.set(text.to_string())
                })
                .on_submit(move |text: &str| {
                    shown.repeat_concurrency.set(text.to_string());
                    repeat_flow_batch(&submit_concurrency_controls, selected.get(), shown);
                }),
            ),
        )
        .child(
            text(tr("间隔 ms", "Interval (ms)"))
                .pl_1()
                .number_of_lines(1)
                .text_size(px(11.5))
                .text_color(fg_secondary()),
        )
        .child(
            view().w(px(68.0)).flex_shrink_0().child(
                compose_input_on(shown.repeat_interval, "200", 26.0, false, move |text| {
                    shown.repeat_interval.set(text.to_string())
                })
                .on_submit(move |text: &str| {
                    shown.repeat_interval.set(text.to_string());
                    repeat_flow_batch(&submit_interval_controls, selected.get(), shown);
                }),
            ),
        )
        .child(view().flex_1())
        .child(
            view()
                .h(px(26.0))
                .px_2p5()
                .rounded(px(6.0))
                .bg(accent_blue())
                .hover(|style| style.bg(accent_blue_hover()))
                .active(|style| style.opacity(0.85))
                .cursor(Cursor::Pointer)
                .flex_row()
                .items_center()
                .on_press(move || {
                    repeat_flow_batch(&run_controls, selected.get(), shown);
                })
                .child(
                    text(tr("开始重放", "Run"))
                        .number_of_lines(1)
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(white()),
                ),
        )
        .child(detail_button(tr("取消", "Cancel"), move || {
            shown.repeating.set(false);
        }))
}

pub(crate) fn repeat_preset_chip(
    count_sig: Signal<String>,
    val: &'static str,
    label: &'static str,
) -> impl IntoElement {
    let is_active = move || count_sig.get().trim() == val;
    view()
        .h(px(24.0))
        .px_2()
        .rounded(px(5.0))
        .border_1()
        .border_color(move || {
            if is_active() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if is_active() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if is_active() {
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
        .on_press(move || count_sig.set(val.to_string()))
        .child(
            mono(label)
                .tabular_nums()
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || {
                    if is_active() {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
        )
}
