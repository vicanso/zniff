#[allow(unused_imports)]
use super::*;

#[derive(Clone)]
pub(crate) struct OnboardingProps {
    pub(crate) controls: Arc<Controls>,
    pub(crate) running: Signal<bool>,
    pub(crate) listen: Signal<String>,
    pub(crate) ca_status: Signal<ca::CaStatus>,
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

/// Returns true when the right detail pane should show the Quick Start onboarding card.
pub(crate) fn should_show_onboarding(phase: Phase, total_count: usize) -> bool {
    phase == Phase::Empty && total_count == 0
}

/// Actionable empty-state onboarding card shown when no flows have been captured yet.
pub(crate) fn onboarding_card(props: OnboardingProps) -> impl IntoElement {
    let card = view()
        .w_full()
        .max_w(px(680.0))
        .p_4()
        .gap_3p5()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .child(onboarding_header(&props));

    #[cfg(not(target_arch = "wasm32"))]
    let card = card
        .child(onboarding_desktop_group(&props))
        .child(onboarding_mobile_group(&props));

    #[cfg(target_arch = "wasm32")]
    let card = card.child(onboarding_web_group(&props));

    card.child(onboarding_compose_import_group(&props))
        .child(onboarding_footer_hint())
}

fn onboarding_header(props: &OnboardingProps) -> impl IntoElement {
    let controls = props.controls.clone();
    let running = props.running;
    let listen = props.listen;
    let langs = ui_langs();

    let status_row = view().flex_row().flex_wrap().items_center().gap_2().child(
        view()
            .h(px(26.0))
            .px_2p5()
            .rounded(px(6.0))
            .border_1()
            .border_color(move || {
                if running.get() {
                    accent_blue_border()
                } else {
                    border_control()
                }
            })
            .bg(move || {
                if running.get() {
                    accent_blue_soft()
                } else {
                    bg_control()
                }
            })
            .hover(|style| {
                style.bg(move || {
                    if running.get() {
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
            .gap_1p5()
            .on_press(move || toggle_capture(controls.clone()))
            .child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(move || {
                if running.get() {
                    status_ok_fg()
                } else {
                    fg_muted()
                }
            }))
            .child(
                mono(move || -> SharedString {
                    let lang = ui_lang(langs);
                    let addr = listen.get();
                    if running.get() && !addr.is_empty() {
                        format!("{} {addr}", t(lang, "监听中", "Listening")).into()
                    } else {
                        t(lang, "点击开始监听", "Start listening").into()
                    }
                })
                .number_of_lines(1)
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if running.get() {
                        accent_blue_text()
                    } else {
                        fg_primary()
                    }
                }),
            ),
    );

    #[cfg(not(target_arch = "wasm32"))]
    let status_row = status_row.child(onboarding_ca_chip(
        props.ca_status,
        props.install_ca.clone(),
    ));

    view().w_full().flex_col().gap_2().child(
        view()
            .w_full()
            .flex_row()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                view()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        text(tr("快速开始抓包", "Quick Start"))
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg_primary()),
                    )
                    .child(
                        text(tr(
                            "选择一种免配入口或移动端工具，即可开始捕获并调试流量。",
                            "Pick a zero-config launcher or mobile setup to start capturing.",
                        ))
                        .text_size(px(12.0))
                        .text_color(fg_muted()),
                    ),
            )
            .child(status_row),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn onboarding_ca_chip(
    ca_status: Signal<ca::CaStatus>,
    install_ca: Rc<dyn Fn()>,
) -> impl IntoElement {
    let langs = ui_langs();
    view()
        .h(px(26.0))
        .px_2p5()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || match ca_status.get() {
            ca::CaStatus::Trusted => border_subtle(),
            ca::CaStatus::InstalledUntrusted | ca::CaStatus::NotInstalled => status_warn_fg(),
        })
        .bg(move || match ca_status.get() {
            ca::CaStatus::Trusted => bg_code(),
            ca::CaStatus::InstalledUntrusted | ca::CaStatus::NotInstalled => status_warn_bg(),
        })
        .hover(|style| {
            style.bg(move || match ca_status.get() {
                ca::CaStatus::Trusted => bg_code(),
                ca::CaStatus::InstalledUntrusted | ca::CaStatus::NotInstalled => status_warn_bg(),
            })
        })
        .style_with(move |s| {
            if ca_status.get() == ca::CaStatus::Trusted {
                s
            } else {
                s.cursor(Cursor::Pointer)
            }
        })
        .flex_row()
        .items_center()
        .gap_1p5()
        .on_press(move || {
            if ca_status.get() != ca::CaStatus::Trusted {
                install_ca.as_ref()();
            }
        })
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .bg(move || match ca_status.get() {
                    ca::CaStatus::Trusted => status_ok_fg(),
                    ca::CaStatus::InstalledUntrusted | ca::CaStatus::NotInstalled => {
                        status_warn_fg()
                    }
                }),
        )
        .child(
            text(move || -> SharedString {
                let lang = ui_lang(langs);
                match ca_status.get() {
                    ca::CaStatus::Trusted => t(lang, "根证书已信任", "Root CA trusted").into(),
                    ca::CaStatus::InstalledUntrusted => {
                        t(lang, "根证书未信任 · 点击信任", "CA untrusted · Trust").into()
                    }
                    ca::CaStatus::NotInstalled => {
                        t(lang, "根证书未安装 · 点击安装", "CA missing · Install").into()
                    }
                }
            })
            .number_of_lines(1)
            .text_size(px(11.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || match ca_status.get() {
                ca::CaStatus::Trusted => fg_secondary(),
                ca::CaStatus::InstalledUntrusted | ca::CaStatus::NotInstalled => status_warn_fg(),
            }),
        )
}

#[cfg(not(target_arch = "wasm32"))]
fn onboarding_desktop_group(props: &OnboardingProps) -> impl IntoElement {
    let browser_controls = props.controls.clone();
    let term_controls = props.controls.clone();
    let env_controls = props.controls.clone();
    let selected = props.selected;
    view()
        .w_full()
        .gap_1p5()
        .flex_col()
        .child(section_title(tr("免配桌面抓包", "Zero-Config Desktop")))
        .child(
            view()
                .w_full()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(onboarding_tile(
                    Glyph::Globe,
                    tr("隔离浏览器", "Isolated Browser"),
                    tr("免装系统证书，独立沙箱", "SPKI trust, isolated profile"),
                    || false,
                    move || {
                        let url = selected_browser_start_url(&browser_controls, selected.get());
                        launch_browser_action(browser_controls.clone(), url);
                    },
                ))
                .child(onboarding_tile(
                    Glyph::Prompt,
                    tr("代理终端", "Proxied Terminal"),
                    tr("自动注入代理与 CA 变量", "Preloaded proxy & CA env"),
                    || false,
                    move || launch_terminal_action(term_controls.clone()),
                ))
                .child(onboarding_tile(
                    Glyph::DeviceDesktop,
                    tr("复制代理环境", "Copy Proxy Env"),
                    tr("导出终端 export 变量", "Copy shell export vars"),
                    || false,
                    move || copy_proxy_env(&env_controls),
                )),
        )
}

#[cfg(not(target_arch = "wasm32"))]
fn onboarding_mobile_group(props: &OnboardingProps) -> impl IntoElement {
    let adb_controls = props.controls.clone();
    let adb_count = props.adb_count;
    let qr_controls = props.controls.clone();
    let running = props.running;
    let qr_open = props.qr_open;
    let pending_qr = props.pending_qr;
    let sys_proxy_on = props.use_system_proxy;
    let toggle_sys = Rc::clone(&props.toggle_system_proxy);
    let langs = ui_langs();

    let row = view()
        .w_full()
        .flex_row()
        .flex_wrap()
        .gap_2()
        .child(onboarding_tile(
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
        ));

    #[cfg(target_os = "macos")]
    let row = {
        let sim_controls = props.controls.clone();
        row.child(onboarding_tile(
            Glyph::Badge,
            tr("iOS 模拟器", "iOS Simulator"),
            tr("注入证书到开机模拟器", "Setup booted simulators"),
            || false,
            move || install_ca_ios_sim(sim_controls.clone()),
        ))
    };

    let row = row
        .child(onboarding_tile(
            Glyph::DeviceMobile,
            tr("手机扫码", "Phone QR"),
            tr("Wi-Fi 扫码下载证书与配置", "Scan QR for mobile cert"),
            || false,
            move || trigger_mobile_qr(&qr_controls, running, pending_qr, qr_open),
        ))
        .child(onboarding_tile(
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
            tr("接管本机系统全局代理", "Toggle OS HTTP proxy"),
            move || sys_proxy_on.get(),
            move || toggle_sys.as_ref()(),
        ));

    view()
        .w_full()
        .gap_1p5()
        .flex_col()
        .child(section_title(tr("移动端与系统", "Mobile & System")))
        .child(row)
}

#[cfg(target_arch = "wasm32")]
fn onboarding_web_group(props: &OnboardingProps) -> impl IntoElement {
    let controls = props.controls.clone();
    let qr_controls = props.controls.clone();
    let running = props.running;
    let qr_open = props.qr_open;
    let pending_qr = props.pending_qr;
    view()
        .w_full()
        .gap_1p5()
        .flex_col()
        .child(section_title(tr("客户端接入", "Client Setup")))
        .child(
            view()
                .w_full()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(onboarding_tile(
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
                .child(onboarding_tile(
                    Glyph::DeviceMobile,
                    tr("手机扫码", "Phone QR"),
                    tr("查看手机安装页与二维码", "Open mobile install QR"),
                    || false,
                    move || trigger_mobile_qr(&qr_controls, running, pending_qr, qr_open),
                )),
        )
}

fn onboarding_compose_import_group(props: &OnboardingProps) -> impl IntoElement {
    let compose = props.compose;
    let selected = props.selected;
    #[cfg(not(target_arch = "wasm32"))]
    let import_controls = props.controls.clone();
    #[cfg(not(target_arch = "wasm32"))]
    let import_profiles = props.profiles;
    #[cfg(target_arch = "wasm32")]
    let panel_open = props.profiles.panel_open;
    let _ = props.profiles;
    view()
        .w_full()
        .gap_1p5()
        .flex_col()
        .child(section_title(tr("接口调试与迁移", "Compose & Import")))
        .child(
            view()
                .w_full()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(onboarding_tile(
                    Glyph::Prompt,
                    tr("构造请求", "New Request"),
                    tr("支持粘贴 cURL 一键发送", "Build HTTP request or paste cURL"),
                    || false,
                    move || begin_new_request(compose, selected),
                ))
                .child(onboarding_tile(
                    Glyph::Download,
                    tr("导入规则 / 会话", "Import Rules / Sessions"),
                    tr(
                        "支持 Whistle / Charles / HAR / TOML",
                        "Whistle, Charles XML/.chlsj, HAR, TOML",
                    ),
                    || false,
                    move || {
                        #[cfg(not(target_arch = "wasm32"))]
                        prompt_import_migration(import_controls.clone(), import_profiles);
                        #[cfg(target_arch = "wasm32")]
                        panel_open.set(true);
                    },
                )),
        )
}

fn onboarding_footer_hint() -> impl IntoElement {
    view()
        .w_full()
        .pt_1()
        .border_t_1()
        .border_t_color(border_subtle())
        .child(
            text(tr(
                "提示：支持直接拖拽 .har / .chlsj / Whistle .txt / Charles .xml / .toml 到窗口导入；Cmd/Ctrl+Shift+P 打开命令面板。",
                "Tip: Drop .har, .chlsj, Whistle .txt, Charles .xml, or .toml files to import; press Cmd/Ctrl+Shift+P for Command Palette.",
            ))
            .text_size(px(11.0))
            .text_color(fg_muted()),
        )
}

fn onboarding_tile(
    icon: Glyph,
    title: impl IntoReactive<SharedString>,
    subtitle: impl IntoReactive<SharedString>,
    active: impl Fn() -> bool + 'static + Clone,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let active_border = active.clone();
    let active_bg = active.clone();
    let active_hover = active.clone();
    let active_icon = active.clone();
    let active_text = active;
    view()
        .flex_1()
        .min_w(px(184.0))
        .px_3()
        .py_2()
        .gap_2()
        .flex_row()
        .items_center()
        .rounded(px(8.0))
        .border_1()
        .border_color(move || {
            if active_border() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if active_bg() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if active_hover() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .on_press(on_press)
        .child(glyph_px(
            14.0,
            move || icon,
            move || {
                color(if active_icon() {
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
                .gap(px(1.5))
                .child(
                    text(title)
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if active_text() {
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
                        .text_size(px(11.0))
                        .text_color(fg_muted()),
                ),
        )
}
