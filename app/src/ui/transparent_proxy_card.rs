#[allow(unused_imports)]
use super::*;

use crate::transparent_proxy::{self, RunningApp, TpState, TransparentProxyStatus};

/// Expandable Connect Hub card for macOS `NETransparentProxyProvider` per-process capture.
pub(crate) fn transparent_proxy_card(
    controls: Arc<Controls>,
    tp_status: Signal<TransparentProxyStatus>,
    tp_apps: Signal<Vec<RunningApp>>,
    open_popover: Signal<bool>,
) -> impl IntoElement {
    let saved = saved_config();
    let expanded = signal(false);
    let capture_all = signal(saved.tp_capture_all);
    let targets = signal(saved.tp_targets);
    let block_quic = signal(saved.tp_block_quic);

    view()
        .w_full()
        .flex_col()
        .rounded(px(7.0))
        .border_1()
        .border_color(move || {
            if tp_status.with(|st| st.is_on_or_starting()) {
                accent_blue_border()
            } else {
                border_subtle()
            }
        })
        .bg(bg_code())
        .overflow_hidden()
        .child(tp_header_row(
            controls.clone(),
            tp_status,
            expanded,
            open_popover,
        ))
        .child(tp_inline_notice(tp_status))
        .child(when(
            move || expanded.get(),
            move || tp_expanded_body(controls.clone(), tp_apps, capture_all, targets, block_quic),
        ))
}

fn tp_header_row(
    controls: Arc<Controls>,
    tp_status: Signal<TransparentProxyStatus>,
    expanded: Signal<bool>,
    open_popover: Signal<bool>,
) -> impl IntoElement {
    let langs = ui_langs();
    let toggle_controls = controls;
    view()
        .w_full()
        .px_2p5()
        .py_1p5()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_1p5()
        .cursor(Cursor::Pointer)
        .hover(|style| style.bg(bg_control_hover()))
        .on_press(move || {
            let next = !expanded.get_untracked();
            expanded.set(next);
            if next {
                refresh_transparent_proxy_state_async();
            }
        })
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(glyph_px(
                    12.0,
                    || Glyph::Shield,
                    move || {
                        color(if tp_status.with(|st| st.is_on_or_starting()) {
                            accent_blue_text()
                        } else {
                            fg_secondary()
                        })
                    },
                ))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex_col()
                        .gap(px(1.0))
                        .child(
                            text(tr("透明按进程抓包", "Transparent Proxy"))
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Tail)
                                .text_size(px(12.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg_primary()),
                        )
                        .child(
                            text(move || -> SharedString {
                                tp_status_subtitle(ui_lang(langs), &tp_status.get()).into()
                            })
                            .number_of_lines(1)
                            .text_overflow(EllipsizeMode::Tail)
                            .text_size(px(10.5))
                            .text_color(move || {
                                tp_status.with(|st| {
                                    if !st.in_applications
                                        || matches!(
                                            st.state,
                                            TpState::NeedsApproval | TpState::Error
                                        )
                                    {
                                        status_warn_fg()
                                    } else if st.is_on_or_starting() {
                                        status_ok_fg()
                                    } else {
                                        fg_muted()
                                    }
                                })
                            }),
                        ),
                ),
        )
        .child(
            view()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_1()
                .child(menu_inline_pill(
                    move || {
                        let lang = ui_lang(langs);
                        SharedString::from(if tp_status.with(|st| st.is_on_or_starting()) {
                            t(lang, "停止", "Stop")
                        } else {
                            t(lang, "开启", "Start")
                        })
                    },
                    move || tp_status.with(|st| st.is_on_or_starting()),
                    move || {
                        let st = tp_status.get_untracked();
                        if !st.in_applications && !st.is_on_or_starting() {
                            expanded.set(true);
                        }
                        let _ = open_popover;
                        toggle_transparent_proxy(toggle_controls.clone());
                    },
                ))
                .child(glyph_px(
                    11.0,
                    move || {
                        if expanded.get() {
                            Glyph::ChevronUp
                        } else {
                            Glyph::Chevron
                        }
                    },
                    || color(fg_muted()),
                )),
        )
}

fn tp_status_subtitle(lang: Lang, st: &TransparentProxyStatus) -> String {
    if !st.in_applications {
        return t(
            lang,
            "需移至 /Applications 才能激活",
            "Move to /Applications to activate",
        )
        .to_string();
    }
    match st.state {
        TpState::Running => t(
            lang,
            "运行中 · 不改系统代理",
            "Running · No OS proxy change",
        )
        .into(),
        TpState::Starting => t(lang, "正在启动系统扩展…", "Starting extension…").into(),
        TpState::Stopping => t(lang, "正在停止…", "Stopping…").into(),
        TpState::NeedsApproval => {
            t(lang, "等待系统设置授权", "Needs System Settings approval").into()
        }
        TpState::Error => {
            if st.error.is_empty() {
                t(lang, "激活或运行出错", "Activation or runtime error").into()
            } else {
                st.error.clone()
            }
        }
        TpState::Idle => t(
            lang,
            "按应用直连抓包，不改系统代理",
            "Per-app capture, no OS proxy",
        )
        .into(),
    }
}

fn tp_inline_notice(tp_status: Signal<TransparentProxyStatus>) -> impl IntoElement {
    when(
        move || {
            tp_status.with(|st| {
                !st.in_applications || st.state == TpState::NeedsApproval || !st.error.is_empty()
            })
        },
        move || {
            let langs = ui_langs();
            view()
                .w_full()
                .px_2p5()
                .py_1p5()
                .gap_1p5()
                .flex_col()
                .border_t_1()
                .border_color(border_subtle())
                .bg(status_warn_bg())
                .child(
                    text(move || -> SharedString {
                        let lang = ui_lang(langs);
                        tp_status.with(|st| {
                            if !st.in_applications {
                                t(
                                    lang,
                                    "请先将 zniff.app 移至 /Applications 目录再激活系统扩展。",
                                    "Move zniff.app into /Applications first to activate the system extension.",
                                )
                                .into()
                            } else if st.state == TpState::NeedsApproval {
                                t(
                                    lang,
                                    "请在系统设置 → 隐私与安全性 / 登录项与扩展中点击「允许」以启用 zniff 网络扩展。",
                                    "Open System Settings → Privacy & Security / Login Items & Extensions and click Allow.",
                                )
                                .into()
                            } else {
                                st.error.clone().into()
                            }
                        })
                    })
                    .text_size(px(10.5))
                    .line_height(Points(15.0))
                    .text_color(status_warn_fg()),
                )
                .child(when(
                    move || tp_status.with(|st| st.state == TpState::NeedsApproval),
                    move || {
                        view().w_full().flex_row().justify_end().child(menu_inline_pill(
                            tr("打开系统设置", "Open System Settings"),
                            || true,
                            transparent_proxy::open_system_settings,
                        ))
                    },
                ))
        },
    )
}

fn tp_expanded_body(
    controls: Arc<Controls>,
    tp_apps: Signal<Vec<RunningApp>>,
    capture_all: Signal<bool>,
    targets: Signal<String>,
    block_quic: Signal<bool>,
) -> impl IntoElement {
    let mode_controls_sel = controls.clone();
    let mode_controls_all = controls.clone();
    let input_controls = controls.clone();
    let chip_controls = controls.clone();
    let quic_controls = controls;
    let langs = ui_langs();

    view()
        .w_full()
        .px_2p5()
        .pb_2p5()
        .pt_2()
        .gap_2()
        .flex_col()
        .border_t_1()
        .border_color(border_subtle())
        .child(
            view()
                .w_full()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_1p5()
                .child(
                    text(tr("拦截范围", "Capture Scope"))
                        .text_size(px(11.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg_secondary()),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(menu_inline_pill(
                            tr("指定应用", "Selected Apps"),
                            move || !capture_all.get(),
                            move || {
                                set_tp_capture_all(&mode_controls_sel, capture_all, false);
                            },
                        ))
                        .child(menu_inline_pill(
                            tr("全部应用", "All Apps"),
                            move || capture_all.get(),
                            move || {
                                set_tp_capture_all(&mode_controls_all, capture_all, true);
                            },
                        )),
                ),
        )
        .child(when(
            move || !capture_all.get(),
            move || {
                tp_targets_section(
                    chip_controls.clone(),
                    input_controls.clone(),
                    tp_apps,
                    targets,
                )
            },
        ))
        .child(
            view()
                .w_full()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_1p5()
                .child(
                    text(tr(
                        "阻止 UDP 443 (HTTP/3 → HTTPS)",
                        "Block UDP 443 (HTTP/3 → HTTPS)",
                    ))
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail)
                    .text_size(px(11.0))
                    .text_color(fg_secondary()),
                )
                .child(menu_inline_pill(
                    move || {
                        let lang = ui_lang(langs);
                        SharedString::from(if block_quic.get() {
                            t(lang, "已开启", "On")
                        } else {
                            t(lang, "已关闭", "Off")
                        })
                    },
                    move || block_quic.get(),
                    move || {
                        let next = !block_quic.get_untracked();
                        block_quic.set(next);
                        save_config_or_notify(&quic_controls, PrefName::TransparentProxy, |cfg| {
                            cfg.tp_block_quic = next;
                        });
                        sync_transparent_proxy_config(&quic_controls);
                    },
                )),
        )
        .child(
            text(tr(
                "自动排除 zniff 自身与系统核心守护进程；请求流会直接附带来源应用名。",
                "Automatically excludes zniff and core macOS daemons; attaches source app names.",
            ))
            .text_size(px(10.0))
            .line_height(Points(14.0))
            .text_color(fg_muted()),
        )
}

fn tp_targets_section(
    chip_controls: Arc<Controls>,
    input_controls: Arc<Controls>,
    tp_apps: Signal<Vec<RunningApp>>,
    targets: Signal<String>,
) -> impl IntoElement {
    view()
        .w_full()
        .gap_1p5()
        .flex_col()
        .child(
            view()
                .w_full()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    text(tr(
                        "运行中应用（点击快速加入）",
                        "Running Apps (click to toggle)",
                    ))
                    .text_size(px(10.5))
                    .text_color(fg_muted()),
                )
                .child(menu_inline_pill(
                    tr("刷新", "Refresh"),
                    || false,
                    refresh_transparent_proxy_state_async,
                )),
        )
        .child(view().w_full().flex_row().flex_wrap().gap_1().child(each(
            move || tp_apps.get(),
            |app| {
                if app.bundle_id.is_empty() {
                    app.name.clone()
                } else {
                    app.bundle_id.clone()
                }
            },
            move |app, _| {
                let controls = chip_controls.clone();
                menu_inline_pill(
                    move || SharedString::from(app.with(|a| a.name.clone())),
                    move || {
                        let raw = targets.get();
                        app.with(|a| {
                            transparent_proxy::has_target(&raw, &a.name)
                                || transparent_proxy::has_target(&raw, &a.exec_name)
                                || (!a.bundle_id.is_empty()
                                    && transparent_proxy::has_target(&raw, &a.bundle_id))
                        })
                    },
                    move || {
                        let next = app.with_untracked(|a| {
                            transparent_proxy::toggle_target_aliases(
                                &targets.get_untracked(),
                                &a.name,
                                &[&a.name, &a.bundle_id, &a.exec_name],
                            )
                        });
                        set_tp_targets(&controls, targets, &next);
                    },
                )
            },
        )))
        .child(
            input()
                .value(targets)
                .placeholder(tr(
                    "进程名或 Bundle ID，逗号分隔（如 Safari, curl, node）",
                    "Process names or bundle IDs, comma-separated (e.g. Safari, curl)",
                ))
                .placeholder_color(move || color(fg_muted()))
                .w_full()
                .h(px(28.0))
                .px_2()
                .rounded(px(6.0))
                .bg(bg_card())
                .border_1()
                .border_color(border_control())
                .focus(|style| style.border_color(accent_blue()))
                .text_size(px(11.5))
                .text_color(fg_primary())
                .on_change(move |text: &str| {
                    set_tp_targets(&input_controls, targets, text);
                }),
        )
}

fn set_tp_capture_all(controls: &Controls, capture_all: Signal<bool>, next: bool) {
    capture_all.set(next);
    save_config_or_notify(controls, PrefName::TransparentProxy, |cfg| {
        cfg.tp_capture_all = next;
    });
    sync_transparent_proxy_config(controls);
}

fn set_tp_targets(controls: &Controls, targets: Signal<String>, raw: &str) {
    targets.set(raw.to_string());
    let stored = raw.trim().to_string();
    save_config_or_notify(controls, PrefName::TransparentProxy, |cfg| {
        cfg.tp_targets = stored;
    });
    sync_transparent_proxy_config(controls);
}
