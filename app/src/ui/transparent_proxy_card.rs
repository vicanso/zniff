#[allow(unused_imports)]
use super::*;

use crate::transparent_proxy::{self, RunningApp, TpState, TransparentProxyStatus};

#[derive(Clone, Copy)]
struct TpCardState {
    tp_status: Signal<TransparentProxyStatus>,
    tp_apps: Signal<Vec<RunningApp>>,
    expanded: Signal<bool>,
    capture_all: Signal<bool>,
    targets: Signal<String>,
    block_quic: Signal<bool>,
    needs_target_prompt: Signal<bool>,
}

fn tp_has_no_effective_targets(capture_all: bool, raw_targets: &str) -> bool {
    !capture_all && transparent_proxy::parse_targets(raw_targets).is_empty()
}

fn tp_shows_empty_target_notice(
    st: &TransparentProxyStatus,
    no_targets: bool,
    prompt: bool,
) -> bool {
    st.in_applications
        && st.error.is_empty()
        && no_targets
        && (st.state == TpState::Running || (!st.is_on_or_starting() && prompt))
}

/// Single state-transition gateway for toggling Transparent Proxy / Linux eBPF.
fn handle_tp_toggle_press(controls: &Arc<Controls>, state: TpCardState) {
    let st = state.tp_status.get_untracked();
    if !st.is_on_or_starting() {
        if !st.in_applications {
            state.expanded.set(true);
            toggle_transparent_proxy(controls.clone());
            return;
        }
        if tp_has_no_effective_targets(
            state.capture_all.get_untracked(),
            &state.targets.get_untracked(),
        ) {
            state.needs_target_prompt.set(true);
            state.expanded.set(true);
            refresh_transparent_proxy_state_async();
            set_status(controls, StatusMsg::TransparentProxyNeedsTargets);
            return;
        }
    }
    state.needs_target_prompt.set(false);
    toggle_transparent_proxy(controls.clone());
}

/// Expandable Connect Hub card for macOS `NETransparentProxyProvider` / Linux eBPF capture.
pub(crate) fn transparent_proxy_card(
    controls: Arc<Controls>,
    tp_status: Signal<TransparentProxyStatus>,
    tp_apps: Signal<Vec<RunningApp>>,
    open_popover: Signal<bool>,
) -> impl IntoElement {
    let saved = saved_config();
    let state = TpCardState {
        tp_status,
        tp_apps,
        expanded: signal(false),
        capture_all: signal(saved.tp_capture_all),
        targets: signal(saved.tp_targets),
        block_quic: signal(saved.tp_block_quic),
        needs_target_prompt: signal(false),
    };

    view()
        .w_full()
        .flex_col()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(move || {
            if state.tp_status.with(|st| st.is_on_or_starting()) {
                accent_blue_border()
            } else {
                border_subtle()
            }
        })
        .bg(bg_code())
        .overflow_hidden()
        .child(tp_header_row(controls.clone(), state, open_popover))
        .child(tp_inline_notice(state))
        .child(when(
            move || state.expanded.get(),
            move || tp_expanded_body(controls.clone(), state),
        ))
}

fn tp_header_row(
    controls: Arc<Controls>,
    state: TpCardState,
    open_popover: Signal<bool>,
) -> impl IntoElement {
    let langs = ui_langs();
    let toggle_controls = controls;
    button()
        .w_full()
        .px_2p5()
        .py_1p5()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_1p5()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .on_press(move |_: &()| {
            let next = !state.expanded.get_untracked();
            state.expanded.set(next);
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
                        color(if state.tp_status.with(|st| st.is_on_or_starting()) {
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
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg_primary()),
                        )
                        .child(
                            text(move || -> SharedString {
                                let no_targets = tp_has_no_effective_targets(
                                    state.capture_all.get(),
                                    &state.targets.get(),
                                );
                                let prompt = state.needs_target_prompt.get();
                                tp_status_subtitle(
                                    ui_lang(langs),
                                    &state.tp_status.get(),
                                    no_targets,
                                    prompt,
                                )
                                .into()
                            })
                            .number_of_lines(1)
                            .text_overflow(EllipsizeMode::Tail)
                            .text_size(px(TEXT_XS))
                            .text_color(move || {
                                let no_targets = tp_has_no_effective_targets(
                                    state.capture_all.get(),
                                    &state.targets.get(),
                                );
                                let prompt = state.needs_target_prompt.get();
                                state.tp_status.with(|st| {
                                    if !st.in_applications
                                        || matches!(
                                            st.state,
                                            TpState::NeedsApproval | TpState::Error
                                        )
                                        || (st.state == TpState::Running && no_targets)
                                        || (!st.is_on_or_starting() && prompt && no_targets)
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
                        SharedString::from(if state.tp_status.with(|st| st.is_on_or_starting()) {
                            t(lang, "停止", "Stop")
                        } else {
                            t(lang, "开启", "Start")
                        })
                    },
                    move || state.tp_status.with(|st| st.is_on_or_starting()),
                    move || {
                        let _ = open_popover;
                        handle_tp_toggle_press(&toggle_controls, state);
                    },
                ))
                .child(glyph_px(
                    11.0,
                    move || {
                        if state.expanded.get() {
                            Glyph::ChevronUp
                        } else {
                            Glyph::Chevron
                        }
                    },
                    || color(fg_muted()),
                )),
        )
}

fn tp_status_subtitle(
    lang: Lang,
    st: &TransparentProxyStatus,
    no_effective_targets: bool,
    needs_target_prompt: bool,
) -> String {
    if !st.in_applications {
        #[cfg(target_os = "linux")]
        return t(
            lang,
            "需要 Linux 5.12+ 与 cgroup v2",
            "Requires Linux 5.12+ and cgroup v2",
        )
        .to_string();
        #[cfg(not(target_os = "linux"))]
        return t(
            lang,
            "需移至 /Applications 才能激活",
            "Move to /Applications to activate",
        )
        .to_string();
    }
    match st.state {
        TpState::Running => {
            if no_effective_targets {
                t(
                    lang,
                    "运行中 · 未选择应用（暂不拦截）",
                    "Running · No app selected (idle)",
                )
                .into()
            } else {
                t(
                    lang,
                    "运行中 · 不改系统代理",
                    "Running · No OS proxy change",
                )
                .into()
            }
        }
        TpState::Starting => {
            #[cfg(target_os = "linux")]
            {
                t(
                    lang,
                    "正在启动 eBPF 助手（可能弹出授权）…",
                    "Starting eBPF helper (auth prompt)…",
                )
                .into()
            }
            #[cfg(not(target_os = "linux"))]
            {
                t(lang, "正在启动系统扩展…", "Starting extension…").into()
            }
        }
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
        TpState::Idle => {
            if needs_target_prompt && no_effective_targets {
                t(
                    lang,
                    "请先选择目标应用或切为「全部应用」",
                    "Select target apps or switch to All Apps",
                )
                .into()
            } else {
                t(
                    lang,
                    "按应用直连抓包，不改系统代理",
                    "Per-app capture, no OS proxy",
                )
                .into()
            }
        }
    }
}

fn tp_inline_notice_text(lang: Lang, state: TpCardState) -> SharedString {
    let no_targets = tp_has_no_effective_targets(state.capture_all.get(), &state.targets.get());
    let prompt = state.needs_target_prompt.get();
    state.tp_status.with(|st| {
        if !st.in_applications {
            #[cfg(target_os = "linux")]
            return t(
                lang,
                "需要 Linux 5.12+ 内核并挂载 cgroup v2（/sys/fs/cgroup/cgroup.controllers）才能启用 eBPF 透明代理。",
                "Requires Linux 5.12+ kernel and cgroup v2 (/sys/fs/cgroup/cgroup.controllers) for eBPF transparent proxy.",
            )
            .into();
            #[cfg(not(target_os = "linux"))]
            return t(
                lang,
                "请先将 zniff.app 移至 /Applications 目录再激活系统扩展。",
                "Move zniff.app into /Applications first to activate the system extension.",
            )
            .into();
        }
        if st.state == TpState::NeedsApproval {
            return t(
                lang,
                "请在系统设置 → 隐私与安全性 / 登录项与扩展中点击「允许」以启用 zniff 网络扩展。",
                "Open System Settings → Privacy & Security / Login Items & Extensions and click Allow.",
            )
            .into();
        }
        if !st.error.is_empty() {
            return st.error.clone().into();
        }
        if st.state == TpState::Running && no_targets {
            return t(
                lang,
                "当前为「指定应用」但未选择任何应用，暂不会拦截流量；请在下方点选应用或切换为「全部应用」。",
                "Selected Apps mode has no apps chosen, so no traffic is intercepted yet. Pick an app below or switch to All Apps.",
            )
            .into();
        }
        if !st.is_on_or_starting() && prompt && no_targets {
            return t(
                lang,
                "请先在下方点选或输入至少一个目标应用（或切换为「全部应用」），再点击「开启」。",
                "Pick or enter at least one target app below (or switch to All Apps), then click Start.",
            )
            .into();
        }
        SharedString::from("")
    })
}

fn tp_inline_notice(state: TpCardState) -> impl IntoElement {
    when(
        move || {
            let no_targets =
                tp_has_no_effective_targets(state.capture_all.get(), &state.targets.get());
            let prompt = state.needs_target_prompt.get();
            state.tp_status.with(|st| {
                !st.in_applications
                    || st.state == TpState::NeedsApproval
                    || !st.error.is_empty()
                    || tp_shows_empty_target_notice(st, no_targets, prompt)
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
                    text(move || tp_inline_notice_text(ui_lang(langs), state))
                        .text_size(px(TEXT_XS))
                        .line_height(Points(15.0))
                        .text_color(status_warn_fg()),
                )
                .child(when(
                    move || {
                        cfg!(target_os = "macos")
                            && state
                                .tp_status
                                .with(|st| st.state == TpState::NeedsApproval)
                    },
                    move || {
                        view()
                            .w_full()
                            .flex_row()
                            .justify_end()
                            .child(menu_inline_pill(
                                tr("打开系统设置", "Open System Settings"),
                                || true,
                                transparent_proxy::open_system_settings,
                            ))
                    },
                ))
                .child(when(
                    move || {
                        state
                            .tp_status
                            .with(|st| st.state == TpState::Error && !st.setcap_command.is_empty())
                    },
                    move || {
                        let copied = signal(false);
                        view()
                            .w_full()
                            .flex_row()
                            .justify_end()
                            .child(menu_inline_pill(
                                move || {
                                    let lang = ui_lang(langs);
                                    SharedString::from(if copied.get() {
                                        t(lang, "已复制", "Copied")
                                    } else {
                                        t(lang, "复制免密命令", "Copy setcap cmd")
                                    })
                                },
                                || true,
                                move || {
                                    let cmd = state
                                        .tp_status
                                        .with_untracked(|st| st.setcap_command.clone());
                                    if !cmd.is_empty() {
                                        set_clipboard(cmd);
                                        copied.set(true);
                                    }
                                },
                            ))
                    },
                ))
        },
    )
}

fn tp_scope_row(controls: Arc<Controls>, state: TpCardState) -> impl IntoElement {
    let mode_controls_sel = controls.clone();
    let mode_controls_all = controls;
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
                    move || !state.capture_all.get(),
                    move || {
                        set_tp_capture_all(&mode_controls_sel, state, false);
                    },
                ))
                .child(menu_inline_pill(
                    tr("全部应用", "All Apps"),
                    move || state.capture_all.get(),
                    move || {
                        set_tp_capture_all(&mode_controls_all, state, true);
                    },
                )),
        )
}

fn tp_expanded_body(controls: Arc<Controls>, state: TpCardState) -> impl IntoElement {
    let scope_controls = controls.clone();
    let target_controls = controls.clone();
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
        .child(tp_scope_row(scope_controls, state))
        .child(when(
            move || !state.capture_all.get(),
            move || tp_targets_section(target_controls.clone(), state),
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
                        "阻止 UDP 443（HTTP/3 → HTTPS）",
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
                        SharedString::from(if state.block_quic.get() {
                            t(lang, "已开启", "On")
                        } else {
                            t(lang, "已关闭", "Off")
                        })
                    },
                    move || state.block_quic.get(),
                    move || {
                        let next = !state.block_quic.get_untracked();
                        state.block_quic.set(next);
                        save_config_or_notify(&quic_controls, PrefName::TransparentProxy, |cfg| {
                            cfg.tp_block_quic = next;
                        });
                        sync_transparent_proxy_config(&quic_controls);
                    },
                )),
        )
        .child({
            #[cfg(target_os = "linux")]
            let note = tr(
                "基于 eBPF（cgroup v2）按进程拦截，自动排除 zniff 与系统 DNS 服务；请求流直接附带来源进程名。",
                "Per-process eBPF (cgroup v2) capture, excludes zniff and DNS services; attaches source app names.",
            );
            #[cfg(not(target_os = "linux"))]
            let note = tr(
                "自动排除 zniff 自身与系统核心守护进程；请求流会直接附带来源应用名。",
                "Automatically excludes zniff and core macOS daemons; attaches source app names.",
            );
            text(note)
                .text_size(px(TEXT_XS))
                .line_height(Points(14.0))
                .text_color(fg_muted())
        })
        .child(when(
            move || {
                state.tp_status.with(|st| {
                    st.in_applications && (st.passwordless_ready || !st.setcap_command.is_empty())
                })
            },
            move || tp_linux_auth_section(state.tp_status),
        ))
}

fn tp_linux_auth_section(tp_status: Signal<TransparentProxyStatus>) -> impl IntoElement {
    let langs = ui_langs();
    let copied = signal(false);
    view()
        .w_full()
        .flex_col()
        .gap_1()
        .child(when(
            move || tp_status.with(|st| st.passwordless_ready),
            move || {
                text(tr(
                    "免密授权已就绪（已授予 eBPF capability，无需每次输入密码）",
                    "Passwordless ready (eBPF capabilities granted)",
                ))
                .text_size(px(TEXT_XS))
                .line_height(Points(14.0))
                .text_color(status_ok_fg())
            },
        ))
        .child(when(
            move || tp_status.with(|st| !st.passwordless_ready && !st.setcap_command.is_empty()),
            move || {
                view()
                    .w_full()
                    .flex_col()
                    .gap_1()
                    .p_1p5()
                    .rounded(px(RADIUS_MD))
                    .bg(bg_card())
                    .border_1()
                    .border_color(border_subtle())
                    .child(
                        view()
                            .w_full()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_1()
                            .child(
                                text(tr(
                                    "免密启动（可选，避免每次 pkexec 弹窗）",
                                    "Passwordless start (optional, skips pkexec prompt)",
                                ))
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Tail)
                                .text_size(px(TEXT_XS))
                                .text_color(fg_secondary()),
                            )
                            .child(menu_inline_pill(
                                move || {
                                    let lang = ui_lang(langs);
                                    SharedString::from(if copied.get() {
                                        t(lang, "已复制", "Copied")
                                    } else {
                                        t(lang, "复制命令", "Copy")
                                    })
                                },
                                || false,
                                move || {
                                    let cmd =
                                        tp_status.with_untracked(|st| st.setcap_command.clone());
                                    if !cmd.is_empty() {
                                        set_clipboard(cmd);
                                        copied.set(true);
                                    }
                                },
                            )),
                    )
                    .child(
                        mono(move || -> SharedString {
                            tp_status.with(|st| st.setcap_command.clone()).into()
                        })
                        .text_size(px(TEXT_XS))
                        .text_color(fg_primary()),
                    )
            },
        ))
}

fn tp_targets_section(controls: Arc<Controls>, state: TpCardState) -> impl IntoElement {
    let chip_controls = controls.clone();
    let input_controls = controls;
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
                    .text_size(px(TEXT_XS))
                    .text_color(fg_muted()),
                )
                .child(menu_inline_pill(
                    tr("刷新", "Refresh"),
                    || false,
                    refresh_transparent_proxy_state_async,
                )),
        )
        .child(view().w_full().flex_row().flex_wrap().gap_1().child(each(
            move || state.tp_apps.get(),
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
                        let raw = state.targets.get();
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
                                &state.targets.get_untracked(),
                                &a.name,
                                &[&a.name, &a.bundle_id, &a.exec_name],
                            )
                        });
                        set_tp_targets(&controls, state, &next);
                    },
                )
            },
        )))
        .child(
            ui_input_a11y(
                input(),
                tr("透明抓包目标进程", "Transparent proxy target apps"),
            )
            .value(state.targets)
            .placeholder({
                #[cfg(target_os = "linux")]
                let ph = tr(
                    "进程名、可执行名或 PID，逗号分隔（如 firefox, curl, node）",
                    "Process names or PIDs, comma-separated (e.g. firefox, curl, node)",
                );
                #[cfg(not(target_os = "linux"))]
                let ph = tr(
                    "进程名或 Bundle ID，逗号分隔（如 Safari, curl, node）",
                    "Process names or bundle IDs, comma-separated (e.g. Safari, curl)",
                );
                ph
            })
            .placeholder_color(move || color(fg_muted()))
            .w_full()
            .h(px(28.0))
            .px_2()
            .rounded(px(6.0))
            .bg(bg_card())
            .border_1()
            .border_color(move || {
                let empty = tp_has_no_effective_targets(false, &state.targets.get());
                let warn = empty
                    && (state.needs_target_prompt.get()
                        || state.tp_status.with(|st| st.state == TpState::Running));
                if warn {
                    status_warn_fg()
                } else {
                    border_control()
                }
            })
            .focus(|style| style.border_color(accent_blue()))
            .text_size(px(11.5))
            .text_color(fg_primary())
            .on_change(move |text: &str| {
                set_tp_targets(&input_controls, state, text);
            }),
        )
}

fn set_tp_capture_all(controls: &Controls, state: TpCardState, next: bool) {
    state.capture_all.set(next);
    if next {
        state.needs_target_prompt.set(false);
    }
    save_config_or_notify(controls, PrefName::TransparentProxy, |cfg| {
        cfg.tp_capture_all = next;
    });
    sync_transparent_proxy_config(controls);
}

fn set_tp_targets(controls: &Controls, state: TpCardState, raw: &str) {
    state.targets.set(raw.to_string());
    if !transparent_proxy::parse_targets(raw).is_empty() {
        state.needs_target_prompt.set(false);
    }
    let stored = raw.trim().to_string();
    save_config_or_notify(controls, PrefName::TransparentProxy, |cfg| {
        cfg.tp_targets = stored;
    });
    sync_transparent_proxy_config(controls);
}
