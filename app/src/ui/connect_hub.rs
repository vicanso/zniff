#[allow(unused_imports)]
use super::*;

#[derive(Clone)]
pub(crate) struct ConnectHubProps {
    pub(crate) controls: Arc<Controls>,
    pub(crate) running: Signal<bool>,
    pub(crate) listen: Signal<String>,
    pub(crate) ca_ui: CaSignals,
    pub(crate) use_system_proxy: Signal<bool>,
    pub(crate) adb_count: Signal<usize>,
    pub(crate) selected: Signal<Option<u64>>,
    pub(crate) install_ca: Rc<dyn Fn()>,
    pub(crate) toggle_system_proxy: Rc<dyn Fn()>,
    pub(crate) qr_open: Signal<bool>,
    pub(crate) pending_qr: Signal<bool>,
}

/// Formats the compact footer Connect Hub trigger text.
pub(crate) fn connect_hub_trigger_label(lang: Lang, adb_count: usize) -> String {
    let base = t(lang, "接入", "Connect");
    if adb_count > 0 {
        format!("{base} · ADB {adb_count}")
    } else {
        base.to_string()
    }
}

/// Opens the mobile QR popover, starting capture automatically when idle.
pub(crate) fn trigger_mobile_qr(
    controls: &Arc<Controls>,
    running: Signal<bool>,
    pending_qr: Signal<bool>,
    qr_open: Signal<bool>,
) {
    if running.get_untracked() {
        qr_open.set(true);
    } else {
        pending_qr.set(true);
        toggle_capture(controls.clone());
    }
}

/// Persistent footer popover consolidating desktop zero-config launchers,
/// Android ADB, iOS Simulator, system proxy, and mobile QR setup.
pub(crate) fn connect_hub_popover(props: ConnectHubProps) -> impl IntoElement {
    let open = signal(false);
    let adb_count = props.adb_count;
    let running = props.running;
    popover()
        .open(move || Some(open.get()))
        .placement(Placement::Top)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(8.0))
        .on_open_change(move |event: &nsh_base::PopoverEvent| {
            if let nsh_base::PopoverEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .trigger(connect_hub_trigger(open, running, adb_count))
        .child(connect_hub_panel(props, open))
}

fn connect_hub_trigger(
    open: Signal<bool>,
    running: Signal<bool>,
    adb_count: Signal<usize>,
) -> impl IntoElement {
    let active = move || open.get() || adb_count.get() > 0;
    view()
        .h(px(22.0))
        .px_2()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_1()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || {
            if active() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if active() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if active() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.75))
        .tooltip(label_tip(
            "接入中心：免配浏览器 / 终端 / ADB / 手机扫码",
            "Connect Hub: Browser / Terminal / ADB / Mobile QR",
        ))
        .child(glyph_px(
            11.5,
            || Glyph::Globe,
            move || {
                color(if active() {
                    accent_blue_text()
                } else if running.get() {
                    fg_secondary()
                } else {
                    fg_muted()
                })
            },
        ))
        .child(
            text({
                let langs = ui_langs();
                move || {
                    SharedString::from(connect_hub_trigger_label(ui_lang(langs), adb_count.get()))
                }
            })
            .tabular_nums()
            .number_of_lines(1)
            .text_size(px(11.5))
            .text_color(move || {
                if active() {
                    accent_blue_text()
                } else {
                    fg_secondary()
                }
            }),
        )
}

fn connect_hub_panel(props: ConnectHubProps, open: Signal<bool>) -> impl IntoElement {
    view()
        .w(move || px((window().width.0 - 20.0).clamp(280.0, 344.0)))
        .max_h(move || px((window().height.0 - 72.0).max(200.0)))
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
        .overflow_hidden()
        .child(
            scroll()
                .w_full()
                .max_h(move || px((window().height.0 - 74.0).max(198.0)))
                .child(connect_hub_body(props, open)),
        )
}

fn connect_hub_body(props: ConnectHubProps, open: Signal<bool>) -> impl IntoElement {
    let body = view()
        .w_full()
        .p_3()
        .gap_2p5()
        .flex_col()
        .child(hub_header())
        .child(hub_status_strip(
            props.controls.clone(),
            props.running,
            props.listen,
            props.ca_ui,
            props.install_ca.clone(),
        ));
    #[cfg(not(target_arch = "wasm32"))]
    let body = body
        .child(hub_desktop_section(&props, open))
        .child(hub_mobile_section(&props, open));
    #[cfg(target_arch = "wasm32")]
    let body = body.child(hub_web_section(&props, open));
    body
}

fn hub_header() -> impl IntoElement {
    view()
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            text(tr("接入中心", "Connect Hub"))
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_primary()),
        )
        .child(
            text(tr("免配一键抓包", "Zero-config capture"))
                .text_size(px(11.0))
                .text_color(fg_muted()),
        )
}

fn hub_status_strip(
    controls: Arc<Controls>,
    running: Signal<bool>,
    listen: Signal<String>,
    ca_ui: CaSignals,
    install_ca: Rc<dyn Fn()>,
) -> impl IntoElement {
    let strip = view()
        .w_full()
        .p_2()
        .gap_1p5()
        .flex_col()
        .rounded(px(8.0))
        .border_1()
        .border_color(border_subtle())
        .bg(bg_code())
        .child(hub_listen_row(controls.clone(), running, listen));
    #[cfg(not(target_arch = "wasm32"))]
    let strip = strip.child(hub_ca_row(controls, ca_ui, install_ca));
    #[cfg(target_arch = "wasm32")]
    let _ = (ca_ui, install_ca);
    strip
}

fn hub_listen_row(
    controls: Arc<Controls>,
    running: Signal<bool>,
    listen: Signal<String>,
) -> impl IntoElement {
    let langs = ui_langs();
    view()
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(move || {
                    if running.get() {
                        status_ok_fg()
                    } else {
                        fg_muted()
                    }
                }))
                .child(
                    mono(move || -> SharedString {
                        let addr = listen.get();
                        if addr.is_empty() {
                            t(ui_lang(langs), "未监听", "Stopped").into()
                        } else {
                            addr.into()
                        }
                    })
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail)
                    .text_size(px(11.5))
                    .text_color(fg_primary()),
                ),
        )
        .child(menu_inline_pill(
            move || {
                let lang = ui_lang(langs);
                SharedString::from(if running.get() {
                    t(lang, "停止", "Stop")
                } else {
                    t(lang, "开始", "Start")
                })
            },
            move || running.get(),
            move || toggle_capture(controls.clone()),
        ))
}

#[cfg(not(target_arch = "wasm32"))]
fn hub_ca_row(
    controls: Arc<Controls>,
    ca_ui: CaSignals,
    install_ca: Rc<dyn Fn()>,
) -> impl IntoElement {
    let langs = ui_langs();
    let reset_controls = controls.clone();
    let reuse_controls = controls;
    view()
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(move || {
                    match ca_ui.status.get() {
                        ca::CaStatus::Trusted => status_ok_fg(),
                        ca::CaStatus::InstalledUntrusted | ca::CaStatus::NotInstalled => {
                            status_warn_fg()
                        }
                    }
                }))
                .child(
                    text(move || -> SharedString {
                        let lang = ui_lang(langs);
                        if let Some(ext) = ca_ui.active_external.get() {
                            return match lang {
                                Lang::Zh => format!("根证书：已复用 {}", ext.tool_name).into(),
                                Lang::En => format!("Root CA: {}", ext.tool_name).into(),
                            };
                        }
                        match ca_ui.status.get() {
                            ca::CaStatus::Trusted => {
                                t(lang, "根证书：已信任", "Root CA: Trusted").into()
                            }
                            ca::CaStatus::InstalledUntrusted => {
                                t(lang, "根证书：未信任", "Root CA: Untrusted").into()
                            }
                            ca::CaStatus::NotInstalled => {
                                t(lang, "根证书：未安装", "Root CA: Not installed").into()
                            }
                        }
                    })
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail)
                    .text_size(px(11.5))
                    .text_color(move || {
                        if ca_ui.active_external.get().is_none()
                            && ca_ui.status.get() == ca::CaStatus::InstalledUntrusted
                        {
                            status_warn_fg()
                        } else {
                            fg_secondary()
                        }
                    }),
                ),
        )
        .child(
            when(
                move || ca_ui.active_external.get().is_some(),
                move || {
                    let cb = reset_controls.clone();
                    menu_inline_pill(
                        tr("切回", "Reset"),
                        || false,
                        move || use_builtin_ca(cb.clone()),
                    )
                },
            )
            .otherwise(move || {
                let reuse_cb = reuse_controls.clone();
                let install = Rc::clone(&install_ca);
                when(
                    move || ca_ui.status.get() != ca::CaStatus::Trusted,
                    move || {
                        let reuse_cb = reuse_cb.clone();
                        let install = Rc::clone(&install);
                        view()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .child(when(
                                move || ca_ui.detected_external.with(|items| !items.is_empty()),
                                move || {
                                    let cb = reuse_cb.clone();
                                    menu_inline_pill(
                                        move || -> SharedString {
                                            let lang = ui_lang(langs);
                                            let tool = ca_ui
                                                .detected_external
                                                .with(|items| {
                                                    items.first().map(|item| item.tool_name.clone())
                                                })
                                                .unwrap_or_default();
                                            match lang {
                                                Lang::Zh => format!("复用 {tool}").into(),
                                                Lang::En => format!("Reuse {tool}").into(),
                                            }
                                        },
                                        || true,
                                        move || {
                                            if let Some(first) = ca_ui
                                                .detected_external
                                                .with(|items| items.first().cloned())
                                            {
                                                use_external_ca(cb.clone(), first.id);
                                            }
                                        },
                                    )
                                },
                            ))
                            .child(menu_inline_pill(
                                move || {
                                    let lang = ui_lang(langs);
                                    SharedString::from(
                                        if ca_ui.status.get() == ca::CaStatus::InstalledUntrusted {
                                            t(lang, "信任", "Trust")
                                        } else {
                                            t(lang, "安装", "Install")
                                        },
                                    )
                                },
                                || true,
                                move || install.as_ref()(),
                            ))
                    },
                )
            }),
        )
}

#[cfg(not(target_arch = "wasm32"))]
fn hub_desktop_section(props: &ConnectHubProps, open: Signal<bool>) -> impl IntoElement {
    let browser_controls = props.controls.clone();
    let term_controls = props.controls.clone();
    let env_controls = props.controls.clone();
    let selected = props.selected;
    view()
        .w_full()
        .gap_1p5()
        .flex_col()
        .child(hub_section_label(tr("桌面免配抓包", "Desktop Zero-Config")))
        .child(hub_action_row(
            Glyph::Globe,
            tr("隔离浏览器 (Chromium)", "Isolated Browser (Chromium)"),
            tr("免装系统证书，独立沙箱", "SPKI trust, isolated profile"),
            || false,
            move || {
                open.set(false);
                let url = selected_browser_start_url(&browser_controls, selected.get());
                launch_browser_action(browser_controls.clone(), url);
            },
        ))
        .child(hub_action_row(
            Glyph::Prompt,
            tr("预注入代理终端", "Proxied Terminal"),
            tr("自动注入代理与 CA 变量", "Preloaded proxy & CA env vars"),
            || false,
            move || {
                open.set(false);
                launch_terminal_action(term_controls.clone());
            },
        ))
        .child(hub_action_row(
            Glyph::DeviceDesktop,
            tr("复制代理环境变量", "Copy Proxy Env"),
            tr("复制 export 命令至剪贴板", "Copy shell export variables"),
            || false,
            move || {
                open.set(false);
                copy_proxy_env(&env_controls);
            },
        ))
}

#[cfg(not(target_arch = "wasm32"))]
fn hub_mobile_section(props: &ConnectHubProps, open: Signal<bool>) -> impl IntoElement {
    let adb_controls = props.controls.clone();
    let adb_count = props.adb_count;
    let qr_controls = props.controls.clone();
    let running = props.running;
    let qr_open = props.qr_open;
    let pending_qr = props.pending_qr;
    let sys_proxy_on = props.use_system_proxy;
    let toggle_sys = Rc::clone(&props.toggle_system_proxy);
    let langs = ui_langs();

    let sec = view()
        .w_full()
        .gap_1p5()
        .flex_col()
        .child(hub_section_label(tr("移动端与系统", "Mobile & System")))
        .child(hub_action_row_reactive(
            Glyph::DeviceMobile,
            move || -> SharedString {
                let lang = ui_lang(langs);
                let n = adb_count.get();
                if n > 0 {
                    format!(
                        "{} (ADB · {n})",
                        t(lang, "断开 Android ADB", "Disconnect ADB")
                    )
                    .into()
                } else {
                    t(lang, "Android ADB 反向代理", "Android ADB Reverse").into()
                }
            },
            tr("USB 线直连端口反代", "USB reverse proxy via adb"),
            move || adb_count.get() > 0,
            move || {
                open.set(false);
                toggle_adb_action(adb_controls.clone());
            },
        ))
        .child(hub_android_cert_guide(props.controls.clone()));

    #[cfg(target_os = "macos")]
    let sec = {
        let sim_controls = props.controls.clone();
        sec.child(hub_action_row(
            Glyph::Badge,
            tr("iOS 模拟器证书与代理", "iOS Simulator Cert & Proxy"),
            tr("写入全部 Booted 模拟器", "Configure booted simulators"),
            || false,
            move || {
                open.set(false);
                install_ca_ios_sim(sim_controls.clone());
            },
        ))
    };

    sec.child(hub_action_row(
        Glyph::DeviceMobile,
        tr("手机扫码安装页", "Phone QR Install"),
        tr("局域网扫码下发证书与地址", "Scan QR for mobile CA & proxy"),
        || false,
        move || {
            open.set(false);
            trigger_mobile_qr(&qr_controls, running, pending_qr, qr_open);
        },
    ))
    .child(hub_pac_row(props.controls.clone(), open))
    .child(hub_action_row_reactive(
        Glyph::Shield,
        move || -> SharedString {
            let lang = ui_lang(langs);
            let state = if sys_proxy_on.get() {
                t(lang, "已开启", "On")
            } else {
                t(lang, "未开启", "Off")
            };
            format!("{} · {state}", t(lang, "系统代理", "System Proxy")).into()
        },
        tr("接管本机系统全局 HTTP 代理", "Route OS HTTP/HTTPS proxy"),
        move || sys_proxy_on.get(),
        move || toggle_sys.as_ref()(),
    ))
}

#[cfg(not(target_arch = "wasm32"))]
fn hub_android_cert_guide(controls: Arc<Controls>) -> impl IntoElement {
    let expanded = signal(false);
    view()
        .w_full()
        .flex_col()
        .rounded(px(7.0))
        .border_1()
        .border_color(border_subtle())
        .bg(bg_code())
        .overflow_hidden()
        .child(hub_android_cert_guide_header(expanded))
        .child(when(
            move || expanded.get(),
            move || hub_android_cert_guide_body(controls.clone()),
        ))
}

#[cfg(not(target_arch = "wasm32"))]
fn hub_android_cert_guide_header(expanded: Signal<bool>) -> impl IntoElement {
    view()
        .w_full()
        .px_2p5()
        .py_1p5()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .cursor(Cursor::Pointer)
        .hover(|style| style.bg(bg_control_hover()))
        .on_press(move || expanded.set(!expanded.get_untracked()))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(glyph_px(11.5, || Glyph::Shield, || color(fg_secondary())))
                .child(
                    text(tr(
                        "Android 7+ 证书配置 (network_security_config)",
                        "Android 7+ Cert Guide (network_security_config)",
                    ))
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail)
                    .text_size(px(11.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg_secondary()),
                ),
        )
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
        ))
}

#[cfg(not(target_arch = "wasm32"))]
fn hub_android_cert_guide_body(controls: Arc<Controls>) -> impl IntoElement {
    let langs = ui_langs();
    let xml_cb = controls.clone();
    let all_cb = controls;
    view()
        .w_full()
        .px_2p5()
        .pb_2()
        .pt_1()
        .gap_1p5()
        .flex_col()
        .border_t_1()
        .border_color(border_subtle())
        .child(
            text(tr(
                "Android 7+ 应用默认忽略用户 CA。在 Debug 构建添加 res/xml/network_security_config.xml 并在 AndroidManifest.xml 引用：",
                "Android 7+ apps ignore user CAs by default. Add res/xml/network_security_config.xml for debug builds and reference it in AndroidManifest.xml:",
            ))
            .text_size(px(10.5))
            .text_color(fg_muted()),
        )
        .child(
            view()
                .w_full()
                .p_2()
                .rounded(px(6.0))
                .border_1()
                .border_color(border_subtle())
                .bg(bg_card())
                .child(
                    mono(crate::android_guide::connect_hub_preview())
                        .text_size(px(10.0))
                        .text_color(fg_primary()),
                ),
        )
        .child(
            mono(crate::android_guide::MANIFEST_APPLICATION_ATTR)
                .text_size(px(10.0))
                .text_color(fg_secondary()),
        )
        .child(
            view()
                .w_full()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(menu_inline_pill(
                    tr("复制 XML", "Copy XML"),
                    || true,
                    move || {
                        set_clipboard(
                            crate::android_guide::NETWORK_SECURITY_CONFIG_XML.to_string(),
                        );
                        set_status(&xml_cb, StatusMsg::CopiedAndroidCertGuide);
                    },
                ))
                .child(menu_inline_pill(
                    tr("复制完整指引（含模拟器命令）", "Copy Full Guide (+ Emulator)"),
                    || false,
                    move || {
                        let guide = crate::android_guide::full_clipboard_guide(ui_lang(langs));
                        set_clipboard(guide);
                        set_status(&all_cb, StatusMsg::CopiedAndroidCertGuide);
                    },
                )),
        )
}

#[cfg(target_arch = "wasm32")]
fn hub_web_section(props: &ConnectHubProps, open: Signal<bool>) -> impl IntoElement {
    let controls = props.controls.clone();
    let qr_controls = props.controls.clone();
    let running = props.running;
    let qr_open = props.qr_open;
    let pending_qr = props.pending_qr;
    view()
        .w_full()
        .gap_1p5()
        .flex_col()
        .child(hub_section_label(tr("客户端接入", "Client Setup")))
        .child(hub_action_row(
            Glyph::Prompt,
            tr("复制代理地址", "Copy Proxy Address"),
            tr(
                "配置客户端 HTTP 代理指向服务端",
                "Point client HTTP proxy to server",
            ),
            || false,
            move || {
                let port = lock_port(&controls.listen_port);
                set_clipboard(format!("http://127.0.0.1:{port}"));
                set_status(&controls, StatusMsg::CopiedUrl);
                open.set(false);
            },
        ))
        .child(hub_action_row(
            Glyph::DeviceMobile,
            tr("手机扫码安装页", "Phone QR Install"),
            tr("打开手机安装页二维码", "Open mobile install QR page"),
            || false,
            move || {
                open.set(false);
                trigger_mobile_qr(&qr_controls, running, pending_qr, qr_open);
            },
        ))
        .child(hub_pac_row(props.controls.clone(), open))
}

fn hub_pac_row(controls: Arc<Controls>, open: Signal<bool>) -> impl IntoElement {
    hub_action_row(
        Glyph::Sliders,
        tr("复制 PAC 自动代理地址", "Copy PAC Auto-Proxy URL"),
        tr(
            "按直通名单动态生成 /proxy.pac 脚本",
            "Dynamic /proxy.pac following TLS bypass list",
        ),
        || false,
        move || {
            let base = share_base(&controls).0;
            set_clipboard(format!("{base}/proxy.pac"));
            set_status(&controls, StatusMsg::CopiedUrl);
            open.set(false);
        },
    )
}

fn hub_section_label(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg_muted())
}

fn hub_action_row(
    icon: Glyph,
    title: impl IntoReactive<SharedString>,
    subtitle: impl IntoReactive<SharedString>,
    active: impl Fn() -> bool + 'static + Clone,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    hub_action_row_reactive(icon, title, subtitle, active, on_press)
}

fn hub_action_row_reactive(
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
        .w_full()
        .px_2p5()
        .py_1p5()
        .gap_2()
        .flex_row()
        .items_center()
        .rounded(px(7.0))
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
        .child(glyph(
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
                .gap(px(1.0))
                .child(
                    text(title)
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .text_size(px(12.0))
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
                        .text_size(px(10.5))
                        .text_color(fg_muted()),
                ),
        )
}
