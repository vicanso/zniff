#[allow(unused_imports)]
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OnboardingStripSummary {
    pub(crate) done: usize,
    pub(crate) total: usize,
    pub(crate) step: ChecklistStep,
}

pub(crate) fn next_incomplete_step(
    steps: &[ChecklistStep; 4],
    input: &ChecklistInput,
) -> Option<ChecklistStep> {
    if let Some(action) = steps
        .iter()
        .find(|step| step.status == StepStatus::ActionRequired)
    {
        return Some(action.clone());
    }
    let host_ca_ready =
        input.ca_trusted || input.ca_optional_web || input.active_external_ca.is_some();
    if !host_ca_ready && steps[1].status != StepStatus::Done {
        return Some(steps[1].clone());
    }
    steps
        .iter()
        .find(|step| {
            matches!(
                step.id,
                ChecklistStepId::ProxyListening
                    | ChecklistStepId::FirstHttp
                    | ChecklistStepId::FirstHttps
            ) && step.status != StepStatus::Done
        })
        .or_else(|| steps.iter().find(|step| step.status != StepStatus::Done))
        .cloned()
}

pub(crate) fn onboarding_strip_state(
    input: &ChecklistInput,
    dismissed: bool,
    flow_total: usize,
    phase: Phase,
    lang: Lang,
) -> Option<OnboardingStripSummary> {
    if dismissed || should_show_onboarding(phase, flow_total) {
        return None;
    }
    let steps = compute_onboarding_checklist(input, lang);
    let done = completed_checklist_count(&steps);
    if done >= VERIFIABLE_CHECKLIST_TOTAL {
        return None;
    }
    let step = next_incomplete_step(&steps, input)?;
    Some(OnboardingStripSummary {
        done,
        total: VERIFIABLE_CHECKLIST_TOTAL,
        step,
    })
}

fn copyable_checklist_input(
    running: Signal<bool>,
    listen: Signal<String>,
    ca_ui: CaSignals,
    traffic: Signal<OnboardingTraffic>,
) -> ChecklistInput {
    #[cfg(target_arch = "wasm32")]
    let _ = ca_ui;
    #[cfg(target_arch = "wasm32")]
    let (ca_trusted, ca_installed_untrusted, ca_optional_web, active_ext, detected_ext) =
        (false, false, true, None, None);

    #[cfg(not(target_arch = "wasm32"))]
    let (ca_trusted, ca_installed_untrusted, ca_optional_web, active_ext, detected_ext) = {
        let status = ca_ui.status.get();
        let active = ca_ui
            .active_external
            .with(|ext| ext.as_ref().map(|item| item.tool_name.clone()));
        let detected = ca_ui
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
        running: running.get(),
        listen_addr: listen.get(),
        ca_trusted,
        ca_installed_untrusted,
        ca_optional_web,
        active_external_ca: active_ext,
        detected_external_ca: detected_ext,
        traffic: traffic.get(),
    }
}

/// Persistent compact setup progress banner shown above the workspace while setup
/// steps remain incomplete after the full empty-state onboarding screen disappears.
pub(crate) fn onboarding_progress_strip(
    props: OnboardingProps,
    phase: Signal<Phase>,
) -> impl IntoElement {
    let running = props.running;
    let listen = props.listen;
    let ca_ui = props.ca_ui;
    let traffic = props.onboarding_traffic;
    let dismissed = props.profiles.onboarding_strip_dismissed;
    let flow_total = props.flow_total;
    let langs = ui_langs();
    when(
        move || {
            let input = copyable_checklist_input(running, listen, ca_ui, traffic);
            onboarding_strip_state(
                &input,
                dismissed.get(),
                flow_total.get(),
                phase.get(),
                ui_lang(langs),
            )
            .is_some()
        },
        move || strip_bar(props.clone(), phase, langs),
    )
}

fn strip_bar(
    props: OnboardingProps,
    phase: Signal<Phase>,
    langs: Signal<Option<Lang>>,
) -> impl IntoElement {
    let running = props.running;
    let listen = props.listen;
    let ca_ui = props.ca_ui;
    let traffic = props.onboarding_traffic;
    let dismissed = props.profiles.onboarding_strip_dismissed;
    let flow_total = props.flow_total;

    let read_summary = move || {
        let input = copyable_checklist_input(running, listen, ca_ui, traffic);
        onboarding_strip_state(
            &input,
            dismissed.get(),
            flow_total.get(),
            phase.get(),
            ui_lang(langs),
        )
    };

    view()
        .w_full()
        .min_h(px(34.0))
        .px_3()
        .py_1()
        .flex_shrink_0()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_2()
        .bg(bg_chrome())
        .border_b_1()
        .border_b_color(border_subtle())
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .gap_2()
                .child(strip_progress_badge(move || {
                    read_summary()
                        .map(|s| format!("{}/{}", s.done, s.total))
                        .unwrap_or_default()
                }))
                .child(
                    text(move || -> SharedString {
                        read_summary()
                            .map(|s| s.step.title)
                            .unwrap_or_default()
                            .into()
                    })
                    .number_of_lines(1)
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(fg_primary()),
                )
                .child(
                    text(move || -> SharedString {
                        read_summary()
                            .map(|s| s.step.hint)
                            .unwrap_or_default()
                            .into()
                    })
                    .flex_1()
                    .min_w(px(0.0))
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail)
                    .text_size(px(TEXT_XS))
                    .text_color(fg_muted()),
                ),
        )
        .child(
            view()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(strip_actions(&props, phase, langs))
                .child(strip_dismiss_button(dismissed, props.controls.clone())),
        )
}

fn strip_progress_badge(label: impl Fn() -> String + 'static) -> impl IntoElement {
    view()
        .h(px(20.0))
        .px_1p5()
        .flex_shrink_0()
        .rounded(px(RADIUS_SM))
        .border_1()
        .border_color(accent_blue_border())
        .bg(accent_blue_soft())
        .items_center()
        .justify_center()
        .child(
            mono(move || SharedString::from(label()))
                .tabular_nums()
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(accent_blue_text()),
        )
}

fn strip_dismiss_button(dismissed: Signal<bool>, controls: Arc<Controls>) -> impl IntoElement {
    view()
        .role(AccessibilityRole::Button)
        .w(px(22.0))
        .h(px(22.0))
        .flex_shrink_0()
        .rounded(px(RADIUS_SM))
        .hover(|s| s.bg(bg_control_hover()))
        .active(|s| s.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .items_center()
        .justify_center()
        .label(tr("关闭引导条", "Dismiss setup strip"))
        .tooltip(label_tip("不再显示顶部引导条", "Dismiss setup guide strip"))
        .on_press(move || {
            dismissed.set(true);
            save_config_or_notify(&controls, PrefName::Onboarding, |cfg| {
                cfg.onboarding_strip_dismissed = true;
            });
        })
        .child(glyph_px(11.0, || Glyph::Close, || color(fg_muted())))
}

#[cfg(not(target_arch = "wasm32"))]
fn strip_actions(
    props: &OnboardingProps,
    phase: Signal<Phase>,
    langs: Signal<Option<Lang>>,
) -> impl IntoElement {
    let running = props.running;
    let listen = props.listen;
    let ca_ui = props.ca_ui;
    let traffic = props.onboarding_traffic;
    let dismissed = props.profiles.onboarding_strip_dismissed;
    let flow_total = props.flow_total;
    let next_id = move || {
        let input = copyable_checklist_input(running, listen, ca_ui, traffic);
        onboarding_strip_state(
            &input,
            dismissed.get(),
            flow_total.get(),
            phase.get(),
            ui_lang(langs),
        )
        .map(|s| s.step.id)
    };
    let start_controls = props.controls.clone();
    let ext_controls = props.controls.clone();
    let browser_controls = props.controls.clone();
    let term_controls = props.controls.clone();
    let qr_controls = props.controls.clone();
    let selected = props.selected;
    let pending_qr = props.pending_qr;
    let qr_open = props.qr_open;
    let install_ca = Rc::clone(&props.install_ca);

    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(when(
            move || next_id() == Some(ChecklistStepId::ProxyListening),
            move || {
                let c = start_controls.clone();
                checklist_action_chip(tr("开始", "Start"), true, move || {
                    toggle_capture(c.clone())
                })
            },
        ))
        .child(when(
            move || {
                next_id() == Some(ChecklistStepId::RootCaTrusted)
                    && ca_ui.detected_external.with(|items| !items.is_empty())
            },
            move || {
                let c = ext_controls.clone();
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
        .child(when(
            move || next_id() == Some(ChecklistStepId::RootCaTrusted),
            move || {
                let cb = Rc::clone(&install_ca);
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
                    true,
                    move || cb.as_ref()(),
                )
            },
        ))
        .child(when(
            move || {
                matches!(
                    next_id(),
                    Some(ChecklistStepId::FirstHttp | ChecklistStepId::FirstHttps)
                )
            },
            move || {
                let b_controls = browser_controls.clone();
                let t_controls = term_controls.clone();
                view()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(checklist_action_chip(
                        tr("隔离浏览器", "Isolated Browser"),
                        true,
                        move || {
                            let url = selected_browser_start_url(&b_controls, selected.get());
                            launch_browser_action(b_controls.clone(), url);
                        },
                    ))
                    .child(checklist_action_chip(
                        tr("代理终端", "Proxied Terminal"),
                        false,
                        move || launch_terminal_action(t_controls.clone()),
                    ))
            },
        ))
        .child(when(
            move || {
                matches!(
                    next_id(),
                    Some(
                        ChecklistStepId::RootCaTrusted
                            | ChecklistStepId::FirstHttp
                            | ChecklistStepId::FirstHttps
                    )
                )
            },
            move || {
                let q_controls = qr_controls.clone();
                checklist_action_chip(tr("手机扫码", "Mobile QR"), false, move || {
                    trigger_mobile_qr(&q_controls, running, pending_qr, qr_open);
                })
            },
        ))
}

#[cfg(target_arch = "wasm32")]
fn strip_actions(
    props: &OnboardingProps,
    phase: Signal<Phase>,
    langs: Signal<Option<Lang>>,
) -> impl IntoElement {
    let running = props.running;
    let listen = props.listen;
    let ca_ui = props.ca_ui;
    let traffic = props.onboarding_traffic;
    let dismissed = props.profiles.onboarding_strip_dismissed;
    let flow_total = props.flow_total;
    let next_id = move || {
        let input = copyable_checklist_input(running, listen, ca_ui, traffic);
        onboarding_strip_state(
            &input,
            dismissed.get(),
            flow_total.get(),
            phase.get(),
            ui_lang(langs),
        )
        .map(|s| s.step.id)
    };
    let start_controls = props.controls.clone();
    let copy_controls = props.controls.clone();
    let qr_controls = props.controls.clone();
    let pending_qr = props.pending_qr;
    let qr_open = props.qr_open;

    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(when(
            move || next_id() == Some(ChecklistStepId::ProxyListening),
            move || {
                let c = start_controls.clone();
                checklist_action_chip(tr("开始", "Start"), true, move || {
                    toggle_capture(c.clone())
                })
            },
        ))
        .child(when(
            move || {
                matches!(
                    next_id(),
                    Some(
                        ChecklistStepId::RootCaTrusted
                            | ChecklistStepId::FirstHttp
                            | ChecklistStepId::FirstHttps
                    )
                )
            },
            move || {
                let c = copy_controls.clone();
                let q = qr_controls.clone();
                view()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(checklist_action_chip(
                        tr("复制代理地址", "Copy Proxy"),
                        true,
                        move || {
                            set_clipboard(client_proxy_url(&c));
                            set_status(&c, StatusMsg::CopiedUrl);
                        },
                    ))
                    .child(checklist_action_chip(
                        tr("手机扫码", "Mobile QR"),
                        false,
                        move || trigger_mobile_qr(&q, running, pending_qr, qr_open),
                    ))
            },
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checklist_input(
        running: bool,
        ca_trusted: bool,
        flows: usize,
        https: bool,
    ) -> ChecklistInput {
        ChecklistInput {
            running,
            listen_addr: if running {
                "127.0.0.1:8877".into()
            } else {
                String::new()
            },
            ca_trusted,
            ca_installed_untrusted: false,
            ca_optional_web: false,
            active_external_ca: None,
            detected_external_ca: None,
            traffic: OnboardingTraffic {
                total_flows: flows,
                has_decrypted_https: https,
                has_tls_error: false,
            },
        }
    }

    #[test]
    fn strip_hidden_when_full_onboarding_shown_or_dismissed_or_complete() {
        let partial = checklist_input(true, false, 1, false);
        assert!(
            onboarding_strip_state(&partial, false, 0, Phase::Empty, Lang::En).is_none(),
            "do not duplicate while full onboarding screen is visible"
        );
        assert!(onboarding_strip_state(&partial, true, 3, Phase::Ready, Lang::En).is_none());

        let complete = checklist_input(true, true, 5, true);
        assert!(onboarding_strip_state(&complete, false, 5, Phase::Ready, Lang::En).is_none());
    }

    #[test]
    fn strip_surfaces_next_incomplete_step_with_progress_ratio() {
        let stopped = checklist_input(false, false, 1, false);
        let summary = onboarding_strip_state(&stopped, false, 1, Phase::Ready, Lang::En).unwrap();
        assert_eq!(summary.done, 1);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.step.id, ChecklistStepId::ProxyListening);

        let needs_ca = checklist_input(true, false, 2, false);
        let summary = onboarding_strip_state(&needs_ca, false, 2, Phase::Ready, Lang::En).unwrap();
        assert_eq!(summary.done, 2);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.step.id, ChecklistStepId::RootCaTrusted);

        let needs_https = checklist_input(true, true, 2, false);
        let summary =
            onboarding_strip_state(&needs_https, false, 2, Phase::Ready, Lang::En).unwrap();
        assert_eq!(summary.done, 2);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.step.id, ChecklistStepId::FirstHttps);
    }
}
