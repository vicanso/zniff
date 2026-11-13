#[allow(unused_imports)]
use super::*;

#[allow(clippy::too_many_arguments)]
#[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
pub(crate) fn more_menu(
    open: Signal<bool>,
    ca_status: Signal<ca::CaStatus>,
    proxy_on: Signal<bool>,
    install: Rc<impl Fn() + 'static>,
    remove: Rc<impl Fn() + 'static>,
    toggle_proxy: impl Fn() + 'static,
    export_har: impl Fn() + 'static,
    copy_env: impl Fn() + 'static,
    clear_unpinned: impl Fn() + 'static,
    import_controls: Arc<Controls>,
    rows: Signal<Vec<Arc<RowLine>>>,
    filter: Signal<String>,
    pretty: Signal<bool>,
    shown_phase: Signal<Phase>,
    share_id: Signal<String>,
    profiles: ProfileSignals,
    compose: Compose,
    seq_builder: SeqBuilder,
    selected: Signal<Option<u64>>,
    update_state: Signal<UpdateState>,
) -> impl IntoElement {
    let items = view()
        .w_full()
        .p_1()
        .gap_0p5()
        .child(menu_export_row(
            open,
            export_har,
            import_controls.clone(),
            rows,
            filter,
        ))
        .child(menu_redact_row(
            open,
            import_controls.clone(),
            rows,
            seq_builder,
            compose,
            selected,
            shown_phase,
        ))
        .child(menu_share_row(
            open,
            import_controls.clone(),
            filter,
            share_id,
            profiles,
        ))
        .child(menu_action_enabled(
            Glyph::List,
            tr("生成时序图", "Sequence diagram"),
            move || {
                let len = rows.with(Vec::len);
                if len > 0 {
                    SharedString::from(len.to_string())
                } else {
                    SharedString::default()
                }
            },
            move || !rows.with(Vec::is_empty),
            {
                let controls = import_controls.clone();
                move || {
                    begin_sequence_diagram(
                        &controls,
                        rows,
                        seq_builder,
                        compose,
                        selected,
                        shown_phase,
                    );
                }
            },
        ))
        .child(menu_action(
            Glyph::Prompt,
            tr("构造请求", "New request"),
            move || begin_new_request(compose, selected),
        ));
    let items = desktop_child!(
        items,
        menu_action(Glyph::List, tr("导入 HAR", "Import HAR"), {
            let controls = import_controls.clone();
            move || prompt_import_har(controls.clone())
        })
    );
    let items = items.child(menu_action(
        Glyph::Trash,
        tr("清空未星标", "Clear unstarred"),
        clear_unpinned,
    ));
    let items = desktop_child!(items, menu_rule());
    let items = desktop_child!(items, proxy_menu_item(proxy_on, toggle_proxy));
    let items = desktop_child!(
        items,
        menu_copy_config_row(open, copy_env, import_controls.clone())
    );
    let items = desktop_child!(items, menu_rule());
    let items = desktop_child!(items, cert_menu_row(open, ca_status, install, remove));
    let items = desktop_child!(items, platform_ca_extra_menu_item(&import_controls));
    let items = items
        .child(menu_rule())
        .child(more_check_action(
            move || pretty.get(),
            tr("默认美化", "Pretty by default"),
            {
                let controls = import_controls.clone();
                move || {
                    let next = !pretty.get();
                    pretty.set(next);
                    if let Err(err) = save_config(|config| config.default_pretty = next) {
                        set_status(
                            &controls,
                            StatusMsg::PrefUnsaved {
                                name: PrefName::Pretty,
                                err: err.to_string(),
                            },
                        );
                    }
                }
            },
        ))
        .child(menu_lang_row(import_controls.clone()))
        .child(menu_action_meta(
            Glyph::Sliders,
            tr("服务与中继设置", "Relay & Service Settings"),
            {
                let langs = ui_langs();
                move || {
                    let has_relay = !profiles.relay_url.get().trim().is_empty();
                    let has_auth = !profiles.proxy_auth.get().trim().is_empty();
                    let label = match (has_relay, has_auth) {
                        (true, true) => t(ui_lang(langs), "中转 · 认证", "Relay · Auth"),
                        (true, false) => t(ui_lang(langs), "中转", "Relay"),
                        (false, true) => t(ui_lang(langs), "认证", "Auth"),
                        (false, false) => "",
                    };
                    SharedString::from(label)
                }
            },
            move || {
                open.set(false);
                profiles.machine_settings_open.set(true);
            },
        ));
    let items = desktop_child!(items, menu_rule());
    let items = desktop_child!(items, update_menu_section(update_state, import_controls));
    view()
        .w(px(288.0))
        .max_w(move || px((window().width.0 - 16.0).max(180.0)))
        .max_h(move || px((window().height.0 - 68.0).max(180.0)))
        .overflow_hidden()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
        .child(
            scroll()
                .w_full()
                .max_h(move || px((window().height.0 - 70.0).max(178.0)))
                .child(items),
        )
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn toolbar_update_pill(
    update_state: Signal<UpdateState>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    when(
        move || update_state.get().has_toolbar_pill(),
        move || {
            let controls_press = controls.clone();
            let langs = ui_langs();
            view()
                .window_drag(false)
                .h(px(CONTROL_H))
                .px_2p5()
                .gap_1p5()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .rounded(px(CONTROL_RADIUS))
                .border_1()
                .border_color(accent_blue_border())
                .bg(accent_blue_soft())
                .hover(|style| {
                    style.bg(move || {
                        if update_state.get().is_busy() {
                            accent_blue_soft()
                        } else {
                            accent_blue_soft_hover()
                        }
                    })
                })
                .cursor(Cursor::Pointer)
                .active(|style| {
                    style.opacity(move || {
                        if update_state.get().is_busy() {
                            1.0_f32
                        } else {
                            0.8_f32
                        }
                    })
                })
                .disabled(move || update_state.get().is_busy())
                .disabled_style(|style| style.cursor(Cursor::Default))
                .on_press(move || {
                    let st = update_state.get();
                    match st.phase {
                        UpdatePhase::ReadyToRestart { .. } => {
                            relaunch_and_exit(&controls_press);
                        }
                        UpdatePhase::OpenedInstaller { .. } => {
                            quit_for_manual_install(&controls_press);
                        }
                        UpdatePhase::Idle => {
                            if let Some(version) = st.nudge {
                                start_upgrade(controls_press.clone(), version);
                            }
                        }
                        UpdatePhase::Checking
                        | UpdatePhase::Downloading { .. }
                        | UpdatePhase::Installing { .. } => {}
                    }
                })
                .child(glyph(|| Glyph::Download, || color(accent_blue_text())))
                .child(
                    control_text(move || {
                        let lang = ui_lang(langs);
                        let st = update_state.get();
                        let label = match &st.phase {
                            UpdatePhase::Downloading {
                                version,
                                received,
                                total,
                            } => {
                                if *total > 0 {
                                    let pct = ((*received as f64 / *total as f64) * 100.0)
                                        .round()
                                        .clamp(0.0, 100.0)
                                        as u32;
                                    format!("{} {version} · {pct}%", t(lang, "下载", "Downloading"))
                                } else {
                                    format!(
                                        "{} {version} · {}",
                                        t(lang, "下载", "Downloading"),
                                        format_bytes(*received as usize)
                                    )
                                }
                            }
                            UpdatePhase::Installing { version } => {
                                format!("{} {version}…", t(lang, "安装", "Installing"))
                            }
                            UpdatePhase::ReadyToRestart { version } => {
                                format!("{} {version}", t(lang, "重启完成", "Restart for"))
                            }
                            UpdatePhase::OpenedInstaller { .. } => {
                                t(lang, "退出完成安装", "Quit to install").to_string()
                            }
                            UpdatePhase::Idle | UpdatePhase::Checking => {
                                let ver = st.nudge.as_deref().unwrap_or_default();
                                format!("{} {ver}", t(lang, "升级", "Upgrade"))
                            }
                        };
                        SharedString::from(label)
                    })
                    .text_color(accent_blue_text()),
                )
        },
    )
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn update_menu_section(
    update_state: Signal<UpdateState>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let controls_upgrade = controls.clone();
    let controls_skip = controls.clone();
    let controls_restart = controls.clone();
    let controls_quit = controls.clone();
    let controls_unskip = controls.clone();
    let controls_check = controls;
    view()
        .gap_0p5()
        .child(when(
            move || {
                let st = update_state.get();
                matches!(st.phase, UpdatePhase::Idle) && st.nudge.is_some()
            },
            move || {
                let controls_up = controls_upgrade.clone();
                let controls_sk = controls_skip.clone();
                let langs = ui_langs();
                view()
                    .gap_0p5()
                    .child(menu_action_meta(
                        Glyph::Download,
                        move || {
                            let ver = update_state.get().nudge.unwrap_or_default();
                            SharedString::from(format!(
                                "{} {ver}",
                                t(ui_lang(langs), "升级到", "Upgrade to")
                            ))
                        },
                        move || {
                            let ver = update_state.get().nudge.unwrap_or_default();
                            SharedString::from(format!("v{} → {ver}", updater::version()))
                        },
                        move || {
                            if let Some(ver) = update_state.get().nudge {
                                start_upgrade(controls_up.clone(), ver);
                            }
                        },
                    ))
                    .child(menu_action_meta(
                        Glyph::Stop,
                        tr("跳过此版本", "Skip this version"),
                        move || SharedString::from(update_state.get().nudge.unwrap_or_default()),
                        move || {
                            if let Some(ver) = update_state.get().nudge {
                                skip_update(&controls_sk, &ver);
                            }
                        },
                    ))
            },
        ))
        .child(when(
            move || matches!(update_state.get().phase, UpdatePhase::Downloading { .. }),
            move || {
                menu_action_enabled(
                    Glyph::Download,
                    tr("正在下载更新…", "Downloading update…"),
                    move || match update_state.get().phase {
                        UpdatePhase::Downloading {
                            received, total, ..
                        } => {
                            if total > 0 {
                                SharedString::from(format!(
                                    "{} / {}",
                                    format_bytes(received as usize),
                                    format_bytes(total as usize)
                                ))
                            } else {
                                SharedString::from(format_bytes(received as usize))
                            }
                        }
                        _ => SharedString::default(),
                    },
                    || false,
                    || {},
                )
            },
        ))
        .child(when(
            move || matches!(update_state.get().phase, UpdatePhase::Installing { .. }),
            move || {
                menu_action_enabled(
                    Glyph::Download,
                    tr("正在安装更新…", "Installing update…"),
                    move || match update_state.get().phase {
                        UpdatePhase::Installing { version } => SharedString::from(version),
                        _ => SharedString::default(),
                    },
                    || false,
                    || {},
                )
            },
        ))
        .child(when(
            move || matches!(update_state.get().phase, UpdatePhase::ReadyToRestart { .. }),
            move || {
                let controls = controls_restart.clone();
                menu_action_meta(
                    Glyph::Play,
                    tr("立即重启完成更新", "Restart to finish update"),
                    move || match update_state.get().phase {
                        UpdatePhase::ReadyToRestart { version } => SharedString::from(version),
                        _ => SharedString::default(),
                    },
                    move || relaunch_and_exit(&controls),
                )
            },
        ))
        .child(when(
            move || {
                matches!(
                    update_state.get().phase,
                    UpdatePhase::OpenedInstaller { .. }
                )
            },
            move || {
                let controls = controls_quit.clone();
                menu_action_meta(
                    Glyph::Stop,
                    tr("退出以完成安装", "Quit to finish install"),
                    move || match update_state.get().phase {
                        UpdatePhase::OpenedInstaller { version } => SharedString::from(version),
                        _ => SharedString::default(),
                    },
                    move || quit_for_manual_install(&controls),
                )
            },
        ))
        .child(when(
            move || {
                let st = update_state.get();
                matches!(st.phase, UpdatePhase::Idle) && st.nudge.is_none() && st.ignored.is_some()
            },
            move || {
                let controls = controls_unskip.clone();
                menu_action_meta(
                    Glyph::Download,
                    tr("恢复版本提醒", "Unskip update"),
                    move || SharedString::from(update_state.get().ignored.unwrap_or_default()),
                    move || unskip_update(&controls),
                )
            },
        ))
        .child(menu_action_enabled(
            Glyph::Download,
            tr("检查更新", "Check for updates"),
            {
                let langs = ui_langs();
                move || {
                    let st = update_state.get();
                    if matches!(st.phase, UpdatePhase::Checking) {
                        SharedString::from(t(ui_lang(langs), "检查中…", "Checking…"))
                    } else {
                        SharedString::from(format!("v{}", updater::version()))
                    }
                }
            },
            move || !update_state.get().is_busy(),
            move || check_update_manual(controls_check.clone()),
        ))
}
