#[allow(unused_imports)]
use super::*;

pub(crate) fn footer(
    listen: Signal<String>,
    running: Signal<bool>,
    memory: Signal<String>,
    dropped: Signal<u64>,
    port_text: Signal<String>,
    lan_url: Signal<String>,
    lan_choice: Signal<String>,
    lan_addrs: Signal<Vec<LanAddr>>,
    qr_path: Signal<String>,
    listen_all: Signal<bool>,
    proxy_auth: Signal<String>,
    config_broken: Signal<Option<String>>,
    config_broken_dismissed: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let port_controls = controls.clone();
    let lan_controls = controls.clone();
    let qr_controls = controls.clone();
    let auth_controls = controls;
    view()
        .h(px(32.0))
        .pl(px(LIST_INSET))
        .pr_3()
        .gap_3()
        .flex_row()
        .items_center()
        .bg(bg_chrome())
        .border_t_1()
        .border_t_color(border_strong())
        .child(
            view()
                .flex_1()
                .flex_row()
                .items_center()
                .gap_2()
                .min_w(px(0.0))
                .overflow_hidden()
                .child(
                    view()
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded_full()
                        .flex_shrink_0()
                        .bg(move || {
                            if running.get() {
                                status_ok_fg()
                            } else {
                                fg_muted()
                            }
                        }),
                )
                .child(
                    when(
                        move || listen.get().is_empty(),
                        move || {
                            text(tr("监听尚未开始", "Not listening"))
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Tail)
                                .text_size(px(11.5))
                                .text_color(fg_muted())
                        },
                    )
                    .otherwise(move || {
                        view()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .child(
                                text(tr("监听", "Listening"))
                                    .flex_shrink_0()
                                    .number_of_lines(1)
                                    .text_size(px(11.5))
                                    .text_color(fg_muted()),
                            )
                            .child(
                                mono(move || listen.get())
                                    .min_w(px(0.0))
                                    .tabular_nums()
                                    .number_of_lines(1)
                                    .text_overflow(EllipsizeMode::Tail)
                                    .text_size(px(11.5))
                                    .text_color(fg_muted()),
                            )
                    }),
                )
                .child(when(
                    move || config_broken.get().is_some(),
                    move || {
                        view()
                            .h(px(20.0))
                            .px_1p5()
                            .flex_shrink_0()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .rounded(px(5.0))
                            .border_1()
                            .border_color(status_warn_fg())
                            .bg(status_warn_bg())
                            .hover(|style| style.opacity(0.85))
                            .active(|style| style.opacity(0.75))
                            .cursor(Cursor::Pointer)
                            .tooltip(
                                tooltip().child(
                                    view()
                                        .max_w(px(420.0))
                                        .px_2()
                                        .py_1()
                                        .rounded(px(6.0))
                                        .bg(bg_card())
                                        .border_1()
                                        .border_color(border_strong())
                                        .child(
                                            text(move || config_broken.get().unwrap_or_default())
                                                .text_size(px(11.5))
                                                .text_color(fg_primary()),
                                        ),
                                ),
                            )
                            .on_press(move || config_broken_dismissed.set(false))
                            .child(glyph_px(12.0, || Glyph::Alert, || color(status_warn_fg())))
                            .child(
                                text(tr("配置错误", "Config error"))
                                    .number_of_lines(1)
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(status_warn_fg()),
                            )
                    },
                )),
        )
        .child(when(
            move || running.get(),
            move || {
                cert_qr(
                    listen,
                    lan_url,
                    lan_choice,
                    lan_addrs,
                    qr_path,
                    listen_all,
                    qr_controls.clone(),
                )
            },
        ))
        .child(when(
            move || !running.get(),
            move || port_field(port_text, port_controls.clone()),
        ))
        .child(when(
            move || !running.get(),
            move || lan_toggle(listen_all, lan_controls.clone()),
        ))
        .child(proxy_auth_popover(proxy_auth, auth_controls))
        .child(
            view()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_1()
                .child(
                    mono(move || memory.get())
                        .tabular_nums()
                        .text_size(px(11.5))
                        .text_color(fg_muted()),
                )
                .child(
                    text(tr("· 已丢弃", "· dropped"))
                        .text_size(px(11.5))
                        .text_color(fg_muted()),
                )
                .child(
                    mono(move || dropped.get().to_string())
                        .tabular_nums()
                        .text_size(px(11.5))
                        .text_color(fg_muted()),
                ),
        )
}

pub(crate) fn proxy_auth_popover(
    proxy_auth: Signal<String>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let open = signal(false);
    let active = move || !proxy_auth.get().trim().is_empty();
    let active_dot = active;
    let active_badge = active;
    let active_badge_text = active;
    let clear_controls = controls.clone();
    popover()
        .placement(Placement::Top)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(8.0))
        .on_open_change(move |event: &nsh_base::PopoverEvent| {
            if let nsh_base::PopoverEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .trigger(
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
                    if open.get() || active() {
                        accent_blue_border()
                    } else {
                        border_control()
                    }
                })
                .bg(move || {
                    if open.get() {
                        accent_blue_soft()
                    } else {
                        bg_control()
                    }
                })
                .hover(|style| {
                    style.bg(move || {
                        if open.get() {
                            accent_blue_soft_hover()
                        } else {
                            bg_control_hover()
                        }
                    })
                })
                .active(|style| style.opacity(0.8))
                .cursor(Cursor::Pointer)
                .child(
                    view()
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded_full()
                        .bg(move || {
                            if active_dot() {
                                status_ok_fg()
                            } else {
                                fg_muted()
                            }
                        }),
                )
                .child(
                    text(tr("认证", "Auth"))
                        .number_of_lines(1)
                        .text_size(px(11.5))
                        .text_color(move || {
                            if open.get() {
                                accent_blue_text()
                            } else {
                                fg_secondary()
                            }
                        }),
                ),
        )
        .child(
            view()
                .w(px(300.0))
                .p_3()
                .gap_2()
                .rounded(px(10.0))
                .border_1()
                .border_color(border_strong())
                .bg(bg_card())
                .shadow_md()
                .child(
                    view()
                        .w_full()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(
                            text(tr("代理认证", "Proxy auth"))
                                .text_size(px(12.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(fg_primary()),
                        )
                        .child(
                            text({
                                let langs = ui_langs();
                                move || -> SharedString {
                                    if active_badge_text() {
                                        t(ui_lang(langs), "已开启", "Enabled").into()
                                    } else {
                                        t(ui_lang(langs), "未开启", "Off").into()
                                    }
                                }
                            })
                            .text_size(px(11.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || {
                                if active_badge() {
                                    status_ok_fg()
                                } else {
                                    fg_muted()
                                }
                            }),
                        ),
                )
                .child(
                    text(tr(
                        "格式为 user:pass，留空表示不校验。修改后立即生效（证书安装页 /zniff 豁免）。",
                        "Format: user:pass. Leave empty to disable. Takes effect immediately (/zniff is exempt).",
                    ))
                    .text_size(px(11.5))
                    .line_height(Points(17.0))
                    .text_color(fg_muted()),
                )
                .child(
                    view()
                        .w_full()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .child(
                            input()
                                .value(proxy_auth)
                                .placeholder(tr(
                                    "user:pass，空着不校验",
                                    "user:pass, empty disables auth",
                                ))
                                .placeholder_color(move || color(fg_muted()))
                                .flex_1()
                                .min_w(px(0.0))
                                .h(px(28.0))
                                .px_2()
                                .rounded(px(6.0))
                                .bg(bg_code())
                                .border_1()
                                .border_color(border_control())
                                .focus(|style| style.border_color(accent_blue()))
                                .text_size(px(12.0))
                                .text_color(fg_primary())
                                .on_change(move |text: &str| {
                                    proxy_auth.set(text.to_string());
                                    let stored = text.trim().to_string();
                                    controls.hub.set_proxy_auth(&stored);
                                    if let Err(err) =
                                        save_config(|config| config.proxy_auth = stored)
                                    {
                                        set_status(
                                            &controls,
                                            StatusMsg::PrefUnsaved {
                                                name: PrefName::ProxyAuth,
                                                err: err.to_string(),
                                            },
                                        );
                                    }
                                }),
                        )
                        .child(when(
                            move || !proxy_auth.get().is_empty(),
                            move || {
                                let clear_controls = clear_controls.clone();
                                view()
                                    .h(px(28.0))
                                    .px_2()
                                    .rounded(px(6.0))
                                    .border_1()
                                    .border_color(border_control())
                                    .bg(bg_control())
                                    .hover(|s| s.bg(bg_control_hover()))
                                    .active(|s| s.opacity(0.8))
                                    .cursor(Cursor::Pointer)
                                    .items_center()
                                    .justify_center()
                                    .on_press(move || {
                                        proxy_auth.set(String::new());
                                        clear_controls.hub.set_proxy_auth("");
                                        if let Err(err) =
                                            save_config(|config| config.proxy_auth.clear())
                                        {
                                            set_status(
                                                &clear_controls,
                                                StatusMsg::PrefUnsaved {
                                                    name: PrefName::ProxyAuth,
                                                    err: err.to_string(),
                                                },
                                            );
                                        }
                                    })
                                    .child(
                                        text(tr("清空", "Clear"))
                                            .text_size(px(11.5))
                                            .text_color(fg_secondary()),
                                    )
                            },
                        )),
                ),
        )
}

pub(crate) fn cert_qr(
    listen: Signal<String>,
    lan_url: Signal<String>,
    lan_choice: Signal<String>,
    lan_addrs: Signal<Vec<LanAddr>>,
    qr_path: Signal<String>,
    listen_all: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let open = signal(false);
    popover()
        .placement(Placement::Top)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(8.0))
        .on_open_change(move |event: &nsh_base::PopoverEvent| {
            if let nsh_base::PopoverEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .trigger(qr_trigger(listen, lan_url, lan_choice, open))
        .child(qr_panel(
            listen, lan_url, lan_choice, lan_addrs, qr_path, listen_all, controls,
        ))
}

pub(crate) fn qr_trigger(
    listen: Signal<String>,
    lan_url: Signal<String>,
    choice: Signal<String>,
    open: Signal<bool>,
) -> impl IntoElement {
    let has_lan = move || is_listening_lan(&listen.get()) && !lan_url.get().is_empty();
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
            if open.get() || has_lan() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if open.get() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if open.get() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.75))
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .bg(move || if has_lan() { accent_blue() } else { fg_muted() }),
        )
        .child(
            text({
                let langs = ui_langs();
                move || -> SharedString {
                    let label = t(ui_lang(langs), "扫码", "Scan");
                    if has_lan() {
                        format!("{label} {}", choice.get()).into()
                    } else {
                        label.into()
                    }
                }
            })
            .tabular_nums()
            .number_of_lines(1)
            .text_size(px(11.5))
            .text_color(move || {
                if open.get() {
                    accent_blue_text()
                } else {
                    fg_secondary()
                }
            }),
        )
}

pub(crate) fn qr_panel(
    listen: Signal<String>,
    lan_url: Signal<String>,
    lan_choice: Signal<String>,
    lan_addrs: Signal<Vec<LanAddr>>,
    qr_path: Signal<String>,
    listen_all: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    view()
        .w(move || {
            let max_w = if is_listening_lan(&listen.get()) {
                612.0
            } else {
                380.0
            };
            px((window().width.0 - 20.0).clamp(280.0, max_w))
        })
        .max_h(move || px((window().height.0 - 50.0).max(200.0)))
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
        .overflow_hidden()
        .child(
            scroll()
                .w_full()
                .max_h(move || px((window().height.0 - 52.0).max(198.0)))
                .child(
                    view()
                        .w_full()
                        .p_3()
                        .gap_2()
                        .items_center()
                        .child(
                            text(tr("手机安装页", "Phone install page"))
                                .text_size(px(12.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(fg_secondary()),
                        )
                        .child(
                            when(
                                move || !is_listening_lan(&listen.get()),
                                move || qr_local_only_notice(listen, listen_all, controls.clone()),
                            )
                            .otherwise(move || {
                                view()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        view()
                                            .w_full()
                                            .p_2()
                                            .rounded(px(6.0))
                                            .bg(bg_code())
                                            .border_1()
                                            .border_color(border_strong())
                                            .child(
                                                mono(move || lan_url.get())
                                                    .selectable(true)
                                                    .number_of_lines(1)
                                                    .text_overflow(EllipsizeMode::Middle)
                                                    .text_size(px(12.0))
                                                    .text_color(fg_primary()),
                                            ),
                                    )
                                    .child(
                                        view()
                                            .w_full()
                                            .flex_row()
                                            .flex_wrap()
                                            .items_stretch()
                                            .gap_3()
                                            .child(qr_code_side(
                                                listen, lan_url, lan_choice, lan_addrs, qr_path,
                                            ))
                                            .child(qr_phone_steps()),
                                    )
                            }),
                        ),
                ),
        )
}

pub(crate) fn qr_local_only_notice(
    listen: Signal<String>,
    listen_all: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    view()
        .w_full()
        .gap_2p5()
        .child(
            view()
                .w_full()
                .p_3()
                .rounded(px(8.0))
                .bg(status_warn_bg())
                .border_1()
                .border_color(status_warn_fg())
                .gap_1p5()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .child(glyph_px(14.0, || Glyph::Alert, || color(status_warn_fg())))
                        .child(
                            text(tr("当前仅监听本机", "Listening on localhost only"))
                                .text_size(px(12.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(fg_primary()),
                        ),
                )
                .child(
                    text({
                        let langs = ui_langs();
                        move || -> SharedString {
                            let addr = listen.get();
                            match ui_lang(langs) {
                                Lang::Zh => format!(
                                    "代理当前只监听 {addr}，手机等局域网设备无法连接。请切换为「局域网监听 (0.0.0.0)」后再扫码。"
                                )
                                .into(),
                                Lang::En => format!(
                                    "The proxy is bound to {addr}, so LAN devices cannot reach it. Switch to LAN listening (0.0.0.0) to scan and connect."
                                )
                                .into(),
                            }
                        }
                    })
                    .text_size(px(12.0))
                    .line_height(Points(18.0))
                    .text_color(fg_secondary()),
                ),
        )
        .child(
            view()
                .w_full()
                .h(px(30.0))
                .px_3()
                .rounded(px(6.0))
                .border_1()
                .border_color(accent_blue_border())
                .bg(accent_blue_soft())
                .hover(|style| style.bg(accent_blue_soft_hover()))
                .active(|style| style.opacity(0.8))
                .cursor(Cursor::Pointer)
                .items_center()
                .justify_center()
                .on_press(move || {
                    listen_all.set(true);
                    switch_to_lan(controls.clone());
                })
                .child(
                    text(tr("切换为局域网监听", "Switch to LAN listening"))
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(accent_blue_text()),
                ),
        )
}

pub(crate) fn qr_code_side(
    listen: Signal<String>,
    lan_url: Signal<String>,
    lan_choice: Signal<String>,
    lan_addrs: Signal<Vec<LanAddr>>,
    qr_path: Signal<String>,
) -> impl IntoElement {
    view()
        .w(px(236.0))
        .max_w(move || px((window().width.0 - 44.0).clamp(200.0, 236.0)))
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .child(
            view().w_full().child(
                text({
                    let langs = ui_langs();
                    move || -> SharedString {
                        let lang = ui_lang(langs);
                        let ip = lan_choice.get();
                        let name = lan_addrs.with(|addrs| {
                            addrs
                                .iter()
                                .find(|addr| addr.ip.to_string() == ip)
                                .map(|addr| addr.name.clone())
                                .unwrap_or_default()
                        });
                        if lan_addrs.with(Vec::len) > 1 {
                            t(
                                lang,
                                "点下面换一张网卡。手机要和这个地址在同一网络。",
                                "Pick a network below. The phone must be on that network.",
                            )
                            .into()
                        } else if name.is_empty() {
                            t(
                                lang,
                                "手机要能访问这个地址。",
                                "The phone must be able to reach this address.",
                            )
                            .into()
                        } else if lang == Lang::Zh {
                            format!("网卡 {name}。手机要和它在同一网络。").into()
                        } else {
                            format!("Interface {name}. The phone must be on that network.").into()
                        }
                    }
                })
                .text_size(px(12.0))
                .text_color(fg_muted()),
            ),
        )
        .child(when(
            move || lan_addrs.with(Vec::len) > 1,
            move || {
                view().w_full().gap_1().child(each(
                    move || lan_addrs.get(),
                    |addr: &LanAddr| addr.ip.to_string(),
                    move |addr, _index| {
                        lan_choice_row(addr, listen, lan_addrs, lan_choice, lan_url, qr_path)
                    },
                ))
            },
        ))
        .child(
            view()
                .w_full()
                .flex_grow(1.0)
                .flex_shrink_0()
                .pt_2()
                .items_center()
                .justify_center()
                .child(when(
                    move || !qr_path.get().is_empty(),
                    move || {
                        view()
                            .p_2()
                            .rounded(px(8.0))
                            .bg(AdaptiveColor::rgb(0xffffff, 0xffffff))
                            .child(image(move || qr_path.get()).w(px(168.0)).h(px(168.0)))
                    },
                ))
                .child(when(
                    move || qr_path.get().is_empty(),
                    move || muted_line(tr("二维码没有生成", "QR code was not generated")),
                )),
        )
}

/// Steps shown beside the QR. Same path as the phone install page, plus the
/// Wi-Fi proxy the page only mentions in one sentence.
pub(crate) fn qr_phone_steps() -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(240.0))
        .gap_3()
        .child(
            view()
                .w_full()
                .gap_1p5()
                .child(qr_os_heading("iOS"))
                .child(
                    view()
                        .w_full()
                        .gap_2()
                        .child(qr_step(
                            "1.",
                            tr(
                                "用系统相机扫码并在 Safari 中打开安装页，点「下载根证书」并允许下载描述文件（勿用微信或第三方浏览器）。",
                                "Scan with Camera to open the page in Safari, tap Download the root certificate, and allow the configuration profile (other browsers only save the file).",
                            ),
                        ))
                        .child(qr_step(
                            "2.",
                            tr(
                                "打开「设置」→「已下载描述文件」（或「设置」→「通用」→「VPN 与设备管理」），安装 zniff CA。",
                                "Open Settings → Profile Downloaded (or Settings → General → VPN & Device Management) and install zniff CA.",
                            ),
                        ))
                        .child(qr_step(
                            "3.",
                            tr(
                                "安装完成后，才会在「设置」→「通用」→「关于本机」→「证书信任设置」里看到 zniff CA，打开它的完全信任。",
                                "Once installed, zniff CA appears in Settings → General → About → Certificate Trust Settings. Enable full trust for it.",
                            ),
                        ))
                        .child(qr_step(
                            "4.",
                            tr(
                                "「设置」→「无线局域网」→ 当前网络右侧 ⓘ →「配置代理」→「手动」。服务器和端口填上面的地址。",
                                "Settings → Wi-Fi → the ⓘ beside this network → Configure Proxy → Manual. Use the server and port above.",
                            ),
                        )),
                ),
        )
        .child(
            view()
                .w_full()
                .gap_1p5()
                .child(qr_os_heading("Android"))
                .child(
                    view()
                        .w_full()
                        .gap_2()
                        .child(qr_step(
                            "1.",
                            tr(
                                "扫码打开安装页，点下载并保存 zniff-ca.crt。",
                                "Scan to open the page, then download and save zniff-ca.crt.",
                            ),
                        ))
                        .child(qr_step(
                            "2.",
                            tr(
                                "打开「设置」→「安全」→「加密与凭据」→「安装证书」→「CA 证书」。",
                                "Open Settings → Security → Encryption & credentials → Install a certificate → CA certificate.",
                            ),
                        ))
                        .child(qr_step(
                            "3.",
                            tr(
                                "打开当前 WLAN 的详情，或「修改网络」→「高级选项」。代理选「手动」，主机名和端口填上面的地址。",
                                "Open this WLAN's details, or Modify network → Advanced. Set Proxy to Manual and use the hostname and port above.",
                            ),
                        )),
                ),
        )
}

pub(crate) fn qr_step(
    index: &'static str,
    body: impl IntoReactive<SharedString> + 'static,
) -> impl IntoElement {
    view()
        .w_full()
        .flex_row()
        .items_start()
        .gap_1p5()
        .child(
            view().w(px(18.0)).flex_shrink_0().child(
                text(index)
                    .text_size(px(12.0))
                    .line_height(Points(18.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(fg_secondary()),
            ),
        )
        .child(
            view().flex_1().min_w(px(0.0)).child(
                text(body)
                    .text_size(px(12.0))
                    .line_height(Points(18.0))
                    .text_color(fg_muted()),
            ),
        )
}

pub(crate) fn qr_os_heading(title: &'static str) -> Text {
    text(title)
        .text_size(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg_primary())
}

pub(crate) fn lan_choice_row(
    addr: ReadSignal<LanAddr>,
    listen: Signal<String>,
    lan_addrs: Signal<Vec<LanAddr>>,
    lan_choice: Signal<String>,
    lan_url: Signal<String>,
    qr_path: Signal<String>,
) -> impl IntoElement {
    let picked = move || lan_choice.get() == addr.with(|item| item.ip.to_string());
    let picked_bg = picked;
    let picked_dot = picked;
    view()
        .h(px(28.0))
        .px_2()
        .w_full()
        .flex_row()
        .items_center()
        .gap_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || {
            if picked() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if picked_bg() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if picked() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .on_press(move || {
            let ip = addr.with(|item| item.ip.to_string());
            publish_lan(&listen.get(), &ip, lan_addrs, lan_choice, lan_url, qr_path);
        })
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .flex_shrink_0()
                .bg(move || {
                    if picked_dot() {
                        accent_blue()
                    } else {
                        fg_muted()
                    }
                }),
        )
        .child(
            mono(move || -> SharedString {
                let item = addr.get();
                if item.name.is_empty() {
                    item.ip.to_string().into()
                } else {
                    format!("{}  {}", item.name, item.ip).into()
                }
            })
            .flex_1()
            .min_w(px(0.0))
            .number_of_lines(1)
            .text_overflow(EllipsizeMode::Tail)
            .text_size(px(12.0))
            .text_color(fg_primary()),
        )
}

pub(crate) fn port_field(port_text: Signal<String>, controls: Arc<Controls>) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_1p5()
        .flex_shrink_0()
        .child(
            text(tr("端口", "Port"))
                .text_size(px(11.5))
                .text_color(fg_muted()),
        )
        .child(
            input()
                .value(port_text)
                .placeholder(config::default_listen_port(config::is_dev_mode()).to_string())
                .placeholder_color(move || color(fg_muted()))
                .w(px(72.0))
                .h(px(22.0))
                .px_2()
                .rounded(px(6.0))
                .bg(bg_code())
                .border_1()
                .border_color(border_control())
                .focus(|style| style.border_color(accent_blue()))
                .text_size(px(12.0))
                .text_color(fg_primary())
                .on_change(move |text: &str| {
                    port_text.set(text.to_string());
                    if let Some(port) = present::parse_port(text) {
                        set_port(&controls.listen_port, port);
                        if let Err(err) = store_listen_port(port) {
                            set_status(
                                &controls,
                                StatusMsg::PrefUnsaved {
                                    name: PrefName::Port,
                                    err: err.to_string(),
                                },
                            );
                        }
                    }
                }),
        )
}

pub(crate) fn control_text(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .number_of_lines(1)
        .text_size(px(CONTROL_TEXT))
        .font_weight(FontWeight::MEDIUM)
}
