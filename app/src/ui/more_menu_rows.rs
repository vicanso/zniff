#[allow(unused_imports)]
use super::*;

pub(crate) fn menu_action(
    icon: Glyph,
    label: impl IntoReactive<SharedString>,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    menu_item()
        .on_select(on_select)
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.opacity(0.75))
        .cursor(Cursor::Pointer)
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
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.opacity(0.75))
        .cursor(Cursor::Pointer)
        .child(glyph(move || icon, || color(fg_secondary())))
        .child(control_text(label).flex_1().text_color(fg_secondary()))
        .child(
            mono(meta)
                .tabular_nums()
                .flex_shrink_0()
                .number_of_lines(1)
                .text_size(px(11.0))
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
        .disabled_style(|style| style.opacity(0.45).cursor(Cursor::Default))
        .on_select(move || {
            if enabled_run() {
                on_select();
            }
        })
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
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
        .active(|style| style.opacity(move || if enabled_active() { 0.75_f32 } else { 0.45_f32 }))
        .cursor(Cursor::Pointer)
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
                .text_size(px(11.0))
                .text_color(fg_muted()),
        )
}

pub(crate) fn more_check_action(
    marked: impl Fn() -> bool + 'static + Clone,
    label: impl IntoReactive<SharedString>,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    let dot_border = marked.clone();
    let dot_bg = marked.clone();
    menu_item()
        .on_select(on_select)
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.opacity(0.75))
        .cursor(Cursor::Pointer)
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
        .child(control_text(label).text_color(move || {
            if marked() {
                accent_blue_text()
            } else {
                fg_secondary()
            }
        }))
}

pub(crate) fn menu_inline_pill(
    label: impl IntoReactive<SharedString>,
    active: impl Fn() -> bool + 'static + Copy,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .h(px(22.0))
        .px_2()
        .rounded(px(5.0))
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
        .active(|style| style.opacity(0.75))
        .cursor(Cursor::Pointer)
        .on_press(on_press)
        .child(control_text(label).text_size(px(11.0)).text_color(move || {
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
                .flex_1()
                .h_full()
                .flex_row()
                .items_center()
                .gap_2()
                .rounded(px(5.0))
                .hover(|style| style.bg(bg_control_hover()))
                .active(|style| style.opacity(0.75))
                .cursor(Cursor::Pointer)
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
                                .rounded(px(2.5))
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
                            .h(px(22.0))
                            .px_2()
                            .rounded(px(5.0))
                            .flex_shrink_0()
                            .flex_row()
                            .items_center()
                            .justify_center()
                            .border_1()
                            .border_color(status_err_fg())
                            .bg(status_err_bg())
                            .hover(|style| style.bg(status_err_bg_hover()))
                            .active(|style| style.opacity(0.75))
                            .cursor(Cursor::Pointer)
                            .on_press(move || {
                                open.set(false);
                                end_share(&controls, share_id);
                            })
                            .child(
                                control_text(tr("停止", "Stop"))
                                    .text_size(px(11.0))
                                    .text_color(status_err_fg()),
                            )
                    },
                ))
                .child(
                    view()
                        .w(px(22.0))
                        .h(px(22.0))
                        .rounded(px(5.0))
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
                        .active(|style| style.opacity(0.75))
                        .cursor(Cursor::Pointer)
                        .tooltip(label_tip("配置中转服务器", "Configure relay server"))
                        .on_press(move || {
                            open.set(false);
                            profiles.machine_settings_open.set(true);
                        })
                        .child(glyph_px(
                            11.5,
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

pub(crate) fn proxy_menu_item(on: Signal<bool>, toggle: impl Fn() + 'static) -> impl IntoElement {
    let bg_on = on;
    let hover_on = on;
    let dot_on = on;
    let label_on = on;
    let state_on = on;
    menu_item()
        .on_select(toggle)
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .bg(move || {
            if bg_on.get() {
                accent_blue_soft()
            } else {
                bg_card()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if hover_on.get() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.75))
        .cursor(Cursor::Pointer)
        .child(glyph(
            || Glyph::Shield,
            move || {
                color(if dot_on.get() {
                    accent_blue_text()
                } else {
                    fg_secondary()
                })
            },
        ))
        .child(
            control_text(tr("系统代理", "System proxy"))
                .flex_1()
                .text_color(move || {
                    if label_on.get() {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(move || {
                    if dot_on.get() {
                        accent_blue()
                    } else {
                        fg_muted()
                    }
                }))
                .child(
                    control_text({
                        let langs = ui_langs();
                        move || -> SharedString {
                            if state_on.get() {
                                t(ui_lang(langs), "开", "On").into()
                            } else {
                                t(ui_lang(langs), "关", "Off").into()
                            }
                        }
                    })
                    .text_color(move || {
                        if state_on.get() {
                            accent_blue_text()
                        } else {
                            fg_muted()
                        }
                    }),
                ),
        )
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn cert_menu_row(
    open: Signal<bool>,
    ca_status: Signal<ca::CaStatus>,
    install: Rc<impl Fn() + 'static>,
    remove: Rc<impl Fn() + 'static>,
) -> impl IntoElement {
    let dot_status = ca_status;
    let color_status = ca_status;
    let install_untrusted = install.clone();
    let install_fresh = install;
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
                        .bg(move || match dot_status.get() {
                            ca::CaStatus::Trusted => status_ok_fg(),
                            ca::CaStatus::InstalledUntrusted | ca::CaStatus::NotInstalled => {
                                status_warn_fg()
                            }
                        }),
                )
                .child(
                    control_text({
                        let langs = ui_langs();
                        move || -> SharedString {
                            match ca_status.get() {
                                ca::CaStatus::Trusted => {
                                    t(ui_lang(langs), "证书：已信任", "CA: trusted").into()
                                }
                                ca::CaStatus::InstalledUntrusted => {
                                    t(ui_lang(langs), "证书：未信任", "CA: untrusted").into()
                                }
                                ca::CaStatus::NotInstalled => {
                                    t(ui_lang(langs), "证书：未安装", "CA: not installed").into()
                                }
                            }
                        }
                    })
                    .text_color(move || match color_status.get() {
                        ca::CaStatus::InstalledUntrusted => status_warn_fg(),
                        ca::CaStatus::Trusted | ca::CaStatus::NotInstalled => fg_secondary(),
                    }),
                ),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(when(
                    move || ca_status.get() == ca::CaStatus::InstalledUntrusted,
                    move || {
                        let install = install_untrusted.clone();
                        menu_inline_pill(
                            tr("信任", "Trust"),
                            || true,
                            move || {
                                open.set(false);
                                install.as_ref()();
                            },
                        )
                    },
                ))
                .child(
                    when(
                        move || ca_status.get().is_installed(),
                        move || {
                            let remove = remove.clone();
                            menu_inline_pill(
                                tr("移除", "Remove"),
                                || false,
                                move || {
                                    open.set(false);
                                    remove.as_ref()();
                                },
                            )
                        },
                    )
                    .otherwise(move || {
                        let install = install_fresh.clone();
                        menu_inline_pill(
                            tr("安装", "Install"),
                            || true,
                            move || {
                                open.set(false);
                                install.as_ref()();
                            },
                        )
                    }),
                ),
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
    let icon_state = danger;
    view()
        .window_drag(false)
        .h(px(CONTROL_H))
        .px_3p5()
        .gap_1p5()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .rounded(px(CONTROL_RADIUS))
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
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.8))
        .on_press(on_press)
        .child(glyph(
            move || {
                if icon_state() {
                    Glyph::Stop
                } else {
                    Glyph::Play
                }
            },
            white,
        ))
        .child(
            control_text(label)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(white()),
        )
}
