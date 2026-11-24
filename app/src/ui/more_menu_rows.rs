#[allow(unused_imports)]
use super::*;

pub(crate) fn menu_group_label(zh: &'static str, en: &'static str) -> impl IntoElement {
    view().px_2p5().pt_1().pb_0p5().child(
        text(tr(zh, en))
            .number_of_lines(1)
            .text_size(px(TEXT_XS))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(fg_muted()),
    )
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn menu_import_row(
    open: Signal<bool>,
    controls: Arc<Controls>,
    profiles: ProfileSignals,
) -> impl IntoElement {
    let har_controls = controls.clone();
    let rules_controls = controls;
    view()
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .child(glyph(|| Glyph::Download, || color(fg_secondary())))
        .child(
            control_text(tr("导入", "Import"))
                .flex_1()
                .text_color(fg_secondary()),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(menu_inline_pill(
                    "HAR/.chlsj",
                    || false,
                    move || {
                        open.set(false);
                        prompt_import_har(har_controls.clone());
                    },
                ))
                .child(menu_inline_pill(
                    tr("规则迁移", "Rules"),
                    || false,
                    move || {
                        open.set(false);
                        prompt_import_migration(rules_controls.clone(), profiles);
                    },
                )),
        )
}

pub(crate) fn menu_action(
    icon: Glyph,
    label: impl IntoReactive<SharedString>,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    menu_item()
        .on_select(on_select)
        .h(px(ControlSize::Md.height()))
        .px_2p5()
        .gap_2()
        .rounded(px(RADIUS_MD))
        .flex_row()
        .items_center()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .child(glyph(move || icon, || color(fg_secondary())))
        .child(control_text(label).text_color(fg_secondary()))
}

pub(crate) fn menu_action_meta(
    icon: Glyph,
    label: impl IntoReactive<SharedString>,
    meta: impl Fn() -> SharedString + 'static,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    menu_item()
        .on_select(on_select)
        .h(px(ControlSize::Md.height()))
        .px_2p5()
        .gap_2()
        .rounded(px(RADIUS_MD))
        .flex_row()
        .items_center()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .child(glyph(move || icon, || color(fg_secondary())))
        .child(control_text(label).flex_1().text_color(fg_secondary()))
        .child(
            mono(meta)
                .tabular_nums()
                .flex_shrink_0()
                .number_of_lines(1)
                .text_size(px(TEXT_XS))
                .text_color(fg_muted()),
        )
}

pub(crate) fn menu_action_enabled(
    icon: Glyph,
    label: impl IntoReactive<SharedString>,
    meta: impl Fn() -> SharedString + 'static,
    enabled: impl Fn() -> bool + 'static + Clone,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    let enabled_off = enabled.clone();
    let enabled_run = enabled.clone();
    let enabled_hover = enabled.clone();
    let enabled_active = enabled.clone();
    let enabled_icon = enabled.clone();
    let enabled_text = enabled;
    menu_item()
        .disabled(move || !enabled_off())
        .disabled_style(|style| style.opacity(0.45))
        .on_select(move || {
            if enabled_run() {
                on_select();
            }
        })
        .h(px(ControlSize::Md.height()))
        .px_2p5()
        .gap_2()
        .rounded(px(RADIUS_MD))
        .flex_row()
        .items_center()
        .hover(|style| {
            style.bg(move || {
                if enabled_hover() {
                    bg_control_hover()
                } else {
                    AdaptiveColor::TRANSPARENT
                }
            })
        })
        .active(|style| {
            style.bg(move || {
                if enabled_active() {
                    bg_control_active()
                } else {
                    AdaptiveColor::TRANSPARENT
                }
            })
        })
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .child(glyph(
            move || icon,
            move || {
                color(if enabled_icon() {
                    fg_secondary()
                } else {
                    fg_muted()
                })
            },
        ))
        .child(control_text(label).flex_1().text_color(move || {
            if enabled_text() {
                fg_secondary()
            } else {
                fg_muted()
            }
        }))
        .child(
            mono(meta)
                .tabular_nums()
                .flex_shrink_0()
                .number_of_lines(1)
                .text_size(px(TEXT_XS))
                .text_color(fg_muted()),
        )
}

pub(crate) fn menu_inline_pill(
    label: impl IntoReactive<SharedString>,
    active: impl Fn() -> bool + 'static + Copy,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .role(AccessibilityRole::Button)
        .h(px(ControlSize::Xs.height()))
        .px_2()
        .rounded(px(RADIUS_SM))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_center()
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
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .on_press(on_press)
        .child(compact_control_text(label, TEXT_XS).text_color(move || {
            if active() {
                accent_blue_text()
            } else {
                fg_secondary()
            }
        }))
}

pub(crate) fn menu_export_row(
    open: Signal<bool>,
    export_har: impl Fn() + 'static,
    controls: Arc<Controls>,
    rows: Signal<Vec<Arc<RowLine>>>,
    filter: Signal<String>,
) -> impl IntoElement {
    let langs = ui_langs();
    view()
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .child(glyph(|| Glyph::Download, || color(fg_secondary())))
        .child(
            control_text(tr("导出", "Export"))
                .flex_1()
                .text_color(fg_secondary()),
        )
        .child(when(
            move || !filter.get().is_empty(),
            move || {
                mono(move || SharedString::from(rows.with(Vec::len).to_string()))
                    .tabular_nums()
                    .flex_shrink_0()
                    .number_of_lines(1)
                    .text_size(px(11.0))
                    .text_color(fg_muted())
            },
        ))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(menu_inline_pill(
                    "HAR",
                    || false,
                    move || {
                        open.set(false);
                        export_har();
                    },
                ))
                .child(menu_inline_pill(
                    "HTML",
                    || false,
                    move || {
                        open.set(false);
                        export_html_file(
                            controls.clone(),
                            &filter.get(),
                            ui_lang(langs) == Lang::Zh,
                        );
                    },
                )),
        )
}

pub(crate) fn menu_openapi_row(
    open: Signal<bool>,
    controls: Arc<Controls>,
    filter: Signal<String>,
) -> impl IntoElement {
    let json_controls = controls.clone();
    let yaml_controls = controls.clone();
    #[cfg(not(target_arch = "wasm32"))]
    let copy_controls = controls;
    let pills = view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(menu_inline_pill(
            "JSON",
            || false,
            move || {
                open.set(false);
                export_openapi_file(json_controls.clone(), &filter.get(), false);
            },
        ))
        .child(menu_inline_pill(
            "YAML",
            || false,
            move || {
                open.set(false);
                export_openapi_file(yaml_controls.clone(), &filter.get(), true);
            },
        ));
    let pills = desktop_child!(
        pills,
        menu_inline_pill(
            tr("复制", "Copy"),
            || false,
            move || {
                open.set(false);
                copy_openapi_yaml(&copy_controls, &filter.get());
            }
        )
    );
    view()
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .child(glyph(|| Glyph::Badge, || color(fg_secondary())))
        .child(control_text("OpenAPI").flex_1().text_color(fg_secondary()))
        .child(pills)
}

pub(crate) fn menu_collections_row(
    open: Signal<bool>,
    controls: Arc<Controls>,
    filter: Signal<String>,
) -> impl IntoElement {
    let postman_controls = controls.clone();
    let bruno_controls = controls.clone();
    let http_controls = controls;
    let pills = view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(menu_inline_pill(
            "Postman",
            || false,
            move || {
                open.set(false);
                export_collection_file(
                    postman_controls.clone(),
                    &filter.get(),
                    collections::CollectionFormat::Postman,
                );
            },
        ))
        .child(menu_inline_pill(
            "Bruno",
            || false,
            move || {
                open.set(false);
                export_collection_file(
                    bruno_controls.clone(),
                    &filter.get(),
                    collections::CollectionFormat::Bruno,
                );
            },
        ))
        .child(menu_inline_pill(
            ".http",
            || false,
            move || {
                open.set(false);
                export_collection_file(
                    http_controls.clone(),
                    &filter.get(),
                    collections::CollectionFormat::Http,
                );
            },
        ));
    view()
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .child(glyph(|| Glyph::Download, || color(fg_secondary())))
        .child(
            control_text(tr("集合", "Collection"))
                .flex_1()
                .text_color(fg_secondary()),
        )
        .child(pills)
}

pub(crate) fn menu_redact_row(
    open: Signal<bool>,
    controls: Arc<Controls>,
    rows: Signal<Vec<Arc<RowLine>>>,
    seq_builder: SeqBuilder,
    compose: Compose,
    selected: Signal<Option<u64>>,
    shown_phase: Signal<Phase>,
) -> impl IntoElement {
    let marked = move || seq_builder.redact_export.get();
    let toggle_controls = controls.clone();
    view()
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .child(
            view()
                .role(AccessibilityRole::Checkbox)
                .flex_1()
                .h_full()
                .flex_row()
                .items_center()
                .gap_2()
                .rounded(px(RADIUS_SM))
                .hover(|style| style.bg(bg_control_hover()))
                .active(|style| style.bg(bg_control_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .on_press(move || {
                    let next = !seq_builder.redact_export.get();
                    seq_builder.redact_export.set(next);
                    *toggle_controls
                        .redact_export
                        .lock()
                        .unwrap_or_else(|err| err.into_inner()) = next;
                    save_config_or_notify(&toggle_controls, PrefName::Redact, |cfg| {
                        cfg.redact_export = next
                    });
                })
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
                                .rounded(px(RADIUS_SM))
                                .border_1()
                                .border_color(
                                    move || {
                                        if marked() { accent_blue() } else { fg_muted() }
                                    },
                                )
                                .bg(move || if marked() { accent_blue() } else { bg_card() }),
                        ),
                )
                .child(
                    control_text(tr("导出/分享脱敏", "Redact exports")).text_color(move || {
                        if marked() {
                            accent_blue_text()
                        } else {
                            fg_secondary()
                        }
                    }),
                ),
        )
        .child(menu_inline_pill(
            tr("预览", "Preview"),
            move || seq_builder.redact_open.get(),
            move || {
                open.set(false);
                begin_redact_preview(&controls, rows, seq_builder, compose, selected, shown_phase);
            },
        ))
}

pub(crate) fn menu_share_row(
    open: Signal<bool>,
    controls: Arc<Controls>,
    filter: Signal<String>,
    share_id: Signal<String>,
    profiles: ProfileSignals,
) -> impl IntoElement {
    let c15 = controls.clone();
    let c60 = controls.clone();
    let c480 = controls.clone();
    view()
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .child(glyph(|| Glyph::Prompt, || color(fg_secondary())))
        .child(
            control_text(tr("实时分享", "Live share"))
                .flex_1()
                .text_color(fg_secondary()),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(menu_inline_pill(
                    "15m",
                    || false,
                    move || {
                        open.set(false);
                        begin_share(&c15, 15, &filter.get(), share_id);
                    },
                ))
                .child(menu_inline_pill(
                    "1h",
                    || false,
                    move || {
                        open.set(false);
                        begin_share(&c60, 60, &filter.get(), share_id);
                    },
                ))
                .child(menu_inline_pill(
                    "8h",
                    || false,
                    move || {
                        open.set(false);
                        begin_share(&c480, 480, &filter.get(), share_id);
                    },
                ))
                .child(when(
                    move || !share_id.get().is_empty(),
                    move || {
                        let controls = controls.clone();
                        view()
                            .role(AccessibilityRole::Button)
                            .h(px(ControlSize::Xs.height()))
                            .px_2()
                            .rounded(px(RADIUS_SM))
                            .flex_shrink_0()
                            .flex_row()
                            .items_center()
                            .justify_center()
                            .border_1()
                            .border_color(status_err_fg())
                            .bg(status_err_bg())
                            .hover(|style| style.bg(status_err_bg_hover()))
                            .active(|style| style.bg(bg_control_active()))
                            .transition_colors()
                            .duration(MOTION_INSTANT_MS)
                            .ease_out()
                            .focus_visible(apply_focus_ring)
                            .on_press(move || {
                                open.set(false);
                                end_share(&controls, share_id);
                            })
                            .child(
                                compact_control_text(tr("停止", "Stop"), TEXT_XS)
                                    .text_color(status_err_fg()),
                            )
                    },
                ))
                .child(
                    view()
                        .role(AccessibilityRole::Button)
                        .w(px(ControlSize::Xs.height()))
                        .h(px(ControlSize::Xs.height()))
                        .rounded(px(RADIUS_SM))
                        .flex_shrink_0()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .border_1()
                        .border_color(move || {
                            if !profiles.relay_url.get().trim().is_empty() {
                                accent_blue_border()
                            } else {
                                border_control()
                            }
                        })
                        .bg(move || {
                            if !profiles.relay_url.get().trim().is_empty() {
                                accent_blue_soft()
                            } else {
                                bg_control()
                            }
                        })
                        .hover(|style| {
                            style.bg(move || {
                                if !profiles.relay_url.get().trim().is_empty() {
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
                        .tooltip(label_tip("配置中转服务器", "Configure relay server"))
                        .on_press(move || {
                            open.set(false);
                            open_settings_tab(profiles, SettingsTab::Relay);
                        })
                        .child(glyph_px(
                            12.0,
                            || Glyph::Sliders,
                            move || {
                                if !profiles.relay_url.get().trim().is_empty() {
                                    color(accent_blue_text())
                                } else {
                                    color(fg_secondary())
                                }
                            },
                        )),
                ),
        )
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn selected_browser_start_url(
    controls: &Controls,
    selected_id: Option<u64>,
) -> Option<String> {
    let id = selected_id?;
    controls.hub.with_session(|session| {
        let flow = session.get(id)?;
        if flow.protocol == session::Protocol::Tunnel {
            return None;
        }
        let scheme = if flow.url.starts_with("https://") {
            "https"
        } else if flow.url.starts_with("http://") {
            "http"
        } else {
            return None;
        };
        let host = flow.host.trim();
        if host.is_empty() {
            None
        } else {
            Some(format!("{scheme}://{host}"))
        }
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn menu_quick_launch_row(
    open: Signal<bool>,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
) -> impl IntoElement {
    let browser_controls = controls.clone();
    let term_controls = controls.clone();
    let adb_controls = controls;
    view()
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .child(glyph(|| Glyph::Globe, || color(fg_secondary())))
        .child(
            control_text(tr("免配抓包", "Launch"))
                .flex_1()
                .text_color(fg_secondary()),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(menu_inline_pill(
                    tr("浏览器", "Browser"),
                    || false,
                    move || {
                        open.set(false);
                        let start_url =
                            selected_browser_start_url(&browser_controls, selected.get());
                        launch_browser_action(browser_controls.clone(), start_url);
                    },
                ))
                .child(menu_inline_pill(
                    tr("终端", "Term"),
                    || false,
                    move || {
                        open.set(false);
                        launch_terminal_action(term_controls.clone());
                    },
                ))
                .child(menu_inline_pill(
                    move || {
                        let _ = open.get();
                        let count = launcher::configured_adb_count();
                        if count > 0 {
                            SharedString::from(format!("ADB · {count}"))
                        } else {
                            SharedString::from("ADB")
                        }
                    },
                    move || {
                        let _ = open.get();
                        launcher::configured_adb_count() > 0
                    },
                    move || {
                        open.set(false);
                        toggle_adb_action(adb_controls.clone());
                    },
                )),
        )
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn menu_copy_config_row(
    open: Signal<bool>,
    copy_env: impl Fn() + 'static,
    controls: Arc<Controls>,
) -> impl IntoElement {
    view()
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .child(glyph(|| Glyph::Prompt, || color(fg_secondary())))
        .child(
            control_text(tr("复制配置", "Copy config"))
                .flex_1()
                .text_color(fg_secondary()),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(menu_inline_pill(
                    tr("代理环境", "Proxy env"),
                    || false,
                    move || {
                        open.set(false);
                        copy_env();
                    },
                ))
                .child(menu_inline_pill(
                    "MCP",
                    || false,
                    move || {
                        open.set(false);
                        copy_mcp_config(&controls);
                    },
                )),
        )
}

pub(crate) fn menu_lang_row(controls: Arc<Controls>) -> impl IntoElement {
    let langs = ui_langs();
    let set_lang = move |choice: Option<Lang>, controls: Arc<Controls>| {
        langs.set(choice);
        if let Err(err) = store_lang_pref(choice) {
            set_status(
                &controls,
                StatusMsg::PrefUnsaved {
                    name: PrefName::Language,
                    err: err.to_string(),
                },
            );
        }
    };
    let c_sys = controls.clone();
    let c_zh = controls.clone();
    let c_en = controls;
    view()
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .child(
            control_text(tr("界面语言", "Language"))
                .flex_1()
                .text_color(fg_secondary()),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(menu_inline_pill(
                    tr("系统", "Auto"),
                    move || langs.get().is_none(),
                    move || set_lang(None, c_sys.clone()),
                ))
                .child(menu_inline_pill(
                    "中文",
                    move || langs.get() == Some(Lang::Zh),
                    move || set_lang(Some(Lang::Zh), c_zh.clone()),
                ))
                .child(menu_inline_pill(
                    "EN",
                    move || langs.get() == Some(Lang::En),
                    move || set_lang(Some(Lang::En), c_en.clone()),
                )),
        )
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn cert_menu_row(
    ca_ui: CaSignals,
    install: Rc<impl Fn() + 'static>,
    remove: Rc<impl Fn() + 'static>,
    switch_builtin: impl Fn() + 'static,
) -> impl IntoElement {
    let langs = ui_langs();
    view()
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .flex_row()
        .items_center()
        .child(glyph(|| Glyph::Badge, || color(fg_secondary())))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(
                    view()
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded_full()
                        .flex_shrink_0()
                        .bg(move || match ca_ui.status.get() {
                            ca::CaStatus::Trusted => status_ok_fg(),
                            ca::CaStatus::InstalledUntrusted | ca::CaStatus::NotInstalled => {
                                status_warn_fg()
                            }
                        }),
                )
                .child(
                    control_text(move || -> SharedString {
                        let lang = ui_lang(langs);
                        if let Some(ext) = ca_ui.active_external.get() {
                            return match lang {
                                Lang::Zh => format!("证书：已复用 {}", ext.tool_name).into(),
                                Lang::En => format!("CA: reusing {}", ext.tool_name).into(),
                            };
                        }
                        match ca_ui.status.get() {
                            ca::CaStatus::Trusted => t(lang, "证书：已信任", "CA: trusted").into(),
                            ca::CaStatus::InstalledUntrusted => {
                                t(lang, "证书：未信任", "CA: untrusted").into()
                            }
                            ca::CaStatus::NotInstalled => {
                                t(lang, "证书：未安装", "CA: not installed").into()
                            }
                        }
                    })
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
        .child(cert_menu_row_actions(
            ca_ui,
            install,
            remove,
            switch_builtin,
        ))
}

#[cfg(not(target_arch = "wasm32"))]
fn cert_menu_row_actions(
    ca_ui: CaSignals,
    install: Rc<impl Fn() + 'static>,
    remove: Rc<impl Fn() + 'static>,
    switch_builtin: impl Fn() + 'static,
) -> impl IntoElement {
    let switch_cb = Rc::new(switch_builtin);
    let install_untrusted = install.clone();
    let install_fresh = install;

    when(
        move || ca_ui.active_external.get().is_some(),
        move || {
            let cb = Rc::clone(&switch_cb);
            menu_inline_pill(
                tr("切回 zniff", "Use zniff"),
                || false,
                move || cb.as_ref()(),
            )
        },
    )
    .otherwise(move || {
        let install_untrusted = install_untrusted.clone();
        let install_fresh = install_fresh.clone();
        let remove = remove.clone();
        view()
            .flex_row()
            .items_center()
            .gap_1()
            .child(when(
                move || ca_ui.status.get() == ca::CaStatus::InstalledUntrusted,
                move || {
                    let install = install_untrusted.clone();
                    menu_inline_pill(tr("信任", "Trust"), || true, move || install.as_ref()())
                },
            ))
            .child(
                when(
                    move || ca_ui.status.get().is_installed(),
                    move || {
                        let remove = remove.clone();
                        menu_inline_pill(tr("移除", "Remove"), || false, move || remove.as_ref()())
                    },
                )
                .otherwise(move || {
                    let install = install_fresh.clone();
                    menu_inline_pill(tr("安装", "Install"), || true, move || install.as_ref()())
                }),
            )
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn external_ca_menu_row(
    open: Signal<bool>,
    ca_ui: CaSignals,
    controls: Arc<Controls>,
) -> impl IntoElement {
    when(
        move || ca_ui.detected_external.with(|items| !items.is_empty()),
        move || {
            let mut pills = view().flex_row().flex_wrap().items_center().gap_1();
            for kind in external_ca::ExternalCaKind::ALL {
                pills = pills.child(external_ca_pill(open, ca_ui, controls.clone(), kind));
            }
            view()
                .min_h(px(32.0))
                .px_2p5()
                .py_1()
                .gap_2()
                .flex_row()
                .items_center()
                .child(glyph(|| Glyph::Sliders, || color(fg_secondary())))
                .child(view().flex_1().min_w(px(0.0)).child(
                    control_text(tr("复用本机证书", "Reuse local CA")).text_color(fg_secondary()),
                ))
                .child(pills)
        },
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn external_ca_pill(
    open: Signal<bool>,
    ca_ui: CaSignals,
    controls: Arc<Controls>,
    kind: external_ca::ExternalCaKind,
) -> impl IntoElement {
    let id = kind.id();
    let label = kind.tool_name();
    when(
        move || {
            ca_ui
                .detected_external
                .with(|items| items.iter().any(|item| item.id == id))
        },
        move || {
            let click_controls = controls.clone();
            menu_inline_pill(
                label,
                move || {
                    ca_ui
                        .active_external
                        .with(|cur| cur.as_ref().is_some_and(|item| item.id == id))
                },
                move || {
                    open.set(false);
                    let already_active = ca_ui
                        .active_external
                        .with(|cur| cur.as_ref().is_some_and(|item| item.id == id));
                    if already_active {
                        use_builtin_ca(click_controls.clone());
                    } else {
                        use_external_ca(click_controls.clone(), id.to_string());
                    }
                },
            )
        },
    )
}

pub(crate) fn menu_rule() -> impl IntoElement {
    view().h(px(1.0)).mx(px(6.0)).bg(border_strong())
}

pub(crate) fn primary_button(
    label: impl IntoReactive<SharedString>,
    danger: impl Fn() -> bool + 'static + Clone,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let danger_bg = danger.clone();
    let danger_hover = danger.clone();
    let danger_active = danger.clone();
    let icon_state = danger;
    view()
        .window_drag(false)
        .role(AccessibilityRole::Button)
        .h(px(ControlSize::Sm.height()))
        .px(px(ControlSize::Sm.padding_x()))
        .gap(px(ControlSize::Sm.gap()))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(RADIUS_MD))
        .bg(move || {
            if danger_bg() {
                danger_button_bg()
            } else {
                accent_blue()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if danger_hover() {
                    danger_button_hover()
                } else {
                    accent_blue_hover()
                }
            })
        })
        .active(|style| {
            style.bg(move || {
                if danger_active() {
                    danger_button_active()
                } else {
                    accent_blue_active()
                }
            })
        })
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .on_press(on_press)
        .child(glyph_px(
            ControlSize::Sm.icon_size(),
            move || {
                if icon_state() {
                    Glyph::Stop
                } else {
                    Glyph::Play
                }
            },
            || color(fg_on_accent()),
        ))
        .child(
            control_text(label)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_on_accent()),
        )
}
