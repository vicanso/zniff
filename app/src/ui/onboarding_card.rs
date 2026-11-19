#[allow(unused_imports)]
use super::*;

#[derive(Clone)]
pub(crate) struct OnboardingProps {
    pub(crate) controls: Arc<Controls>,
    pub(crate) running: Signal<bool>,
    pub(crate) listen: Signal<String>,
    pub(crate) ca_ui: CaSignals,
    pub(crate) use_system_proxy: Signal<bool>,
    pub(crate) adb_count: Signal<usize>,
    pub(crate) flow_total: Signal<usize>,
    pub(crate) compose: Compose,
    pub(crate) selected: Signal<Option<u64>>,
    pub(crate) profiles: ProfileSignals,
    pub(crate) install_ca: Rc<dyn Fn()>,
    pub(crate) toggle_system_proxy: Rc<dyn Fn()>,
    pub(crate) qr_open: Signal<bool>,
    pub(crate) pending_qr: Signal<bool>,
}

/// Returns true when the right detail pane should show the Quick Start onboarding screen.
pub(crate) fn should_show_onboarding(phase: Phase, total_count: usize) -> bool {
    phase == Phase::Empty && total_count == 0
}

/// Returns `1` column when the detail pane narrows below 460pt, otherwise `2`.
pub(crate) fn onboarding_grid_cols(pane_width: f32) -> u16 {
    if pane_width > 0.0 && pane_width < 460.0 {
        1
    } else {
        2
    }
}

/// Frameless empty-state onboarding view rendered directly on the detail canvas.
pub(crate) fn onboarding_card(props: OnboardingProps, pane_width: Signal<f32>) -> impl IntoElement {
    let root = view()
        .w_full()
        .max_w(px(580.0))
        .flex_col()
        .gap_5()
        .child(onboarding_header(&props));

    #[cfg(not(target_arch = "wasm32"))]
    let root = root.child(onboarding_primary_section(&props, pane_width));

    #[cfg(target_arch = "wasm32")]
    let root = root.child(onboarding_web_section(&props, pane_width));

    root.child(onboarding_secondary_section(&props, pane_width))
        .child(onboarding_footer_hint())
}

fn onboarding_header(props: &OnboardingProps) -> impl IntoElement {
    view()
        .w_full()
        .flex_col()
        .gap_1()
        .child(
            view()
                .w_full()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text(tr("快速开始抓包", "Quick Start"))
                        .text_size(px(16.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg_primary()),
                )
                .child(onboarding_header_status(props)),
        )
        .child(
            text(tr(
                "选择免配入口即可捕获流量，无需修改全局代理设置。",
                "Pick a zero-config entry to capture traffic without touching global proxy settings.",
            ))
            .text_size(px(12.5))
            .text_color(fg_muted()),
        )
}

#[cfg(not(target_arch = "wasm32"))]
fn onboarding_header_status(props: &OnboardingProps) -> impl IntoElement {
    let controls = props.controls.clone();
    let running = props.running;
    let listen = props.listen;
    let ca_ui = props.ca_ui;
    let install_ca = props.install_ca.clone();
    let langs = ui_langs();

    when(
        move || running.get() && ca_ui.status.get() == ca::CaStatus::Trusted,
        move || {
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(
                    view()
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded_full()
                        .bg(status_ok_fg()),
                )
                .child(
                    mono(move || -> SharedString {
                        let lang = ui_lang(langs);
                        let addr = listen.get();
                        let ca_label = if let Some(ext) = ca_ui.active_external.get() {
                            match lang {
                                Lang::Zh => format!("已复用 {} 证书", ext.tool_name),
                                Lang::En => format!("Reusing {} CA", ext.tool_name),
                            }
                        } else {
                            t(lang, "根证书已信任", "CA trusted").to_string()
                        };
                        if addr.is_empty() {
                            ca_label.into()
                        } else {
                            format!("{addr} · {ca_label}").into()
                        }
                    })
                    .number_of_lines(1)
                    .text_size(px(11.5))
                    .text_color(fg_muted()),
                )
        },
    )
    .otherwise(move || {
        let start_controls = controls.clone();
        let ext_controls = controls.clone();
        let install_cb = install_ca.clone();
        view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(when(
                move || !running.get(),
                move || {
                    let toggle_controls = start_controls.clone();
                    header_action_chip(tr("开始监听", "Start listening"), true, move || {
                        toggle_capture(toggle_controls.clone())
                    })
                },
            ))
            .child(when(
                move || {
                    ca_ui.status.get() != ca::CaStatus::Trusted
                        && ca_ui.detected_external.with(|items| !items.is_empty())
                },
                move || {
                    let reuse_controls = ext_controls.clone();
                    header_action_chip(
                        move || -> SharedString {
                            let lang = ui_lang(langs);
                            let tool = ca_ui
                                .detected_external
                                .with(|items| items.first().map(|item| item.tool_name.clone()))
                                .unwrap_or_default();
                            match lang {
                                Lang::Zh => format!("复用 {tool} 证书").into(),
                                Lang::En => format!("Reuse {tool} CA").into(),
                            }
                        },
                        true,
                        move || {
                            if let Some(first) =
                                ca_ui.detected_external.with(|items| items.first().cloned())
                            {
                                use_external_ca(reuse_controls.clone(), first.id);
                            }
                        },
                    )
                },
            ))
            .child(when(
                move || ca_ui.status.get() != ca::CaStatus::Trusted,
                move || {
                    let cb = install_cb.clone();
                    header_action_chip(
                        move || -> SharedString {
                            let lang = ui_lang(langs);
                            match ca_ui.status.get() {
                                ca::CaStatus::InstalledUntrusted => {
                                    t(lang, "信任根证书", "Trust CA").into()
                                }
                                ca::CaStatus::NotInstalled | ca::CaStatus::Trusted => {
                                    t(lang, "安装根证书", "Install CA").into()
                                }
                            }
                        },
                        false,
                        move || cb.as_ref()(),
                    )
                },
            ))
    })
}

#[cfg(target_arch = "wasm32")]
fn onboarding_header_status(props: &OnboardingProps) -> impl IntoElement {
    let controls = props.controls.clone();
    let running = props.running;
    let listen = props.listen;
    when(
        move || running.get(),
        move || {
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(
                    view()
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded_full()
                        .bg(status_ok_fg()),
                )
                .child(
                    mono(move || SharedString::from(listen.get()))
                        .number_of_lines(1)
                        .text_size(px(11.5))
                        .text_color(fg_muted()),
                )
        },
    )
    .otherwise(move || {
        let toggle_controls = controls.clone();
        header_action_chip(tr("开始监听", "Start listening"), true, move || {
            toggle_capture(toggle_controls.clone())
        })
    })
}

fn header_action_chip(
    label: impl IntoReactive<SharedString>,
    accent: bool,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    nsh_base::button()
        .on_press(on_press)
        .h(px(26.0))
        .px_2p5()
        .gap_1p5()
        .flex_row()
        .items_center()
        .rounded(px(6.0))
        .border_1()
        .transition_colors()
        .duration(120.0)
        .ease_out()
        .border_color(if accent {
            accent_blue_border()
        } else {
            status_warn_fg()
        })
        .bg(if accent {
            accent_blue_soft()
        } else {
            status_warn_bg()
        })
        .hover(move |s| {
            s.bg(if accent {
                accent_blue_soft_hover()
            } else {
                status_warn_bg()
            })
        })
        .active(move |s| {
            s.bg(if accent {
                accent_blue_soft_hover()
            } else {
                status_warn_bg()
            })
            .opacity(0.85)
        })
        .focus_visible(|s| {
            s.outline(px(2.0))
                .outline_color(accent_blue())
                .outline_offset(px(2.0))
        })
        .child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(if accent {
            accent_blue_text()
        } else {
            status_warn_fg()
        }))
        .child(
            text(label)
                .number_of_lines(1)
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if accent {
                    accent_blue_text()
                } else {
                    status_warn_fg()
                }),
        )
}

#[cfg(not(target_arch = "wasm32"))]
fn onboarding_primary_section(
    props: &OnboardingProps,
    pane_width: Signal<f32>,
) -> impl IntoElement {
    let browser_controls = props.controls.clone();
    let term_controls = props.controls.clone();
    let adb_controls = props.controls.clone();
    let qr_controls = props.controls.clone();
    let selected = props.selected;
    let adb_count = props.adb_count;
    let running = props.running;
    let qr_open = props.qr_open;
    let pending_qr = props.pending_qr;
    let langs = ui_langs();

    let grid = view()
        .w_full()
        .grid()
        .grid_cols(move || onboarding_grid_cols(pane_width.get()))
        .gap_2p5()
        .child(onboarding_primary_card(
            Glyph::Globe,
            tr("隔离浏览器", "Isolated Browser"),
            tr("独立沙箱，免装系统证书", "SPKI trust, isolated profile"),
            || false,
            move || {
                let url = selected_browser_start_url(&browser_controls, selected.get());
                launch_browser_action(browser_controls.clone(), url);
            },
        ))
        .child(onboarding_primary_card(
            Glyph::Prompt,
            tr("代理终端", "Proxied Terminal"),
            tr("自动注入代理与 CA 变量", "Preloaded proxy & CA env"),
            || false,
            move || launch_terminal_action(term_controls.clone()),
        ))
        .child(onboarding_primary_card(
            Glyph::DeviceMobile,
            move || -> SharedString {
                let lang = ui_lang(langs);
                let n = adb_count.get();
                if n > 0 {
                    format!("{} · {n}", t(lang, "断开 ADB", "Disconnect ADB")).into()
                } else {
                    t(lang, "Android ADB", "Android ADB").into()
                }
            },
            tr("USB 端口反代并推送证书", "USB reverse proxy & cert"),
            move || adb_count.get() > 0,
            move || toggle_adb_action(adb_controls.clone()),
        ))
        .child(onboarding_primary_card(
            Glyph::QrCode,
            tr("手机扫码", "Phone QR"),
            tr("Wi-Fi 扫码下载证书与配置", "Scan QR for mobile cert"),
            || false,
            move || trigger_mobile_qr(&qr_controls, running, pending_qr, qr_open),
        ));

    view()
        .w_full()
        .flex_col()
        .gap_2()
        .child(section_title(tr("免配入口", "Quick Capture")))
        .child(grid)
}

#[cfg(target_arch = "wasm32")]
fn onboarding_web_section(props: &OnboardingProps, pane_width: Signal<f32>) -> impl IntoElement {
    let controls = props.controls.clone();
    let qr_controls = props.controls.clone();
    let running = props.running;
    let qr_open = props.qr_open;
    let pending_qr = props.pending_qr;

    let grid = view()
        .w_full()
        .grid()
        .grid_cols(move || onboarding_grid_cols(pane_width.get()))
        .gap_2p5()
        .child(onboarding_primary_card(
            Glyph::Prompt,
            tr("复制代理地址", "Copy Proxy Address"),
            tr("将客户端代理指向服务端端口", "Point client HTTP proxy here"),
            || false,
            move || {
                let port = lock_port(&controls.listen_port);
                set_clipboard(format!("http://127.0.0.1:{port}"));
                set_status(&controls, StatusMsg::CopiedUrl);
            },
        ))
        .child(onboarding_primary_card(
            Glyph::QrCode,
            tr("手机扫码", "Phone QR"),
            tr("查看手机安装页与二维码", "Open mobile install QR"),
            || false,
            move || trigger_mobile_qr(&qr_controls, running, pending_qr, qr_open),
        ));

    view()
        .w_full()
        .flex_col()
        .gap_2()
        .child(section_title(tr("免配入口", "Quick Capture")))
        .child(grid)
}

fn onboarding_secondary_section(
    props: &OnboardingProps,
    pane_width: Signal<f32>,
) -> impl IntoElement {
    let compose = props.compose;
    let selected = props.selected;
    #[cfg(not(target_arch = "wasm32"))]
    let import_controls = props.controls.clone();
    #[cfg(not(target_arch = "wasm32"))]
    let import_profiles = props.profiles;
    #[cfg(target_arch = "wasm32")]
    let panel_open = props.profiles.panel_open;
    let _ = props.profiles;

    let workflow_grid = view()
        .w_full()
        .grid()
        .grid_cols(move || onboarding_grid_cols(pane_width.get()))
        .gap_2p5()
        .child(onboarding_secondary_tile(
            Glyph::Play,
            tr("构造请求", "New Request"),
            tr(
                "编写请求或粘贴 cURL 发送",
                "Build HTTP request or paste cURL",
            ),
            move || begin_new_request(compose, selected),
        ))
        .child(onboarding_secondary_tile(
            Glyph::Download,
            tr("导入规则 / 会话", "Import Rules / Sessions"),
            tr(
                "Whistle / Charles / HAR / TOML",
                "Whistle / Charles / HAR / TOML",
            ),
            move || {
                #[cfg(not(target_arch = "wasm32"))]
                prompt_import_migration(import_controls.clone(), import_profiles);
                #[cfg(target_arch = "wasm32")]
                panel_open.set(true);
            },
        ));

    let section = view().w_full().flex_col().gap_2p5().child(
        view()
            .w_full()
            .flex_col()
            .gap_2()
            .child(section_title(tr("调试与更多", "Tools & More")))
            .child(workflow_grid),
    );

    #[cfg(not(target_arch = "wasm32"))]
    let section = section.child(onboarding_utility_strip(props));

    section
}

#[cfg(not(target_arch = "wasm32"))]
fn onboarding_utility_strip(props: &OnboardingProps) -> impl IntoElement {
    let sys_proxy_on = props.use_system_proxy;
    let toggle_sys = Rc::clone(&props.toggle_system_proxy);
    let env_controls = props.controls.clone();
    let langs = ui_langs();

    let strip = view()
        .w_full()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_2()
        .child(onboarding_utility_chip(
            Glyph::Shield,
            move || -> SharedString {
                let lang = ui_lang(langs);
                let st = if sys_proxy_on.get() {
                    t(lang, "开", "On")
                } else {
                    t(lang, "关", "Off")
                };
                format!("{} · {st}", t(lang, "系统代理", "System Proxy")).into()
            },
            move || sys_proxy_on.get(),
            move || toggle_sys.as_ref()(),
        ));

    #[cfg(target_os = "macos")]
    let strip = {
        let sim_controls = props.controls.clone();
        strip.child(onboarding_utility_chip(
            Glyph::Badge,
            tr("iOS 模拟器", "iOS Simulator"),
            || false,
            move || install_ca_ios_sim(sim_controls.clone()),
        ))
    };

    strip.child(onboarding_utility_chip(
        Glyph::DeviceDesktop,
        tr("复制代理环境", "Copy Proxy Env"),
        || false,
        move || copy_proxy_env(&env_controls),
    ))
}

fn interactive_surface_button(
    rest_bg: AdaptiveColor,
    rest_border: AdaptiveColor,
    active: impl Fn() -> bool + 'static + Clone,
    on_press: impl Fn() + 'static,
) -> nsh_base::ButtonElement {
    let active_sel = active.clone();
    let active_border = active.clone();
    let active_bg = active.clone();
    let active_hover = active.clone();
    let active_press = active;

    nsh_base::button()
        .selected(active_sel)
        .on_press(on_press)
        .flex_row()
        .items_center()
        .border_1()
        .transition_colors()
        .duration(120.0)
        .ease_out()
        .border_color(move || {
            if active_border() {
                accent_blue_border()
            } else {
                rest_border
            }
        })
        .bg(move || {
            if active_bg() {
                accent_blue_soft()
            } else {
                rest_bg
            }
        })
        .hover(move |s| {
            let h_bg = active_hover.clone();
            let h_bd = active_hover;
            s.bg(move || {
                if h_bg() {
                    accent_blue_soft_hover()
                } else {
                    bg_control()
                }
            })
            .border_color(move || {
                if h_bd() {
                    accent_blue_border()
                } else {
                    border_control()
                }
            })
        })
        .active(move |s| {
            s.bg(move || {
                if active_press() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .focus_visible(|s| {
            s.outline(px(2.0))
                .outline_color(accent_blue())
                .outline_offset(px(2.0))
        })
}

fn onboarding_primary_card(
    icon: Glyph,
    title: impl IntoReactive<SharedString>,
    subtitle: impl IntoReactive<SharedString>,
    active: impl Fn() -> bool + 'static + Clone,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let active_well = active.clone();
    let active_icon = active.clone();
    let active_text = active.clone();

    interactive_surface_button(bg_card(), border_strong(), active, on_press)
        .w_full()
        .min_w(px(0.0))
        .px_3p5()
        .py_3()
        .gap_3()
        .rounded(px(8.0))
        .child(
            view()
                .w(px(32.0))
                .h(px(32.0))
                .rounded(px(7.0))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .bg(move || {
                    if active_well() {
                        accent_blue_soft_hover()
                    } else {
                        bg_control()
                    }
                })
                .child(glyph_px(
                    15.0,
                    move || icon,
                    move || {
                        color(if active_icon() {
                            accent_blue_text()
                        } else {
                            fg_secondary()
                        })
                    },
                )),
        )
        .child(onboarding_card_labels(title, subtitle, active_text))
}

fn onboarding_card_labels(
    title: impl IntoReactive<SharedString>,
    subtitle: impl IntoReactive<SharedString>,
    active: impl Fn() -> bool + 'static,
) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(0.0))
        .flex_col()
        .gap_0p5()
        .child(
            text(title)
                .number_of_lines(1)
                .text_overflow(EllipsizeMode::Tail)
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if active() {
                        accent_blue_text()
                    } else {
                        fg_primary()
                    }
                }),
        )
        .child(
            text(subtitle)
                .number_of_lines(1)
                .text_overflow(EllipsizeMode::Tail)
                .text_size(px(11.5))
                .text_color(fg_muted()),
        )
}

fn onboarding_secondary_tile(
    icon: Glyph,
    title: impl IntoReactive<SharedString>,
    subtitle: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    nsh_base::button()
        .on_press(on_press)
        .w_full()
        .min_w(px(0.0))
        .px_3()
        .py_2p5()
        .gap_2p5()
        .flex_row()
        .items_center()
        .rounded(px(8.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_canvas())
        .transition_colors()
        .duration(120.0)
        .ease_out()
        .hover(|s| s.bg(bg_card()).border_color(border_control()))
        .active(|s| s.bg(bg_control()))
        .focus_visible(|s| {
            s.outline(px(2.0))
                .outline_color(accent_blue())
                .outline_offset(px(2.0))
        })
        .child(
            view()
                .w(px(18.0))
                .h(px(18.0))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .child(glyph_px(14.0, move || icon, || color(fg_muted()))),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_col()
                .gap_0p5()
                .child(
                    text(title)
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg_secondary()),
                )
                .child(
                    text(subtitle)
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .text_size(px(11.5))
                        .text_color(fg_muted()),
                ),
        )
}

#[cfg(not(target_arch = "wasm32"))]
fn onboarding_utility_chip(
    icon: Glyph,
    label: impl IntoReactive<SharedString>,
    active: impl Fn() -> bool + 'static + Clone,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let active_icon = active.clone();
    let active_text = active.clone();

    interactive_surface_button(bg_card(), border_control(), active, on_press)
        .h(px(28.0))
        .px_2p5()
        .gap_1p5()
        .rounded(px(6.0))
        .child(glyph_px(
            12.5,
            move || icon,
            move || {
                color(if active_icon() {
                    accent_blue_text()
                } else {
                    fg_muted()
                })
            },
        ))
        .child(
            text(label)
                .number_of_lines(1)
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if active_text() {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

fn onboarding_footer_hint() -> impl IntoElement {
    #[cfg(target_os = "macos")]
    let tip = tr(
        "提示：支持直接拖拽 .har / .chlsj / Whistle .txt / Charles .xml / .toml 到窗口导入；⌘⇧P 打开命令面板。",
        "Tip: Drop .har, .chlsj, Whistle .txt, Charles .xml, or .toml files to import; press ⌘⇧P for Command Palette.",
    );
    #[cfg(not(target_os = "macos"))]
    let tip = tr(
        "提示：支持直接拖拽 .har / .chlsj / Whistle .txt / Charles .xml / .toml 到窗口导入；Ctrl+Shift+P 打开命令面板。",
        "Tip: Drop .har, .chlsj, Whistle .txt, Charles .xml, or .toml files to import; press Ctrl+Shift+P for Command Palette.",
    );

    view()
        .w_full()
        .pt_3()
        .border_t_1()
        .border_t_color(border_subtle())
        .child(text(tip).text_size(px(11.5)).text_color(fg_muted()))
}
