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
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_muted()),
        )
        .child(
            view()
                .w_full()
                .p_2p5()
                .gap_2()
                .flex_col()
                .rounded(px(RADIUS_LG))
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
    ui_switch_row(tr(zh, en), marked, move |_| on_toggle())
}

pub(crate) fn settings_certificate_tab(props: SettingsWindowProps) -> impl IntoElement {
    #[cfg(not(target_arch = "wasm32"))]
    {
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
                    .child(external_ca_menu_row(None, ca_ui, controls.clone()))
                    .child(platform_ca_extra_menu_item(&controls)),
            ))
            .child(
                text(tr(
                    "手机或模拟器证书安装请通过底栏「接入」中心扫码或访问代理端口的 /zniff 页面。",
                    "For mobile devices or emulators, use the footer Connect hub or visit /zniff on the proxy port.",
                ))
                .text_size(px(TEXT_SM))
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
            .text_size(px(TEXT_SM))
            .line_height(Points(18.0))
            .text_color(fg_secondary()),
        )
    }
}

fn listener_needs_restart(active_listen: &str, port_text: &str, listen_all: bool) -> bool {
    if active_listen.is_empty() {
        return false;
    }
    let Some(desired_port) = present::parse_port(port_text) else {
        return false;
    };
    let active_port = active_listen
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok());
    if active_port != Some(desired_port) {
        return true;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let active_all = active_listen.starts_with("0.0.0.0:");
        if active_all != listen_all {
            return true;
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = listen_all;
    false
}

fn restart_capture_banner(
    controls: Arc<Controls>,
    langs: Signal<Option<Lang>>,
) -> impl IntoElement {
    view()
        .w_full()
        .px_2()
        .py_1p5()
        .gap_2()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(accent_blue_border())
        .bg(accent_blue_soft())
        .flex_row()
        .items_center()
        .justify_between()
        .child(
            text(tr(
                "监听端口或范围已修改，重启抓包后生效",
                "Listener settings changed — restart capture to apply",
            ))
            .flex_1()
            .text_size(px(TEXT_XS))
            .text_color(status_warn_fg()),
        )
        .child(
            ui_button(ButtonVariant::Secondary, ControlSize::Xs)
                .accessibility_label(move || {
                    Some(t(ui_lang(langs), "重启抓包", "Restart Capture").into())
                })
                .on_press(move || restart_capture(controls.clone()))
                .child(
                    text(move || -> SharedString {
                        t(ui_lang(langs), "重启抓包", "Restart Capture").into()
                    })
                    .number_of_lines(1)
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg_primary()),
                ),
        )
}

fn capture_listener_card(props: &SettingsWindowProps) -> impl IntoElement {
    let controls = props.controls.clone();
    let restart_controls = controls.clone();
    let port_text = props.port_text;
    let listen_all = props.listen_all;
    let running = props.running;
    let listen = props.listen;
    let langs = ui_langs();

    let listener_row = view()
        .w_full()
        .px_1()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_3()
        .child(port_field(port_text, controls));
    let listener_row = desktop_child!(listener_row, lan_toggle(listen_all, props.controls.clone()));

    let port_invalid = move || !present::is_valid_port_input(&port_text.get());
    let needs_restart = move || {
        running.get()
            && !port_invalid()
            && listen
                .with(|active| listener_needs_restart(active, &port_text.get(), listen_all.get()))
    };

    settings_card(
        "监听地址",
        "Capture Listener",
        view()
            .w_full()
            .gap_1p5()
            .flex_col()
            .child(listener_row)
            .child(when(port_invalid, || {
                text(tr(
                    "端口无效：请输入 1–65535 之间的数字（当前沿用旧端口）",
                    "Invalid port: enter 1–65535 (previous port remains active)",
                ))
                .px_1()
                .text_size(px(TEXT_XS))
                .text_color(status_err_fg())
            }))
            .child(when(needs_restart, move || {
                restart_capture_banner(restart_controls.clone(), langs)
            })),
    )
}

fn upstream_proxy_card(upstream_text: Signal<String>, controls: Arc<Controls>) -> impl IntoElement {
    let invalid = move || !present::is_valid_upstream_proxy(&upstream_text.get());
    settings_card(
        "上游代理",
        "Upstream Proxy",
        view()
            .w_full()
            .gap_1p5()
            .flex_col()
            .child(
                input()
                    .value(upstream_text)
                    .placeholder(tr(
                        "http://127.0.0.1:7890 或 socks5h://127.0.0.1:1080",
                        "http://127.0.0.1:7890 or socks5h://127.0.0.1:1080",
                    ))
                    .placeholder_color(move || color(fg_muted()))
                    .w_full()
                    .h(px(30.0))
                    .px_2p5()
                    .rounded(px(RADIUS_MD))
                    .bg(bg_code())
                    .border_1()
                    .border_color(move || {
                        if invalid() {
                            status_err_fg()
                        } else {
                            border_control()
                        }
                    })
                    .focus(|style| {
                        style.border_color(move || {
                            if invalid() {
                                status_err_fg()
                            } else {
                                accent_blue()
                            }
                        })
                    })
                    .text_size(px(TEXT_SM))
                    .text_color(fg_primary())
                    .on_change(move |text: &str| {
                        upstream_text.set(text.to_string());
                        if present::is_valid_upstream_proxy(text) {
                            controls.hub.set_upstream(text);
                            save_config_or_notify(&controls, PrefName::Upstream, |cfg| {
                                cfg.upstream_proxy = text.to_string();
                            });
                        }
                    }),
            )
            .child(when(invalid, || {
                text(tr(
                    "上游代理格式无效，请使用 http://、socks5:// 或 socks5h://host:port（当前沿用旧配置）",
                    "Invalid upstream proxy URL: use http://, socks5://, or socks5h://host:port (previous setting remains active)",
                ))
                .px_1()
                .text_size(px(TEXT_XS))
                .text_color(status_err_fg())
            })),
    )
}

pub(crate) fn settings_network_tab(props: SettingsWindowProps) -> impl IntoElement {
    let controls = props.controls.clone();
    let profiles = props.profiles;
    let bypass_text = profiles.bypass;
    let show_tunnels = profiles.show_tunnels;
    let bypass_controls = controls.clone();
    let tunnel_controls = controls.clone();

    view()
        .w_full()
        .gap_3()
        .flex_col()
        .child(capture_listener_card(&props))
        .child(upstream_proxy_card(profiles.upstream, controls))
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
    let denied_controls = controls.clone();
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
                ))
                .child(allow_denied_summary_section(
                    profiles.allow_denied,
                    profiles.allow_hosts,
                    profiles.allow_hosts_locked,
                    denied_controls,
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
                                #[cfg(not(target_arch = "wasm32"))]
                                if !close_on_preview {
                                    nsh_core::element::desktop::main_window().focus();
                                }
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
