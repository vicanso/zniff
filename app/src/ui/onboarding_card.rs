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
    pub(crate) onboarding_traffic: Signal<OnboardingTraffic>,
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
        .child(onboarding_header(&props))
        .child(onboarding_checklist(&props));

    #[cfg(not(target_arch = "wasm32"))]
    let root = root.child(onboarding_primary_section(&props, pane_width));

    #[cfg(target_arch = "wasm32")]
    let root = root.child(onboarding_web_section(&props, pane_width));

    root.child(onboarding_secondary_section(&props, pane_width))
        .child(onboarding_footer_hint())
}

fn onboarding_header(props: &OnboardingProps) -> impl IntoElement {
    let running = props.running;
    let listen = props.listen;
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
                        .text_size(px(TEXT_LG))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg_primary()),
                )
                .child(when(
                    move || running.get() && !listen.with(String::is_empty),
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
                                    .bg(status_ok_fg())
                                    .style_with(|s| {
                                        if reduce_motion() {
                                            s
                                        } else {
                                            s.animation(ui_pulse_animation(MOTION_PULSE_MS))
                                        }
                                    }),
                            )
                            .child(
                                mono(move || SharedString::from(listen.get()))
                                    .number_of_lines(1)
                                    .text_size(px(TEXT_XS))
                                    .text_color(fg_muted()),
                            )
                    },
                )),
        )
        .child(
            text(tr(
                "选择免配入口即可捕获流量，无需修改全局代理设置。",
                "Pick a zero-config entry to capture traffic without touching global proxy settings.",
            ))
            .text_size(px(TEXT_SM))
            .text_color(fg_muted()),
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
                set_clipboard(client_proxy_url(&controls));
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

/// Imports the built-in demo HAR session so users can explore zniff before configuring a proxy.
pub(crate) fn load_sample_traffic(controls: &Controls, lang: Lang) -> usize {
    let doc = crate::sample_har::sample_har_document();
    let count = controls.hub.import_har(&doc);
    set_status(
        controls,
        StatusMsg::Imported {
            count,
            path: t(lang, "内置示例流量", "Built-in sample traffic").to_string(),
        },
    );
    count
}

fn onboarding_secondary_section(
    props: &OnboardingProps,
    pane_width: Signal<f32>,
) -> impl IntoElement {
    let compose = props.compose;
    let selected = props.selected;
    let sample_controls = props.controls.clone();
    let langs = ui_langs();
    #[cfg(not(target_arch = "wasm32"))]
    let import_controls = props.controls.clone();
    #[cfg(not(target_arch = "wasm32"))]
    let import_profiles = props.profiles;
    #[cfg(target_arch = "wasm32")]
    let web_profiles = props.profiles;
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
            if cfg!(target_arch = "wasm32") {
                tr(
                    "Whistle / Charles XML / TOML",
                    "Whistle / Charles XML / TOML",
                )
            } else {
                tr(
                    "Whistle / Charles / HAR / TOML",
                    "Whistle / Charles / HAR / TOML",
                )
            },
            move || {
                #[cfg(not(target_arch = "wasm32"))]
                prompt_import_migration(import_controls.clone(), import_profiles);
                #[cfg(target_arch = "wasm32")]
                {
                    web_profiles.import_open.set(true);
                    web_profiles.preset_panel_open.set(true);
                }
            },
        ));

    let sample_tile = onboarding_secondary_tile(
        Glyph::List,
        tr("加载示例流量", "Load Sample Traffic"),
        tr(
            "导入内置演示会话，无需配置代理即可体验详情、规则与图表",
            "Import demo flows to try inspector, rules & diagrams without proxy setup",
        ),
        move || {
            load_sample_traffic(&sample_controls, ui_lang(langs));
        },
    );

    let section = view().w_full().flex_col().gap_2p5().child(
        view()
            .w_full()
            .flex_col()
            .gap_2()
            .child(section_title(tr("调试与更多", "Tools & More")))
            .child(workflow_grid)
            .child(sample_tile),
    );

    #[cfg(not(target_arch = "wasm32"))]
    let section = section.child(onboarding_more_connection_disclosure(props));

    section
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn onboarding_more_connection_disclosure(props: &OnboardingProps) -> impl IntoElement {
    let more_open = signal(props.use_system_proxy.get_untracked());
    let strip_props = props.clone();
    collapsible()
        .w_full()
        .flex_col()
        .open(move || more_open.get())
        .reveal(true)
        .content(
            view()
                .w_full()
                .pt_2()
                .child(onboarding_utility_strip(strip_props)),
        )
        .child(
            button()
                .self_start()
                .h(px(ControlSize::Xs.height()))
                .px_1p5()
                .rounded(px(RADIUS_SM))
                .flex_row()
                .items_center()
                .gap_1()
                .hover(|s| s.bg(bg_control_hover()))
                .active(|s| s.bg(bg_control_active()))
                .transition_colors()
                .duration(move || motion_ms(MOTION_INSTANT_MS))
                .ease_out()
                .focus_visible(apply_focus_ring)
                .accessibility_label(tr_opt("更多连接方式", "More connection options"))
                .on_press(move |_: &()| more_open.update(|v| *v = !*v))
                .child(ui_rotating_chevron(
                    10.0,
                    move || more_open.get(),
                    || color(fg_muted()),
                ))
                .child(
                    text(tr("更多连接方式", "More connection options"))
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg_muted()),
                ),
        )
}

#[cfg(not(target_arch = "wasm32"))]
fn onboarding_utility_strip(props: OnboardingProps) -> impl IntoElement {
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
        .transition()
        .duration(move || motion_ms(MOTION_INSTANT_MS))
        .ease_bezier(0.22, 1.0, 0.36, 1.0)
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
            s.scale(move || if reduce_motion() { 1.0_f32 } else { 0.985_f32 })
                .bg(move || {
                    if active_press() {
                        accent_blue_soft_hover()
                    } else {
                        bg_control_active()
                    }
                })
        })
        .focus_visible(apply_focus_ring)
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
        .rounded(px(RADIUS_LG))
        .child(
            view()
                .w(px(32.0))
                .h(px(32.0))
                .rounded(px(RADIUS_MD))
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
                    16.0,
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
                .text_size(px(TEXT_MD))
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
                .text_size(px(TEXT_XS))
                .text_color(fg_muted()),
        )
}

fn onboarding_secondary_tile(
    icon: Glyph,
    title: impl IntoReactive<SharedString>,
    subtitle: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    interactive_surface_button(bg_canvas(), border_strong(), || false, on_press)
        .w_full()
        .min_w(px(0.0))
        .px_3()
        .py_2p5()
        .gap_2p5()
        .rounded(px(RADIUS_MD))
        .child(
            view()
                .w(px(18.0))
                .h(px(18.0))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .child(glyph_px(14.0, move || icon, || color(fg_muted()))),
        )
        .child(onboarding_card_labels(title, subtitle, || false))
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
        .h(px(CONTROL_H))
        .px_2p5()
        .gap_1p5()
        .rounded(px(RADIUS_MD))
        .child(glyph_px(
            12.0,
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
                .text_size(px(TEXT_SM))
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
        .child(text(tip).text_size(px(TEXT_XS)).text_color(fg_muted()))
}
