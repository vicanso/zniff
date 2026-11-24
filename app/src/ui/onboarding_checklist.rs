//! Live 4-step onboarding setup checklist rendered at the top of the empty-state card.

#[allow(unused_imports)]
use super::*;

/// Session-level flow milestones used to auto-check onboarding steps 3 and 4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct OnboardingTraffic {
    /// Total flows recorded in the current session.
    pub(crate) total_flows: usize,
    /// True once at least one decrypted HTTPS or HTTP/2 flow without error is recorded.
    pub(crate) has_decrypted_https: bool,
    /// True when at least one flow failed with a TLS / certificate / tunnel error.
    pub(crate) has_tls_error: bool,
}

impl OnboardingTraffic {
    pub(crate) fn from_flows<'a>(flows: impl IntoIterator<Item = &'a session::Flow>) -> Self {
        let mut out = Self::default();
        for flow in flows {
            if flow.is_marker() {
                continue;
            }
            out.total_flows += 1;
            if is_decrypted_https_flow(flow) {
                out.has_decrypted_https = true;
            } else if is_tls_failure_flow(flow) {
                out.has_tls_error = true;
            }
        }
        out
    }
}

fn is_decrypted_https_flow(flow: &session::Flow) -> bool {
    if flow.is_marker() || flow.error.is_some() || flow.protocol == session::Protocol::Tunnel {
        return false;
    }
    flow.protocol == session::Protocol::Http2
        || flow.url.starts_with("https://")
        || flow.url.starts_with("wss://")
}

fn is_tls_failure_flow(flow: &session::Flow) -> bool {
    let Some(err) = flow.error.as_deref() else {
        return false;
    };
    flow.protocol == session::Protocol::Tunnel
        || classify_error(flow.status, err) == Some(ErrorCategory::TlsTrust)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChecklistStepId {
    ProxyListening,
    RootCaTrusted,
    FirstHttp,
    FirstHttps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepStatus {
    Done,
    Info,
    ActionRequired,
    Waiting,
}

/// Number of checklist steps whose readiness can be objectively verified on the host.
pub(crate) const VERIFIABLE_CHECKLIST_TOTAL: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChecklistInput {
    pub(crate) running: bool,
    pub(crate) listen_addr: String,
    pub(crate) ca_trusted: bool,
    pub(crate) ca_installed_untrusted: bool,
    pub(crate) ca_optional_web: bool,
    pub(crate) active_external_ca: Option<String>,
    pub(crate) detected_external_ca: Option<String>,
    pub(crate) traffic: OnboardingTraffic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChecklistStep {
    pub(crate) id: ChecklistStepId,
    pub(crate) status: StepStatus,
    pub(crate) title: String,
    pub(crate) hint: String,
}

/// Pure computation of the 4-step onboarding checklist state and bilingual hints.
pub(crate) fn compute_onboarding_checklist(
    input: &ChecklistInput,
    lang: Lang,
) -> [ChecklistStep; 4] {
    [
        step_proxy_listening(input, lang),
        step_root_ca_trusted(input, lang),
        step_first_http(input, lang),
        step_first_https(input, lang),
    ]
}

/// Counts verifiable steps that are completed (`ProxyListening`, `FirstHttp`, `FirstHttps`).
///
/// `RootCaTrusted` is excluded because desktop cannot verify whether mobile/remote clients
/// have trusted the CA until real HTTPS traffic arrives.
pub(crate) fn completed_checklist_count(steps: &[ChecklistStep; 4]) -> usize {
    steps
        .iter()
        .filter(|s| s.id != ChecklistStepId::RootCaTrusted && s.status == StepStatus::Done)
        .count()
}

fn step_proxy_listening(input: &ChecklistInput, lang: Lang) -> ChecklistStep {
    let (status, hint) = if input.running {
        let hint = if input.listen_addr.is_empty() {
            t(lang, "代理监听已启动", "Proxy is listening").to_string()
        } else {
            match lang {
                Lang::Zh => format!("正在监听 {}", input.listen_addr),
                Lang::En => format!("Listening on {}", input.listen_addr),
            }
        };
        (StepStatus::Done, hint)
    } else {
        (
            StepStatus::ActionRequired,
            t(
                lang,
                "代理未启动，点击右侧「开始」监听本地端口",
                "Proxy stopped — click Start to begin listening",
            )
            .to_string(),
        )
    };
    ChecklistStep {
        id: ChecklistStepId::ProxyListening,
        status,
        title: t(lang, "代理已监听", "Proxy listening").to_string(),
        hint,
    }
}

fn step_root_ca_trusted(input: &ChecklistInput, lang: Lang) -> ChecklistStep {
    let (status, hint) = if input.ca_optional_web {
        (
            StepStatus::Info,
            t(
                lang,
                "服务端模式 · 请在客户端设备信任服务端 CA，是否生效将在收到首条 HTTPS 时验证",
                "Server mode · Trust the server CA on client devices; verified on the first HTTPS flow",
            )
            .to_string(),
        )
    } else if input.ca_trusted {
        let hint = if let Some(tool) = input.active_external_ca.as_deref() {
            match lang {
                Lang::Zh => format!(
                    "本机已复用 {tool} 证书 · 手机等远程设备需扫码或访问 /zniff，无法由本机检测是否已信任"
                ),
                Lang::En => format!(
                    "Host reusing {tool} CA · Mobile/remote clients need /zniff and cannot be detected from host"
                ),
            }
        } else {
            t(
                lang,
                "本机系统库已信任 · 手机等远程设备需扫码或访问 /zniff，无法由本机检测是否已信任",
                "Host store trusted · Mobile/remote clients need /zniff and cannot be detected from host",
            )
            .to_string()
        };
        (StepStatus::Info, hint)
    } else if input.ca_installed_untrusted {
        (
            StepStatus::ActionRequired,
            t(
                lang,
                "本机证书已安装但尚未信任；点击信任，或改用隔离浏览器 / 手机扫码（远程设备状态无法提前检测）",
                "Host CA installed but untrusted; trust it, or use Isolated Browser / Mobile QR (remote trust cannot be pre-checked)",
            )
            .to_string(),
        )
    } else if let Some(tool) = input.detected_external_ca.as_deref() {
        let hint = match lang {
            Lang::Zh => format!(
                "本机未安装（可复用 {tool} 证书）· 手机抓包请扫码访问 /zniff；客户端信任状态将在首条 HTTPS 时验证"
            ),
            Lang::En => format!(
                "Host CA not installed ({tool} CA detected) · Mobile: scan QR /zniff; client trust is verified on first HTTPS"
            ),
        };
        (StepStatus::Info, hint)
    } else {
        (
            StepStatus::Info,
            t(
                lang,
                "本机未安装（仅系统代理需要）· 手机抓包请扫码访问 /zniff；客户端信任状态将在首条 HTTPS 时验证",
                "Host CA not installed (host proxy only) · Mobile: scan QR /zniff; client trust is verified on first HTTPS",
            )
            .to_string(),
        )
    };
    ChecklistStep {
        id: ChecklistStepId::RootCaTrusted,
        status,
        title: t(
            lang,
            "证书准备（本机 / 移动端）",
            "Certificate Setup (Host / Mobile)",
        )
        .to_string(),
        hint,
    }
}

fn step_first_http(input: &ChecklistInput, lang: Lang) -> ChecklistStep {
    let (status, hint) = if input.traffic.total_flows > 0 {
        let n = input.traffic.total_flows;
        let hint = match lang {
            Lang::Zh => format!("已捕获 {n} 条流量"),
            Lang::En => format!("Captured {n} flow(s)"),
        };
        (StepStatus::Done, hint)
    } else if !input.running {
        (
            StepStatus::Waiting,
            t(
                lang,
                "请先完成第 1 步启动代理监听",
                "Start the proxy in step 1 first",
            )
            .to_string(),
        )
    } else {
        (
            StepStatus::Waiting,
            t(
                lang,
                "等待流量：打开下方隔离浏览器 / 代理终端，或将客户端代理指向监听地址",
                "Waiting for traffic: open Isolated Browser / Proxied Terminal, or point a client here",
            )
            .to_string(),
        )
    };
    ChecklistStep {
        id: ChecklistStepId::FirstHttp,
        status,
        title: t(lang, "收到第一条 HTTP", "First HTTP received").to_string(),
        hint,
    }
}

fn step_first_https(input: &ChecklistInput, lang: Lang) -> ChecklistStep {
    let (status, hint) = if input.traffic.has_decrypted_https {
        (
            StepStatus::Done,
            t(lang, "HTTPS 流量已成功解密", "HTTPS traffic decrypted").to_string(),
        )
    } else if input.traffic.has_tls_error {
        (
            StepStatus::ActionRequired,
            t(
                lang,
                "检测到 TLS 握手失败：请检查证书信任，或该应用启用了证书钉扎（可加入直通）",
                "TLS handshake error: verify CA trust, or bypass TLS if the app uses pinning",
            )
            .to_string(),
        )
    } else if input.traffic.total_flows > 0 {
        (
            StepStatus::Waiting,
            t(
                lang,
                "仅收到明文 HTTP；访问任意 HTTPS 地址验证解密（失败请检查 CA 或证书钉扎）",
                "Only plain HTTP arrived; visit an HTTPS URL to verify (check CA/pinning if it fails)",
            )
            .to_string(),
        )
    } else if !input.ca_optional_web && !input.ca_trusted {
        (
            StepStatus::Waiting,
            t(
                lang,
                "信任根证书或使用隔离浏览器访问任意 HTTPS 站点即可完成",
                "Trust the CA or open Isolated Browser and visit any HTTPS site",
            )
            .to_string(),
        )
    } else {
        (
            StepStatus::Waiting,
            t(
                lang,
                "访问任意 HTTPS 站点即可验证解密",
                "Visit any HTTPS site to verify decryption",
            )
            .to_string(),
        )
    };
    ChecklistStep {
        id: ChecklistStepId::FirstHttps,
        status,
        title: t(lang, "收到第一条解密的 HTTPS", "First decrypted HTTPS").to_string(),
        hint,
    }
}

fn current_checklist_input(props: &OnboardingProps) -> ChecklistInput {
    #[cfg(target_arch = "wasm32")]
    let (ca_trusted, ca_installed_untrusted, ca_optional_web, active_ext, detected_ext) =
        (false, false, true, None, None);

    #[cfg(not(target_arch = "wasm32"))]
    let (ca_trusted, ca_installed_untrusted, ca_optional_web, active_ext, detected_ext) = {
        let status = props.ca_ui.status.get();
        let active = props
            .ca_ui
            .active_external
            .with(|ext| ext.as_ref().map(|item| item.tool_name.clone()));
        let detected = props
            .ca_ui
            .detected_external
            .with(|items| items.first().map(|item| item.tool_name.clone()));
        (
            status == ca::CaStatus::Trusted,
            status == ca::CaStatus::InstalledUntrusted,
            false,
            active,
            detected,
        )
    };

    ChecklistInput {
        running: props.running.get(),
        listen_addr: props.listen.get(),
        ca_trusted,
        ca_installed_untrusted,
        ca_optional_web,
        active_external_ca: active_ext,
        detected_external_ca: detected_ext,
        traffic: props.onboarding_traffic.get(),
    }
}

/// Reactive 4-step setup checklist card rendered at the top of the onboarding view.
pub(crate) fn onboarding_checklist(props: &OnboardingProps) -> impl IntoElement {
    let props_hdr = props.clone();
    let langs = ui_langs();

    let header = view()
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(section_title(tr("准备清单", "Setup Checklist")))
        .child(
            mono(move || -> SharedString {
                let lang = ui_lang(langs);
                let steps =
                    compute_onboarding_checklist(&current_checklist_input(&props_hdr), lang);
                let done = completed_checklist_count(&steps);
                match lang {
                    Lang::Zh => format!("{done}/{VERIFIABLE_CHECKLIST_TOTAL} 已完成").into(),
                    Lang::En => format!("{done}/{VERIFIABLE_CHECKLIST_TOTAL} ready").into(),
                }
            })
            .text_size(px(TEXT_XS))
            .font_weight(FontWeight::MEDIUM)
            .text_color(fg_muted()),
        );

    let card = view()
        .w_full()
        .flex_col()
        .gap_2()
        .px_3p5()
        .py_3()
        .rounded(px(RADIUS_LG))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .child(checklist_step_row(
            props,
            0,
            ChecklistStepId::ProxyListening,
        ))
        .child(checklist_divider())
        .child(checklist_step_row(props, 1, ChecklistStepId::RootCaTrusted))
        .child(checklist_divider())
        .child(checklist_step_row(props, 2, ChecklistStepId::FirstHttp))
        .child(checklist_divider())
        .child(checklist_step_row(props, 3, ChecklistStepId::FirstHttps));

    view().w_full().flex_col().gap_2().child(header).child(card)
}

fn checklist_divider() -> impl IntoElement {
    view().w_full().h(px(1.0)).bg(border_subtle())
}

fn checklist_step_row(
    props: &OnboardingProps,
    index: usize,
    step_id: ChecklistStepId,
) -> impl IntoElement {
    let props_status = props.clone();
    let props_title = props.clone();
    let props_hint = props.clone();
    let props_hint_color = props.clone();
    let langs = ui_langs();

    let row = view()
        .w_full()
        .flex_row()
        .items_start()
        .gap_2p5()
        .child(checklist_step_badge(props_status, index, step_id))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_col()
                .gap_0p5()
                .child(
                    text(move || -> SharedString {
                        let lang = ui_lang(langs);
                        let steps = compute_onboarding_checklist(
                            &current_checklist_input(&props_title),
                            lang,
                        );
                        steps[index].title.clone().into()
                    })
                    .text_size(px(TEXT_MD))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg_primary()),
                )
                .child(
                    text(move || -> SharedString {
                        let lang = ui_lang(langs);
                        let steps = compute_onboarding_checklist(
                            &current_checklist_input(&props_hint),
                            lang,
                        );
                        steps[index].hint.clone().into()
                    })
                    .text_size(px(TEXT_XS))
                    .text_color(move || {
                        let lang = ui_lang(langs);
                        let steps = compute_onboarding_checklist(
                            &current_checklist_input(&props_hint_color),
                            lang,
                        );
                        if step_id == ChecklistStepId::FirstHttps
                            && steps[index].status == StepStatus::ActionRequired
                        {
                            status_warn_fg()
                        } else {
                            fg_muted()
                        }
                    }),
                ),
        );

    match step_id {
        ChecklistStepId::ProxyListening => row.child(step_proxy_actions(props)),
        ChecklistStepId::RootCaTrusted => row.child(step_ca_actions(props)),
        ChecklistStepId::FirstHttp | ChecklistStepId::FirstHttps => row,
    }
}

fn checklist_step_status(props: &OnboardingProps, index: usize, lang: Lang) -> StepStatus {
    compute_onboarding_checklist(&current_checklist_input(props), lang)[index].status
}

fn checklist_step_number(step_id: ChecklistStepId) -> &'static str {
    match step_id {
        ChecklistStepId::ProxyListening => "1",
        ChecklistStepId::RootCaTrusted => "",
        ChecklistStepId::FirstHttp => "2",
        ChecklistStepId::FirstHttps => "3",
    }
}

fn checklist_step_badge(
    props: OnboardingProps,
    index: usize,
    step_id: ChecklistStepId,
) -> impl IntoElement {
    let p_bg = props.clone();
    let p_done = props.clone();
    let p_info = props.clone();
    let p_warn = props;
    let langs = ui_langs();
    let num_label = checklist_step_number(step_id);

    let badge = view()
        .w(px(18.0))
        .h(px(18.0))
        .mt(px(1.0))
        .rounded_full()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .bg(
            move || match checklist_step_status(&p_bg, index, ui_lang(langs)) {
                StepStatus::Done => status_ok_bg(),
                StepStatus::Info => accent_blue_soft(),
                StepStatus::ActionRequired => status_warn_bg(),
                StepStatus::Waiting => bg_control(),
            },
        )
        .child(
            when(
                move || checklist_step_status(&p_done, index, ui_lang(langs)) == StepStatus::Done,
                move || glyph_px(11.0, || Glyph::Check, || color(status_ok_fg())),
            )
            .otherwise(move || {
                let p_i = p_info.clone();
                let p_w = p_warn.clone();
                when(
                    move || checklist_step_status(&p_i, index, ui_lang(langs)) == StepStatus::Info,
                    move || glyph_px(11.0, || Glyph::Info, || color(accent_blue_text())),
                )
                .otherwise(move || {
                    let p_w2 = p_w.clone();
                    when(
                        move || {
                            checklist_step_status(&p_w2, index, ui_lang(langs))
                                == StepStatus::ActionRequired
                        },
                        move || glyph_px(11.0, || Glyph::Alert, || color(status_warn_fg())),
                    )
                    .otherwise(move || {
                        mono(num_label)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg_muted())
                    })
                })
            }),
        );

    if step_id == ChecklistStepId::RootCaTrusted {
        badge.tooltip(label_tip(
            "为何不打勾：本机无法读取手机或远程设备的证书信任状态，实际是否就绪会在收到第一条解密的 HTTPS 流量时自动确认",
            "Why no checkmark: desktop cannot inspect mobile or remote trust stores; readiness is confirmed automatically once the first decrypted HTTPS flow arrives",
        ))
    } else {
        badge
    }
}

fn step_proxy_actions(props: &OnboardingProps) -> impl IntoElement {
    let running = props.running;
    let controls = props.controls.clone();
    when(
        move || !running.get(),
        move || {
            let c = controls.clone();
            checklist_action_chip(tr("开始", "Start"), true, move || {
                toggle_capture(c.clone())
            })
        },
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn step_ca_actions(props: &OnboardingProps) -> impl IntoElement {
    let ca_ui = props.ca_ui;
    let controls = props.controls.clone();
    let running = props.running;
    let pending_qr = props.pending_qr;
    let qr_open = props.qr_open;
    let install_ca = props.install_ca.clone();
    let langs = ui_langs();
    let qr_controls = controls.clone();
    let untrusted = move || ca_ui.status.get() != ca::CaStatus::Trusted;

    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_end()
        .gap_1p5()
        .flex_shrink_0()
        .child(when(
            move || untrusted() && ca_ui.detected_external.with(|items| !items.is_empty()),
            move || {
                let c = controls.clone();
                checklist_action_chip(
                    move || -> SharedString {
                        let lang = ui_lang(langs);
                        let tool = ca_ui
                            .detected_external
                            .with(|items| items.first().map(|i| i.tool_name.clone()))
                            .unwrap_or_default();
                        match lang {
                            Lang::Zh => format!("复用 {tool}").into(),
                            Lang::En => format!("Reuse {tool}").into(),
                        }
                    },
                    true,
                    move || {
                        if let Some(first) =
                            ca_ui.detected_external.with(|items| items.first().cloned())
                        {
                            use_external_ca(c.clone(), first.id);
                        }
                    },
                )
            },
        ))
        .child(when(untrusted, move || {
            let install_cb = install_ca.clone();
            checklist_action_chip(
                move || -> SharedString {
                    let lang = ui_lang(langs);
                    match ca_ui.status.get() {
                        ca::CaStatus::InstalledUntrusted => {
                            t(lang, "信任本机证书", "Trust Host CA").into()
                        }
                        ca::CaStatus::NotInstalled | ca::CaStatus::Trusted => {
                            t(lang, "安装本机证书", "Install Host CA").into()
                        }
                    }
                },
                false,
                move || install_cb.as_ref()(),
            )
        }))
        .child(checklist_action_chip(
            tr("手机扫码", "Mobile QR"),
            false,
            move || {
                trigger_mobile_qr(&qr_controls, running, pending_qr, qr_open);
            },
        ))
}

#[cfg(target_arch = "wasm32")]
fn step_ca_actions(_props: &OnboardingProps) -> impl IntoElement {
    view()
}

fn checklist_action_chip(
    label: impl IntoReactive<SharedString>,
    accent: bool,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    nsh_base::button()
        .on_press(on_press)
        .h(px(ControlSize::Xs.height()))
        .px_2()
        .flex_row()
        .items_center()
        .rounded(px(RADIUS_SM))
        .border_1()
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .border_color(if accent {
            accent_blue_border()
        } else {
            border_control()
        })
        .bg(if accent {
            accent_blue_soft()
        } else {
            bg_control()
        })
        .hover(move |s| {
            s.bg(if accent {
                accent_blue_soft_hover()
            } else {
                bg_control_hover()
            })
        })
        .active(|s| s.bg(bg_control_active()))
        .focus_visible(apply_focus_ring)
        .child(
            text(label)
                .number_of_lines(1)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if accent {
                    accent_blue_text()
                } else {
                    fg_secondary()
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{FlowDraft, Protocol, Session};

    fn base_input() -> ChecklistInput {
        ChecklistInput {
            running: false,
            listen_addr: String::new(),
            ca_trusted: false,
            ca_installed_untrusted: false,
            ca_optional_web: false,
            active_external_ca: None,
            detected_external_ca: None,
            traffic: OnboardingTraffic::default(),
        }
    }

    #[test]
    fn checklist_initial_stopped_and_untrusted() {
        let steps = compute_onboarding_checklist(&base_input(), Lang::En);
        assert_eq!(steps[0].status, StepStatus::ActionRequired);
        assert_eq!(steps[1].status, StepStatus::Info);
        assert_eq!(steps[2].status, StepStatus::Waiting);
        assert_eq!(steps[3].status, StepStatus::Waiting);
        assert_eq!(completed_checklist_count(&steps), 0);
        assert!(steps[0].hint.contains("Start"));
        assert!(steps[1].title.contains("Host / Mobile"));
        assert!(steps[1].hint.contains("/zniff"));
        assert!(steps[1].hint.contains("verified on first HTTPS"));
    }

    #[test]
    fn checklist_proxy_listening_and_ca_trusted_transitions() {
        let mut input = base_input();
        input.running = true;
        input.listen_addr = "127.0.0.1:8877".into();
        let steps = compute_onboarding_checklist(&input, Lang::Zh);
        assert_eq!(steps[0].status, StepStatus::Done);
        assert!(steps[0].hint.contains("127.0.0.1:8877"));
        assert_eq!(steps[1].status, StepStatus::Info);
        assert_eq!(completed_checklist_count(&steps), 1);
        assert!(steps[1].hint.contains("/zniff"));

        // Even when host CA is trusted, Step 2 stays Info and does not count as Done.
        input.ca_trusted = true;
        let steps = compute_onboarding_checklist(&input, Lang::En);
        assert_eq!(steps[1].status, StepStatus::Info);
        assert_eq!(completed_checklist_count(&steps), 1);
        assert!(steps[1].hint.contains("/zniff"));
        assert!(steps[1].hint.contains("cannot be detected from host"));

        input.active_external_ca = Some("Charles".into());
        let steps = compute_onboarding_checklist(&input, Lang::Zh);
        assert_eq!(steps[1].status, StepStatus::Info);
        assert!(steps[1].hint.contains("Charles"));
        assert!(steps[1].hint.contains("/zniff"));
        assert!(steps[1].hint.contains("无法由本机检测"));
    }

    #[test]
    fn checklist_ca_installed_untrusted_and_detected_external() {
        let mut input = base_input();
        input.ca_installed_untrusted = true;
        let steps = compute_onboarding_checklist(&input, Lang::En);
        assert_eq!(steps[1].status, StepStatus::ActionRequired);
        assert!(steps[1].hint.contains("untrusted"));
        assert!(steps[1].hint.contains("cannot be pre-checked"));

        input.ca_installed_untrusted = false;
        input.detected_external_ca = Some("Whistle".into());
        let steps = compute_onboarding_checklist(&input, Lang::Zh);
        assert_eq!(steps[1].status, StepStatus::Info);
        assert!(steps[1].hint.contains("Whistle"));
        assert!(steps[1].hint.contains("/zniff"));
    }

    #[test]
    fn checklist_wasm_marks_ca_info() {
        let mut input = base_input();
        input.running = true;
        input.ca_optional_web = true;
        let steps = compute_onboarding_checklist(&input, Lang::En);
        assert_eq!(steps[1].status, StepStatus::Info);
        assert_eq!(completed_checklist_count(&steps), 1);
    }

    #[test]
    fn checklist_http_only_and_tls_error_and_https_success() {
        let mut input = base_input();
        input.running = true;
        input.ca_trusted = true;

        // Plain HTTP only -> 2/3 verifiable steps ready
        input.traffic = OnboardingTraffic {
            total_flows: 2,
            has_decrypted_https: false,
            has_tls_error: false,
        };
        let steps = compute_onboarding_checklist(&input, Lang::En);
        assert_eq!(steps[2].status, StepStatus::Done);
        assert_eq!(steps[3].status, StepStatus::Waiting);
        assert!(steps[3].hint.contains("plain HTTP"));
        assert_eq!(completed_checklist_count(&steps), 2);

        // TLS error without decrypted HTTPS
        input.traffic.has_tls_error = true;
        let steps = compute_onboarding_checklist(&input, Lang::Zh);
        assert_eq!(steps[2].status, StepStatus::Done);
        assert_eq!(steps[3].status, StepStatus::ActionRequired);
        assert!(steps[3].hint.contains("TLS"));

        // Decrypted HTTPS succeeds -> all 3 verifiable steps done
        input.traffic.has_decrypted_https = true;
        let steps = compute_onboarding_checklist(&input, Lang::En);
        assert_eq!(steps[3].status, StepStatus::Done);
        assert_eq!(
            completed_checklist_count(&steps),
            VERIFIABLE_CHECKLIST_TOTAL
        );
    }

    #[test]
    fn onboarding_traffic_classifies_session_flows() {
        let mut session = Session::default();
        // Plain HTTP flow
        session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "example.com".into(),
            path: "/".into(),
            url: "http://example.com/".into(),
            request_headers: vec![],
        });
        let t1 = OnboardingTraffic::from_flows(session.iter());
        assert_eq!(t1.total_flows, 1);
        assert!(!t1.has_decrypted_https);
        assert!(!t1.has_tls_error);

        // Failed TLS tunnel
        let err_id = session.open(FlowDraft {
            protocol: Protocol::Tunnel,
            method: "CONNECT".into(),
            host: "pinned.example.com".into(),
            path: String::new(),
            url: "https://pinned.example.com:443".into(),
            request_headers: vec![],
        });
        session.set_error(err_id, "client TLS handshake failed: unknown ca");
        let t2 = OnboardingTraffic::from_flows(session.iter());
        assert_eq!(t2.total_flows, 2);
        assert!(!t2.has_decrypted_https);
        assert!(t2.has_tls_error);

        // Decrypted HTTPS/HTTP2 flow
        session.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "GET".into(),
            host: "api.example.com".into(),
            path: "/v1".into(),
            url: "https://api.example.com/v1".into(),
            request_headers: vec![],
        });
        let t3 = OnboardingTraffic::from_flows(session.iter());
        assert_eq!(t3.total_flows, 3);
        assert!(t3.has_decrypted_https);
        assert!(t3.has_tls_error);
    }
}
