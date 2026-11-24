#[allow(unused_imports)]
use super::*;

#[derive(Clone)]
pub(crate) struct AccessControlProps {
    pub(crate) open: Signal<bool>,
    pub(crate) proxy_auth: Signal<String>,
    pub(crate) allow_hosts: Signal<String>,
    pub(crate) allow_hosts_strict: Signal<bool>,
    pub(crate) allow_hosts_locked: Signal<bool>,
    pub(crate) allow_denied: Signal<allow_hosts::AllowDeniedSummary>,
    pub(crate) controls: Arc<Controls>,
}

/// Evaluates whether Basic auth is active and how host restrictions affect proxied requests.
pub(crate) fn access_summary_state(
    proxy_auth: &str,
    allow_hosts: &str,
    strict: bool,
) -> (bool, bool, bool) {
    let auth_active = !proxy_auth.trim().is_empty();
    let rules = crate::allow_hosts::AllowHostRules::parse(allow_hosts, strict);
    (auth_active, rules.denies_all(), rules.is_restricted())
}

/// Combined footer popover for machine-level proxy authentication and allowed host rules.
pub(crate) fn access_control_popover(props: AccessControlProps) -> impl IntoElement {
    let open = props.open;
    let controls = props.controls.clone();
    let denied_controls = props.controls.clone();
    let allow_hosts = props.allow_hosts;
    let allow_hosts_strict = props.allow_hosts_strict;
    let allow_hosts_locked = props.allow_hosts_locked;
    let allow_denied = props.allow_denied;
    let proxy_auth = props.proxy_auth;
    popover()
        .open(move || Some(open.get()))
        .popup_style(ui_popup_style)
        .popup_starting_style(ui_popup_starting_style)
        .placement(Placement::Top)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(8.0))
        .on_open_change(move |event: &nsh_base::PopoverEvent| {
            if let nsh_base::PopoverEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .trigger(access_trigger_pill(open, &props))
        .child(
            view()
                .w(px(340.0))
                .p_3()
                .gap_2p5()
                .child(proxy_auth_section(proxy_auth, controls.clone()))
                .child(ui_divider())
                .child(allow_hosts_section(
                    allow_hosts,
                    allow_hosts_strict,
                    allow_hosts_locked,
                    Rc::new(move |raw: &str| {
                        apply_allow_hosts_change(&controls, allow_hosts, raw);
                    }),
                ))
                .child(allow_denied_summary_section(
                    allow_denied,
                    allow_hosts,
                    allow_hosts_locked,
                    denied_controls,
                )),
        )
}

/// Footer trigger pill showing live authentication, host-restriction, and strict deny-all state.
pub(crate) fn access_trigger_pill(
    open: Signal<bool>,
    props: &AccessControlProps,
) -> impl IntoElement {
    let langs = ui_langs();
    let proxy_auth = props.proxy_auth;
    let allow_hosts = props.allow_hosts;
    let allow_hosts_strict = props.allow_hosts_strict;
    let allow_denied = props.allow_denied;
    let state = move || {
        access_summary_state(
            &proxy_auth.get(),
            &allow_hosts.get(),
            allow_hosts_strict.get(),
        )
    };
    let denied_total = move || allow_denied.with(|summary| summary.total);
    view()
        .role(AccessibilityRole::Button)
        .h(px(ControlSize::Xs.height()))
        .px(px(ControlSize::Xs.padding_x()))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap(px(ControlSize::Xs.gap()))
        .rounded(px(RADIUS_SM))
        .border_1()
        .border_color(move || {
            let (auth_on, denies_all, restricted) = state();
            if denies_all || denied_total() > 0 {
                status_warn_fg()
            } else if open.get() || auth_on || restricted {
                accent_blue_border()
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .bg(move || {
            let (auth_on, denies_all, restricted) = state();
            if denies_all || denied_total() > 0 {
                status_warn_bg()
            } else if open.get() || auth_on || restricted {
                accent_blue_soft()
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .hover(|style| {
            style.bg(move || {
                let (auth_on, denies_all, restricted) = state();
                if denies_all || denied_total() > 0 {
                    status_warn_bg()
                } else if open.get() || auth_on || restricted {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(move || {
            let (auth_on, denies_all, restricted) = state();
            if denies_all || denied_total() > 0 {
                status_warn_fg()
            } else if auth_on || restricted {
                status_ok_fg()
            } else {
                fg_muted()
            }
        }))
        .child(
            text(move || -> SharedString {
                let lang = ui_lang(langs);
                let (_, denies_all, _) = state();
                let blocked = denied_total();
                if blocked > 0 {
                    let blocked = present::format_thousands(blocked);
                    match lang {
                        Lang::Zh => format!("安全 · 拦截 {blocked}").into(),
                        Lang::En => format!("Access · {blocked} denied").into(),
                    }
                } else if denies_all {
                    t(lang, "安全 · 全拒", "Access · Denied").into()
                } else {
                    t(lang, "安全", "Access").into()
                }
            })
            .tabular_nums()
            .number_of_lines(1)
            .text_size(px(TEXT_XS))
            .text_color(move || {
                let (auth_on, denies_all, restricted) = state();
                if denies_all || denied_total() > 0 {
                    status_warn_fg()
                } else if open.get() || auth_on || restricted {
                    accent_blue_text()
                } else {
                    fg_secondary()
                }
            }),
        )
}

pub(crate) fn proxy_auth_section(
    proxy_auth: Signal<String>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let langs = ui_langs();
    let active = move || !proxy_auth.get().trim().is_empty();
    let change_controls = controls.clone();
    let clear_controls = controls;
    view()
        .w_full()
        .gap_1p5()
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
                    text(move || -> SharedString {
                        if active() {
                            t(ui_lang(langs), "已开启", "Enabled").into()
                        } else {
                            t(ui_lang(langs), "未开启", "Off").into()
                        }
                    })
                    .text_size(px(11.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || if active() { status_ok_fg() } else { fg_muted() }),
                ),
        )
        .child(
            text(tr(
                "格式为 user:pass，留空表示不校验（/zniff 安装页豁免）。",
                "Format: user:pass. Empty disables auth (/zniff is exempt).",
            ))
            .text_size(px(TEXT_XS))
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
                        .h(px(ControlSize::Sm.height()))
                        .px_2()
                        .rounded(px(RADIUS_MD))
                        .bg(bg_code())
                        .border_1()
                        .border_color(border_control())
                        .focus(|style| style.border_color(accent_blue()))
                        .text_size(px(TEXT_SM))
                        .text_color(fg_primary())
                        .on_change(move |text: &str| {
                            apply_proxy_auth_change(&change_controls, proxy_auth, text);
                        }),
                )
                .child(when(
                    move || !proxy_auth.get().is_empty(),
                    move || {
                        let controls = clear_controls.clone();
                        clear_pill_button(move || {
                            apply_proxy_auth_change(&controls, proxy_auth, "");
                        })
                    },
                )),
        )
}

/// Renders the host allow-list editor or read-only locked view inside the Access popover.
pub(crate) fn allow_hosts_section(
    allow_hosts: Signal<String>,
    strict: Signal<bool>,
    locked: Signal<bool>,
    on_change_hosts: Rc<dyn Fn(&str)>,
) -> impl IntoElement {
    let langs = ui_langs();
    let rules = move || crate::allow_hosts::AllowHostRules::parse(&allow_hosts.get(), strict.get());
    let edit_cb = on_change_hosts.clone();
    let clear_cb = on_change_hosts;
    view()
        .w_full()
        .gap_1p5()
        .child(allow_hosts_header_row(allow_hosts, strict, locked, langs))
        .child(when(
            move || rules().denies_all(),
            move || {
                view()
                    .w_full()
                    .px_2()
                    .py_1p5()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(status_warn_fg())
                    .bg(status_warn_bg())
                    .child(
                        text(tr(
                            "当前拒绝所有代理域名（返回 403）。请填入允许域名，或填写 * 放行全部。",
                            "Strictly denying all proxied domains (403). Add allowed hosts below, or use * for all.",
                        ))
                        .text_size(px(11.0))
                        .line_height(Points(16.0))
                        .text_color(status_warn_fg()),
                    )
            },
        ))
        .child(
            text(tr(
                "每行或逗号分隔：精确域名、*.子域、* 放行全部、-域名 排除，支持 # 注释。",
                "Line/comma separated: exact host, *.sub, * for all, -host to exclude, # comments.",
            ))
            .text_size(px(TEXT_XS))
            .line_height(Points(17.0))
            .text_color(fg_muted()),
        )
        .child(
            when(
                move || locked.get(),
                move || {
                    view()
                        .w_full()
                        .p_2()
                        .rounded(px(RADIUS_MD))
                        .border_1()
                        .border_color(border_control())
                        .bg(bg_code())
                        .child(
                            mono(move || -> SharedString {
                                let raw = allow_hosts.get();
                                if raw.trim().is_empty() {
                                    t(ui_lang(langs), "(未配置 · 全部拒绝)", "(empty · all denied)")
                                        .into()
                                } else {
                                    raw.into()
                                }
                            })
                            .text_size(px(TEXT_XS))
                            .text_color(fg_secondary()),
                        )
                },
            )
            .otherwise(move || {
                let edit_cb = edit_cb.clone();
                let clear_cb = clear_cb.clone();
                view()
                    .w_full()
                    .gap_1p5()
                    .child(
                        input()
                            .value(allow_hosts)
                            .multiline(true)
                            .placeholder(tr(
                                "api.vicanso.dev\n*.staging.vicanso.dev",
                                "api.vicanso.dev\n*.staging.vicanso.dev",
                            ))
                            .placeholder_color(move || color(fg_muted()))
                            .w_full()
                            .h(px(76.0))
                            .px_2()
                            .py_1p5()
                            .rounded(px(6.0))
                            .bg(bg_code())
                            .border_1()
                            .border_color(border_control())
                            .focus(|style| style.border_color(accent_blue()))
                            .text_size(px(12.0))
                            .text_color(fg_primary())
                            .on_change(move |text: &str| edit_cb(text)),
                    )
                    .child(when(
                        move || !allow_hosts.get().is_empty(),
                        move || {
                            let clear_cb = clear_cb.clone();
                            view()
                                .w_full()
                                .flex_row()
                                .justify_end()
                                .child(clear_pill_button(move || clear_cb("")))
                        },
                    ))
            }),
        )
}

fn allow_hosts_header_row(
    allow_hosts: Signal<String>,
    strict: Signal<bool>,
    locked: Signal<bool>,
    langs: Signal<Option<crate::i18n::Lang>>,
) -> impl IntoElement {
    let rules = move || crate::allow_hosts::AllowHostRules::parse(&allow_hosts.get(), strict.get());
    view()
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_1p5()
        .child(
            text(tr("允许代理的域名", "Allowed hosts"))
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_primary()),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(when(
                    move || locked.get(),
                    move || {
                        view()
                            .px_1p5()
                            .py_0p5()
                            .rounded(px(RADIUS_SM))
                            .border_1()
                            .border_color(border_control())
                            .bg(bg_control())
                            .child(
                                text(tr("环境变量锁定", "Locked by env/CLI"))
                                    .text_size(px(TEXT_XS))
                                    .text_color(fg_muted()),
                            )
                    },
                ))
                .child(
                    text(move || -> SharedString {
                        let lang = ui_lang(langs);
                        let parsed = rules();
                        if parsed.denies_all() {
                            t(lang, "已拒绝全部", "All denied").into()
                        } else if parsed.is_restricted() {
                            t(lang, "已限制", "Restricted").into()
                        } else {
                            t(lang, "不限制", "Unrestricted").into()
                        }
                    })
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || {
                        let parsed = rules();
                        if parsed.denies_all() {
                            status_warn_fg()
                        } else if parsed.is_restricted() {
                            status_ok_fg()
                        } else {
                            fg_muted()
                        }
                    }),
                ),
        )
}

fn clear_pill_button(on_press: impl Fn() + 'static) -> impl IntoElement {
    ui_button(ButtonVariant::Outline, ControlSize::Sm)
        .on_press(on_press)
        .child(
            text(tr("清空", "Clear"))
                .text_size(px(TEXT_SM))
                .text_color(fg_secondary()),
        )
}

fn apply_proxy_auth_change(controls: &Controls, proxy_auth: Signal<String>, raw: &str) {
    proxy_auth.set(raw.to_string());
    let stored = raw.trim().to_string();
    controls.hub.set_proxy_auth(&stored);
    #[cfg(not(target_arch = "wasm32"))]
    sync_transparent_proxy_config(controls);
    if let Err(err) = save_config(|config| config.proxy_auth = stored) {
        set_status(
            controls,
            StatusMsg::PrefUnsaved {
                name: PrefName::ProxyAuth,
                err: err.to_string(),
            },
        );
    }
}

pub(crate) fn apply_allow_hosts_change(
    controls: &Controls,
    allow_hosts: Signal<String>,
    raw: &str,
) {
    allow_hosts.set(raw.to_string());
    let stored = raw.trim().to_string();
    if let Err(err) = controls.hub.set_allow_hosts(&stored) {
        set_status(
            controls,
            StatusMsg::PrefUnsaved {
                name: PrefName::AllowHosts,
                err,
            },
        );
        return;
    }
    if let Err(err) = save_config(|config| config.allow_hosts = stored) {
        set_status(
            controls,
            StatusMsg::PrefUnsaved {
                name: PrefName::AllowHosts,
                err: err.to_string(),
            },
        );
    }
}
