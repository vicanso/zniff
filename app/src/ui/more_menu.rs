#[allow(unused_imports)]
use super::*;

#[allow(clippy::too_many_arguments)]
#[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
pub(crate) fn more_menu(
    open: Signal<bool>,
    export_har: impl Fn() + 'static,
    copy_env: impl Fn() + 'static,
    clear_unpinned: impl Fn() + 'static,
    import_controls: Arc<Controls>,
    rows: Signal<Vec<Arc<RowLine>>>,
    filter: Signal<String>,
    shown_phase: Signal<Phase>,
    share_id: Signal<String>,
    profiles: ProfileSignals,
    compose: Compose,
    seq_builder: SeqBuilder,
    selected: Signal<Option<u64>>,
    update_state: Signal<UpdateState>,
) -> impl IntoElement {
    let apple = is_apple_platform();
    let settings_shortcut = format_shortcut_badge("mod+,", apple);
    let shortcuts_key = format_shortcut_badge("mod+/", apple);

    // Group 1: Export (<= 6 items)
    let items = view()
        .w_full()
        .p_1()
        .gap_0p5()
        .child(menu_group_label("导出", "Export"))
        .child(menu_export_row(
            open,
            export_har,
            import_controls.clone(),
            rows,
            filter,
        ))
        .child(menu_openapi_row(open, import_controls.clone(), filter))
        .child(menu_collections_row(open, import_controls.clone(), filter));
    let items = desktop_child!(
        items,
        menu_action(
            Glyph::Download,
            tr("生成本地 Mock…", "Export Mock dir…"),
            {
                let controls = import_controls.clone();
                let rules_text = profiles.rules;
                move || {
                    open.set(false);
                    export_mock_dir(controls.clone(), &filter.get(), rules_text);
                }
            }
        )
    );
    let items = items
        .child(menu_share_row(
            open,
            import_controls.clone(),
            filter,
            share_id,
            profiles,
        ))
        .child(menu_redact_row(
            open,
            import_controls.clone(),
            rows,
            seq_builder,
            compose,
            selected,
            shown_phase,
        ));

    // Group 2: Tools (<= 6 items)
    let items = items
        .child(menu_rule())
        .child(menu_group_label("工具", "Tools"))
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
                    open.set(false);
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
        .child(
            view()
                .h(px(32.0))
                .px_2p5()
                .gap_2()
                .rounded(px(6.0))
                .flex_row()
                .items_center()
                .child(glyph(|| Glyph::Prompt, || color(fg_secondary())))
                .child(
                    control_text(tr("新建", "New"))
                        .flex_1()
                        .text_color(fg_secondary()),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(menu_inline_pill(
                            tr("构造请求", "Request"),
                            || false,
                            move || {
                                open.set(false);
                                begin_new_request(compose, selected);
                            },
                        ))
                        .child(
                            view()
                                .tooltip(label_tip("插入会话标记", "Insert marker"))
                                .child(menu_inline_pill(
                                    tr("会话标记", "Marker"),
                                    || false,
                                    move || {
                                        open.set(false);
                                        profiles.marker_modal_open.set(true);
                                    },
                                )),
                        ),
                ),
        );
    let items = desktop_child!(
        items,
        menu_import_row(open, import_controls.clone(), profiles)
    );
    let items = desktop_child!(
        items,
        menu_quick_launch_row(open, import_controls.clone(), selected)
    );
    let items = desktop_child!(
        items,
        menu_copy_config_row(open, copy_env, import_controls.clone())
    );
    let items = items.child(menu_clear_unstarred_row(open, rows, clear_unpinned));

    // Group 3: Help & Settings (<= 6 items)
    let items = items
        .child(menu_rule())
        .child(menu_group_label("帮助", "Help"))
        .child(menu_action_meta(
            Glyph::Sliders,
            tr("设置…", "Settings…"),
            move || SharedString::from(settings_shortcut.clone()),
            move || {
                open.set(false);
                open_settings_tab(profiles, SettingsTab::General);
            },
        ))
        .child(menu_action_meta(
            Glyph::List,
            tr("快捷键速查", "Keyboard shortcuts"),
            move || SharedString::from(shortcuts_key.clone()),
            move || {
                open.set(false);
                profiles.shortcuts_open.set(true);
            },
        ));
    let items = desktop_child!(
        items,
        menu_action_enabled(
            Glyph::Download,
            tr("检查更新", "Check for updates"),
            {
                let langs = ui_langs();
                move || {
                    let lang = ui_lang(langs);
                    let st = update_state.get();
                    if let Some(ver) = st.nudge {
                        SharedString::from(ver)
                    } else if matches!(st.phase, UpdatePhase::Checking) {
                        SharedString::from(t(lang, "检查中…", "Checking…"))
                    } else {
                        SharedString::default()
                    }
                }
            },
            move || !update_state.get().is_busy(),
            {
                let controls = import_controls.clone();
                move || {
                    open.set(false);
                    if update_state.get().nudge.is_some() {
                        open_settings_tab(profiles, SettingsTab::Updates);
                    } else {
                        check_update_manual(controls.clone());
                    }
                }
            },
        )
    );
    let diag_controls = import_controls.clone();
    let diag_label = if cfg!(target_arch = "wasm32") {
        tr("复制诊断包", "Copy Diagnostics")
    } else {
        tr("导出诊断包…", "Export Diagnostics…")
    };
    let items = items.child(menu_action(Glyph::Download, diag_label, move || {
        open.set(false);
        export_diagnostics_file(diag_controls.clone());
    }));
    let items = items.child(menu_action_meta(
        Glyph::Badge,
        tr("关于 zniff", "About zniff"),
        || SharedString::from(format!("v{}", env!("CARGO_PKG_VERSION"))),
        move || {
            open.set(false);
            profiles.about_open.set(true);
        },
    ));
    view()
        .w(px(304.0))
        .max_w(move || px((window().width.0 - 16.0).max(180.0)))
        .max_h(move || px((window().height.0 - 68.0).max(180.0)))
        .overflow_hidden()
        .child(
            scroll()
                .w_full()
                .max_h(move || px((window().height.0 - 70.0).max(178.0)))
                .child(items),
        )
}

fn menu_clear_unstarred_row(
    open: Signal<bool>,
    rows: Signal<Vec<Arc<RowLine>>>,
    clear_unpinned: impl Fn() + 'static,
) -> impl IntoElement {
    let langs = ui_langs();
    let has_unpinned = move || rows.with(|list| list.iter().any(|r| !r.pinned));
    let confirm_clear = signal(false);
    let clear_job = stored(None::<Spawned<()>>);
    effect(move || {
        if !open.get() {
            clear_job.replace(None);
            confirm_clear.set(false);
        }
    });
    menu_item()
        .disabled(move || !has_unpinned())
        .disabled_style(|s| s.opacity(0.45))
        .close_on_select(false)
        .on_select(move || {
            if !has_unpinned() {
                return;
            }
            if confirm_clear.get_untracked() {
                clear_job.replace(None);
                confirm_clear.set(false);
                open.set(false);
                clear_unpinned();
            } else {
                confirm_clear.set(true);
                clear_job.replace(Some(set_timeout(Duration::from_millis(3500), move || {
                    confirm_clear.set(false);
                })));
            }
        })
        .h(px(ControlSize::Md.height()))
        .px_2p5()
        .gap_2()
        .rounded(px(RADIUS_MD))
        .flex_row()
        .items_center()
        .hover(move |style| {
            style.bg(move || {
                if !has_unpinned() {
                    AdaptiveColor::TRANSPARENT
                } else if confirm_clear.get() {
                    status_warn_bg_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(move |style| {
            style.bg(move || {
                if has_unpinned() {
                    bg_control_active()
                } else {
                    AdaptiveColor::TRANSPARENT
                }
            })
        })
        .transition_colors()
        .duration(move || motion_ms(MOTION_INSTANT_MS))
        .ease_out()
        .focus_visible(apply_focus_ring)
        .child(glyph(
            || Glyph::Trash,
            move || {
                color(if confirm_clear.get() {
                    status_warn_fg()
                } else if has_unpinned() {
                    fg_secondary()
                } else {
                    fg_muted()
                })
            },
        ))
        .child(
            control_text(move || -> SharedString {
                let lang = ui_lang(langs);
                if confirm_clear.get() {
                    t(lang, "再次点击确认清空", "Click again to clear")
                } else {
                    t(lang, "清空未星标", "Clear unstarred")
                }
                .into()
            })
            .flex_1()
            .text_color(move || {
                if confirm_clear.get() {
                    status_warn_fg()
                } else if has_unpinned() {
                    fg_secondary()
                } else {
                    fg_muted()
                }
            }),
        )
}

pub(crate) fn format_update_header(st: &UpdateState, lang: Lang) -> String {
    match &st.phase {
        UpdatePhase::Downloading {
            version,
            received,
            total,
        } => {
            if *total > 0 {
                format!(
                    "{} {version} ({} / {})",
                    t(lang, "正在下载", "Downloading"),
                    format_bytes(*received as usize),
                    format_bytes(*total as usize),
                )
            } else {
                format!(
                    "{} {version} ({})",
                    t(lang, "正在下载", "Downloading"),
                    format_bytes(*received as usize),
                )
            }
        }
        UpdatePhase::Installing { version } => {
            format!(
                "{} {version}…",
                t(lang, "正在安装更新", "Installing update")
            )
        }
        UpdatePhase::ReadyToRestart { version } => {
            format!(
                "{} {version}",
                t(
                    lang,
                    "更新已就绪，点击立即重启",
                    "Update ready, click to restart"
                )
            )
        }
        UpdatePhase::OpenedInstaller { version } => {
            format!(
                "{} {version}",
                t(
                    lang,
                    "已打开安装包，点击退出以完成安装",
                    "Installer opened, click to quit and finish"
                )
            )
        }
        UpdatePhase::Idle | UpdatePhase::Checking => {
            let ver = st.nudge.as_deref().unwrap_or_default();
            if ver.is_empty() {
                t(lang, "检查更新", "Check for updates").to_string()
            } else {
                format!(
                    "{} {ver} · {}",
                    t(lang, "发现新版本", "New version"),
                    t(lang, "点击更新并自动重启", "Click to update and restart")
                )
            }
        }
    }
}

pub(crate) fn format_update_notes_markdown(st: &UpdateState) -> Option<String> {
    st.notes
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
pub(crate) fn format_update_tooltip(st: &UpdateState, lang: Lang) -> String {
    let header = format_update_header(st, lang);
    match format_update_notes_markdown(st) {
        Some(body) => format!("{header}\n\n{body}"),
        None => header,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn update_hover_card(
    trigger: impl IntoElement,
    placement: Placement,
    align: Align,
    update_state: Signal<UpdateState>,
    langs: Signal<Option<Lang>>,
) -> impl IntoElement {
    nsh_base::hover_card()
        .open_delay(250.0)
        .close_delay(250.0)
        .placement(placement)
        .align(align)
        .offset(px(6.0))
        .margin(px(10.0))
        .popup_style(ui_popup_style)
        .popup_starting_style(ui_popup_starting_style)
        .popup_ending_style(ui_popup_ending_style)
        .trigger(trigger)
        .child(
            view()
                .max_w(px(460.0))
                .px_3()
                .py_2p5()
                .gap_2()
                .child(
                    text(move || format_update_header(&update_state.get(), ui_lang(langs)))
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg_primary()),
                )
                .child(when(
                    move || {
                        update_state
                            .with(|st| st.notes.as_deref().is_some_and(|s| !s.trim().is_empty()))
                    },
                    move || {
                        view()
                            .w(px(436.0))
                            .max_h(px(360.0))
                            .pt_2()
                            .border_t_1()
                            .border_t_color(border_subtle())
                            .child(
                                scroll().w_full().max_h(px(348.0)).pr_2().child(
                                    nsh_base::text_view::text_view()
                                        .document(move || {
                                            format_update_notes_markdown(&update_state.get()).map(
                                                |md| {
                                                    nsh_base::text_view::Parser::new().markdown(&md)
                                                },
                                            )
                                        })
                                        .selectable(true)
                                        .render_image(|img: &nsh_base::text_view::Image| {
                                            let label = if img.alt.is_empty() {
                                                format!("[img: {}]", img.src)
                                            } else {
                                                format!("[img: {}]", img.alt)
                                            };
                                            mono(label)
                                                .text_size(px(TEXT_XS))
                                                .text_color(fg_muted())
                                        }),
                                ),
                            )
                    },
                )),
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
            let pill = view()
                .window_drag(false)
                .h(px(CONTROL_H))
                .px_2()
                .gap_1p5()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .rounded(px(RADIUS_MD))
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
                .active(|style| {
                    style.bg(move || {
                        if update_state.get().is_busy() {
                            accent_blue_soft()
                        } else {
                            bg_control_active()
                        }
                    })
                })
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .disabled(move || update_state.get().is_busy())
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
                .child(glyph_px(
                    ControlSize::Sm.icon_size(),
                    || Glyph::Download,
                    || color(accent_blue_text()),
                ))
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
                                format!("{} {ver}", t(lang, "更新并重启", "Update & restart"))
                            }
                        };
                        SharedString::from(label)
                    })
                    .text_color(accent_blue_text()),
                );
            update_hover_card(pill, Placement::Bottom, Align::End, update_state, langs)
        },
    )
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn update_menu_section(
    open: Option<Signal<bool>>,
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
                    .child(update_hover_card(
                        menu_action_meta(
                            Glyph::Download,
                            move || {
                                let ver = update_state.get().nudge.unwrap_or_default();
                                SharedString::from(format!(
                                    "{} {ver}",
                                    t(ui_lang(langs), "更新并重启", "Update & restart")
                                ))
                            },
                            move || {
                                let ver = update_state.get().nudge.unwrap_or_default();
                                SharedString::from(format!("v{} → {ver}", updater::version()))
                            },
                            move || {
                                if let Some(menu_open) = open {
                                    menu_open.set(false);
                                }
                                if let Some(ver) = update_state.get().nudge {
                                    start_upgrade(controls_up.clone(), ver);
                                }
                            },
                        ),
                        Placement::Left,
                        Align::Start,
                        update_state,
                        langs,
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
                    move || {
                        if let Some(menu_open) = open {
                            menu_open.set(false);
                        }
                        relaunch_and_exit(&controls);
                    },
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
                    move || {
                        if let Some(menu_open) = open {
                            menu_open.set(false);
                        }
                        quit_for_manual_install(&controls);
                    },
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
                    let lang = ui_lang(langs);
                    let st = update_state.get();
                    if matches!(st.phase, UpdatePhase::Checking) {
                        SharedString::from(t(lang, "检查中…", "Checking…"))
                    } else {
                        match st.last_check {
                            Some(UpdateCheckNotice::UpToDate(ref ver)) => SharedString::from(
                                t(lang, "已是最新 ", "Up to date ").to_string()
                                    + &format!("v{ver}"),
                            ),
                            Some(UpdateCheckNotice::Available(ref ver)) => {
                                SharedString::from(ver.clone())
                            }
                            Some(UpdateCheckNotice::Failed(_)) => {
                                SharedString::from(t(lang, "检查失败", "Failed"))
                            }
                            None => SharedString::default(),
                        }
                    }
                }
            },
            move || !update_state.get().is_busy(),
            move || {
                if let Some(menu_open) = open {
                    menu_open.set(false);
                }
                check_update_manual(controls_check.clone());
            },
        ))
}
