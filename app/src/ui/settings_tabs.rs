//! Tab content builders for Certificate, Network & Upstream, Security, and Relay settings.

#[allow(unused_imports)]
use super::*;

pub(crate) fn settings_card(
    title_zh: &'static str,
    title_en: &'static str,
    body: impl IntoElement,
) -> impl IntoElement {
    view()
        .w_full()
        .gap_1p5()
        .flex_col()
        .child(
            text(tr(title_zh, title_en))
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_muted()),
        )
        .child(
            view()
                .w_full()
                .p_2p5()
                .gap_2()
                .flex_col()
                .rounded(px(8.0))
                .border_1()
                .border_color(border_subtle())
                .bg(bg_canvas())
                .child(body),
        )
}

pub(crate) fn settings_toggle_row(
    zh: &'static str,
    en: &'static str,
    marked: impl Fn() -> bool + 'static + Clone,
    on_toggle: impl Fn() + 'static,
) -> impl IntoElement {
    let dot_border = marked.clone();
    let dot_bg = marked.clone();
    view()
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .hover(|s| s.bg(bg_control_hover()))
        .active(|s| s.opacity(0.8))
        .cursor(Cursor::Pointer)
        .on_press(on_toggle)
        .child(
            view()
                .w(px(13.0))
                .h(px(13.0))
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .justify_center()
                .child(
                    view()
                        .w(px(10.0))
                        .h(px(10.0))
                        .rounded(px(2.5))
                        .border_1()
                        .border_color(move || {
                            if dot_border() {
                                accent_blue()
                            } else {
                                border_control()
                            }
                        })
                        .bg(move || if dot_bg() { accent_blue() } else { bg_card() }),
                ),
        )
        .child(control_text(tr(zh, en)).text_color(move || {
            if marked() {
                accent_blue_text()
            } else {
                fg_secondary()
            }
        }))
}

pub(crate) fn settings_certificate_tab(props: SettingsWindowProps) -> impl IntoElement {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let open = props.profiles.settings_open;
        let controls = props.controls;
        let ca_ui = props.ca_ui;
        let install = props.install_ca;
        let remove = props.remove_ca;
        let switch_controls = controls.clone();
        view()
            .w_full()
            .gap_3()
            .flex_col()
            .child(settings_card(
                "HTTPS 根证书",
                "HTTPS Root Certificate",
                view()
                    .w_full()
                    .gap_1()
                    .flex_col()
                    .child(cert_menu_row(
                        ca_ui,
                        Rc::new(move || (install)()),
                        Rc::new(move || (remove)()),
                        move || use_builtin_ca(switch_controls.clone()),
                    ))
                    .child(external_ca_menu_row(open, ca_ui, controls.clone()))
                    .child(platform_ca_extra_menu_item(&controls)),
            ))
            .child(
                text(tr(
                    "手机或模拟器证书安装请通过底栏「接入」中心扫码或访问代理端口的 /zniff 页面。",
                    "For mobile devices or emulators, use the footer Connect hub or visit /zniff on the proxy port.",
                ))
                .text_size(px(11.5))
                .line_height(Points(17.0))
                .text_color(fg_muted()),
            )
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = props;
        settings_card(
            "HTTPS 根证书",
            "HTTPS Root Certificate",
            text(tr(
                "Web 操作台模式下，证书由运行 `zniff --web` 的服务器管理。客户端可访问代理端口的 /zniff 页面下载安装根证书。",
                "In Web console mode, certificates are managed on the `zniff --web` host. Clients can install the root CA via /zniff on the proxy port.",
            ))
            .text_size(px(12.0))
            .line_height(Points(18.0))
            .text_color(fg_secondary()),
        )
    }
}

pub(crate) fn settings_network_tab(props: SettingsWindowProps) -> impl IntoElement {
    let controls = props.controls;
    let profiles = props.profiles;
    let upstream_text = profiles.upstream;
    let bypass_text = profiles.bypass;
    let show_tunnels = profiles.show_tunnels;
    let upstream_controls = controls.clone();
    let bypass_controls = controls.clone();
    let tunnel_controls = controls.clone();

    let listener_row = view()
        .w_full()
        .px_1()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_3()
        .child(port_field(props.port_text, controls.clone()));
    let listener_row = desktop_child!(listener_row, lan_toggle(props.listen_all, controls.clone()));

    view()
        .w_full()
        .gap_3()
        .flex_col()
        .child(settings_card("监听地址", "Capture Listener", listener_row))
        .child(settings_card(
            "上游代理",
            "Upstream Proxy",
            side_input(
                upstream_text,
                tr(
                    "http://127.0.0.1:7890 或 socks5h://127.0.0.1:1080",
                    "http://127.0.0.1:7890 or socks5h://127.0.0.1:1080",
                ),
                30.0,
                false,
                move |text| {
                    upstream_text.set(text.to_string());
                    upstream_controls.hub.set_upstream(text);
                    save_config_or_notify(&upstream_controls, PrefName::Upstream, |cfg| {
                        cfg.upstream_proxy = text.to_string();
                    });
                },
            ),
        ))
        .child(settings_card(
            "TLS 直通名单",
            "TLS Bypass",
            view()
                .w_full()
                .gap_2()
                .flex_col()
                .child(side_input(
                    bypass_text,
                    tr(
                        "每行一个域名，如 *.apple.com 或 only:api.example.com",
                        "One host per line, e.g. *.apple.com or only:api.example.com",
                    ),
                    72.0,
                    true,
                    move |text| {
                        bypass_text.set(text.to_string());
                        bypass_controls.hub.set_bypass(text);
                        save_config_or_notify(&bypass_controls, PrefName::Bypass, |cfg| {
                            cfg.bypass_hosts = text.to_string();
                        });
                    },
                ))
                .child(settings_toggle_row(
                    "显示隧道失败",
                    "Show tunnel failures",
                    move || show_tunnels.get(),
                    move || {
                        let next = !show_tunnels.get();
                        show_tunnels.set(next);
                        tunnel_controls.hub.set_show_tunnel_failures(next);
                        save_config_or_notify(&tunnel_controls, PrefName::Tunnel, |cfg| {
                            cfg.show_tunnel_failures = next;
                        });
                    },
                )),
        ))
}

pub(crate) fn settings_security_tab(
    props: SettingsWindowProps,
    close_on_preview: bool,
) -> impl IntoElement {
    let profiles = props.profiles;
    let controls = props.controls;
    let allow_controls = controls.clone();
    let redact_controls = controls.clone();
    let strict_controls = controls.clone();
    let preview_controls = controls.clone();
    let seq_builder = props.seq_builder;

    view()
        .w_full()
        .gap_3()
        .flex_col()
        .child(settings_card(
            "代理访问控制",
            "Proxy Access Control",
            view()
                .w_full()
                .gap_2p5()
                .flex_col()
                .child(proxy_auth_section(profiles.proxy_auth, controls))
                .child(view().w_full().h(px(1.0)).bg(border_subtle()))
                .child(allow_hosts_section(
                    profiles.allow_hosts,
                    profiles.allow_hosts_strict,
                    profiles.allow_hosts_locked,
                    Rc::new(move |raw: &str| {
                        apply_allow_hosts_change(&allow_controls, profiles.allow_hosts, raw);
                    }),
                )),
        ))
        .child(settings_card(
            "导出与分享脱敏",
            "Export & Share Redaction",
            view()
                .w_full()
                .gap_1p5()
                .flex_col()
                .child(settings_toggle_row(
                    "导出/分享时自动脱敏敏感头与字段",
                    "Redact sensitive headers & fields on export/share",
                    move || seq_builder.redact_export.get(),
                    move || {
                        let next = !seq_builder.redact_export.get();
                        seq_builder.redact_export.set(next);
                        *redact_controls
                            .redact_export
                            .lock()
                            .unwrap_or_else(|err| err.into_inner()) = next;
                        save_config_or_notify(&redact_controls, PrefName::Redact, |cfg| {
                            cfg.redact_export = next;
                        });
                    },
                ))
                .child(settings_toggle_row(
                    "严格遮罩（隐藏无法结构化脱敏的二进制/非结构化正文）",
                    "Strict mask (omit unstructured or binary bodies)",
                    move || seq_builder.redact_strict.get(),
                    move || {
                        let next = !seq_builder.redact_strict.get();
                        seq_builder.redact_strict.set(next);
                        *strict_controls
                            .redact_strict
                            .lock()
                            .unwrap_or_else(|err| err.into_inner()) = next;
                        save_config_or_notify(&strict_controls, PrefName::Redact, |cfg| {
                            cfg.redact_strict = next;
                        });
                    },
                ))
                .child(
                    view()
                        .w_full()
                        .flex_row()
                        .justify_end()
                        .pt_0p5()
                        .child(menu_inline_pill(
                            tr(
                                "脱敏预览与自定义字段…",
                                "Redaction preview & custom fields…",
                            ),
                            move || seq_builder.redact_open.get(),
                            move || {
                                if close_on_preview {
                                    profiles.settings_open.set(false);
                                }
                                begin_redact_preview(
                                    &preview_controls,
                                    props.rows,
                                    seq_builder,
                                    props.compose,
                                    props.selected,
                                    props.shown_phase,
                                );
                            },
                        )),
                ),
        ))
}

pub(crate) fn settings_relay_tab(props: SettingsWindowProps) -> impl IntoElement {
    view()
        .w_full()
        .gap_3()
        .flex_col()
        .child(relay_settings_card(props.profiles, props.controls))
}
