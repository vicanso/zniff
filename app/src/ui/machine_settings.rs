#[allow(unused_imports)]
use super::*;

/// Modal dialog for machine-wide service & relay settings that live outside saved profiles.
pub(crate) fn machine_settings_modal(
    profiles: ProfileSignals,
    controls: Arc<Controls>,
) -> impl IntoElement {
    when(
        move || profiles.machine_settings_open.get(),
        move || {
            let proxy_auth_text = profiles.proxy_auth;
            let relay_url_text = profiles.relay_url;
            let relay_token_text = signal(lock_string(&controls.relay_token));
            let relay_accept_on = profiles.relay_accept;
            let proxy_auth_controls = controls.clone();
            let relay_url_controls = controls.clone();
            let relay_token_controls = controls.clone();
            let relay_accept_controls = controls.clone();

            let backdrop = view()
                .absolute()
                .inset_0()
                .bg(AdaptiveColor::new(
                    rgba(0x0F172A5C),
                    rgba(0x0000008F),
                ))
                .on_press(move || profiles.machine_settings_open.set(false));

            let header = view()
                .w_full()
                .px_4()
                .py_3()
                .flex_row()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_b_color(border_subtle())
                .child(glyph_px(
                    14.0,
                    || Glyph::Sliders,
                    || color(accent_blue_text()),
                ))
                .child(
                    text(tr("服务与中继设置", "Relay & Service Settings"))
                        .flex_1()
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg_primary()),
                )
                .child(
                    view()
                        .w(px(24.0))
                        .h(px(24.0))
                        .rounded(px(6.0))
                        .flex_shrink_0()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(bg_control_hover()))
                        .active(|s| s.opacity(0.75))
                        .cursor(Cursor::Pointer)
                        .label(tr("关闭", "Close"))
                        .on_press(move || profiles.machine_settings_open.set(false))
                        .child(glyph_px(11.0, || Glyph::Close, || color(fg_muted()))),
                );

            let body = view()
                .w_full()
                .p_4()
                .gap_3()
                .flex_col()
                .child(
                    view()
                        .w_full()
                        .px_3()
                        .py_2()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(border_subtle())
                        .bg(bg_chrome())
                        .child(
                            text(tr(
                                "以下为本机全局设置，修改后立即生效并自动保存，不随配置方案（Profile）切换或导出。",
                                "Machine-wide settings. Applied and saved immediately, independent of profiles.",
                            ))
                            .text_size(px(11.5))
                            .text_color(fg_secondary()),
                        ),
                )
                .child(section_heading(tr("中转分享", "Live-share Relay")))
                .child(
                    view()
                        .w_full()
                        .p_3()
                        .gap_2p5()
                        .flex_col()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(border_subtle())
                        .bg(bg_canvas())
                        .child(config_field(
                            "中转地址",
                            "Relay address",
                            side_input(
                                relay_url_text,
                                tr(
                                    "http(s)://主机:端口[/前缀]，空着只在本机分享",
                                    "http(s)://host:port[/prefix], empty shares locally",
                                ),
                                30.0,
                                false,
                                move |text| {
                                    relay_url_text.set(text.to_string());
                                    let stored = text.trim().to_string();
                                    *relay_url_controls
                                        .relay_url
                                        .lock()
                                        .unwrap_or_else(|err| err.into_inner()) = stored.clone();
                                    if let Err(err) =
                                        save_config(|config| config.relay_url = stored)
                                    {
                                        set_status(
                                            &relay_url_controls,
                                            StatusMsg::PrefUnsaved {
                                                name: PrefName::Relay,
                                                err: err.to_string(),
                                            },
                                        );
                                    }
                                },
                            ),
                        ))
                        .child(config_field(
                            "中转密钥",
                            "Relay token",
                            side_input(
                                relay_token_text,
                                tr(
                                    "上传和接收用同一个密钥",
                                    "Same secret for upload and receive",
                                ),
                                30.0,
                                false,
                                move |text| {
                                    relay_token_text.set(text.to_string());
                                    let stored = text.trim().to_string();
                                    *relay_token_controls
                                        .relay_token
                                        .lock()
                                        .unwrap_or_else(|err| err.into_inner()) = stored.clone();
                                    relay_token_controls.hub.set_relay_token(&stored);
                                    if let Err(err) =
                                        save_config(|config| config.relay_token = stored)
                                    {
                                        set_status(
                                            &relay_token_controls,
                                            StatusMsg::PrefUnsaved {
                                                name: PrefName::Relay,
                                                err: err.to_string(),
                                            },
                                        );
                                    }
                                },
                            ),
                        ))
                        .child(
                            view().w_full().flex_row().pt_0p5().child(enable_switch(
                                relay_accept_on,
                                "接收中继",
                                "Accept relay",
                                move |on| {
                                    relay_accept_controls.hub.set_relay_accept(on);
                                    if on && !relay_accept_controls.hub.relay_token_ready() {
                                        set_status(
                                            &relay_accept_controls,
                                            StatusMsg::RelayAcceptNeedToken,
                                        );
                                    }
                                    if let Err(err) =
                                        save_config(|config| config.relay_accept = on)
                                    {
                                        set_status(
                                            &relay_accept_controls,
                                            StatusMsg::PrefUnsaved {
                                                name: PrefName::Relay,
                                                err: err.to_string(),
                                            },
                                        );
                                    }
                                },
                            )),
                        ),
                )
                .child(section_heading(tr("代理接入", "Proxy Access")))
                .child(
                    view()
                        .w_full()
                        .p_3()
                        .flex_col()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(border_subtle())
                        .bg(bg_canvas())
                        .child(config_field(
                            "代理认证",
                            "Proxy auth",
                            side_input(
                                proxy_auth_text,
                                tr(
                                    "user:pass，空着不校验",
                                    "user:pass, empty disables auth",
                                ),
                                30.0,
                                false,
                                move |text| {
                                    proxy_auth_text.set(text.to_string());
                                    let stored = text.trim().to_string();
                                    proxy_auth_controls.hub.set_proxy_auth(&stored);
                                    if let Err(err) =
                                        save_config(|config| config.proxy_auth = stored)
                                    {
                                        set_status(
                                            &proxy_auth_controls,
                                            StatusMsg::PrefUnsaved {
                                                name: PrefName::ProxyAuth,
                                                err: err.to_string(),
                                            },
                                        );
                                    }
                                },
                            ),
                        )),
                );

            let footer = view()
                .w_full()
                .px_4()
                .py_2p5()
                .flex_row()
                .items_center()
                .justify_end()
                .border_t_1()
                .border_t_color(border_subtle())
                .bg(bg_chrome())
                .child(
                    view()
                        .h(px(28.0))
                        .px_3p5()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(accent_blue_border())
                        .bg(accent_blue_soft())
                        .hover(|s| s.bg(accent_blue_soft_hover()))
                        .active(|s| s.opacity(0.8))
                        .cursor(Cursor::Pointer)
                        .on_press(move || profiles.machine_settings_open.set(false))
                        .child(
                            control_text(tr("完成", "Done"))
                                .text_color(accent_blue_text()),
                        ),
                );

            let card = view()
                .w(move || px((window().width.0 - 32.0).clamp(320.0, 460.0)))
                .max_h(move || px((window().height.0 - 56.0).max(240.0)))
                .flex_col()
                .overflow_hidden()
                .rounded(px(12.0))
                .border_1()
                .border_color(border_strong())
                .bg(bg_card())
                .shadow_lg()
                .child(header)
                .child(
                    scroll()
                        .w_full()
                        .max_h(move || px((window().height.0 - 156.0).max(160.0)))
                        .child(body),
                )
                .child(footer);

            overlay().inset_0().child(
                view()
                    .w_full()
                    .h_full()
                    .items_center()
                    .justify_center()
                    .child(backdrop)
                    .child(card),
            )
        },
    )
}

fn section_heading(title: impl IntoReactive<SharedString>) -> impl IntoElement {
    text(title)
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg_muted())
}
