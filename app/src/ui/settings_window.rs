//! Dedicated tabbed Settings window (`Cmd/Ctrl+,`): native child window on desktop, modal on wasm32.

#[allow(unused_imports)]
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SettingsTab {
    #[default]
    General,
    Certificate,
    Network,
    Security,
    Relay,
    Updates,
}

impl SettingsTab {
    pub(crate) const ALL: [Self; 6] = [
        Self::General,
        Self::Certificate,
        Self::Network,
        Self::Security,
        Self::Relay,
        Self::Updates,
    ];

    pub(crate) fn label(self) -> (&'static str, &'static str) {
        match self {
            Self::General => ("通用", "General"),
            Self::Certificate => ("证书", "Certificate"),
            Self::Network => ("网络与上游", "Network & Upstream"),
            Self::Security => ("安全", "Security"),
            Self::Relay => ("中继", "Relay"),
            Self::Updates => ("更新", "Updates"),
        }
    }

    pub(crate) fn glyph(self) -> Glyph {
        match self {
            Self::General => Glyph::Sliders,
            Self::Certificate => Glyph::Badge,
            Self::Network => Glyph::Globe,
            Self::Security => Glyph::Shield,
            Self::Relay => Glyph::Prompt,
            Self::Updates => Glyph::Download,
        }
    }
}

pub(crate) fn open_settings_tab(profiles: ProfileSignals, tab: SettingsTab) {
    profiles.settings_tab.set(tab);
    profiles.settings_open.set(true);
}

#[derive(Clone)]
pub(crate) struct SettingsWindowProps {
    pub(crate) profiles: ProfileSignals,
    pub(crate) controls: Arc<Controls>,
    pub(crate) ca_ui: CaSignals,
    pub(crate) running: Signal<bool>,
    pub(crate) listen: Signal<String>,
    pub(crate) proxy_on: Signal<bool>,
    pub(crate) port_text: Signal<String>,
    pub(crate) listen_all: Signal<bool>,
    pub(crate) pretty: Signal<bool>,
    pub(crate) seq_builder: SeqBuilder,
    pub(crate) rows: Signal<Vec<Arc<RowLine>>>,
    pub(crate) compose: Compose,
    pub(crate) selected: Signal<Option<u64>>,
    pub(crate) shown_phase: Signal<Phase>,
    pub(crate) update_state: Signal<UpdateState>,
    pub(crate) install_ca: Rc<dyn Fn()>,
    pub(crate) remove_ca: Rc<dyn Fn()>,
    pub(crate) toggle_proxy: Rc<dyn Fn()>,
}

/// Renders the Settings window as a native desktop window, or as a modal overlay on wasm32.
#[cfg(not(target_arch = "wasm32"))]
fn settings_window_body(props: SettingsWindowProps) -> impl IntoElement {
    let esc_open = props.profiles.settings_open;
    on_key("escape", move |_| esc_open.set(false));
    let mod_open = props.profiles.settings_open;
    on_key_with(
        "mod+,",
        KeyOptions {
            when: nsh_core::keymap::When::Always,
            ..KeyOptions::default()
        },
        move |_| mod_open.set(false),
    );
    view()
        .w_full()
        .h_full()
        .bg(bg_card())
        .child(settings_shell(props, false))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn settings_window(props: SettingsWindowProps) -> impl IntoElement {
    let open = props.profiles.settings_open;
    nsh_core::element::desktop::window()
        .id("settings")
        .kind(nsh_core::desktop::WindowKind::Normal)
        .parent("main")
        .default_size(Size::new(px(740.0), px(540.0)))
        .restore_bounds(true)
        .title(tr("设置 — zniff", "Settings — zniff"))
        .open(move || open.get())
        .on_open_change(move |ev| {
            if !ev.open {
                open.set(false);
            }
        })
        .content(move || settings_window_body(props.clone()))
}

/// Renders the Settings window as a modal overlay in the browser console (`wasm32`).
#[cfg(target_arch = "wasm32")]
pub(crate) fn settings_window(props: SettingsWindowProps) -> impl IntoElement {
    let open = props.profiles.settings_open;
    when(
        move || open.get(),
        move || {
            let props = props.clone();
            let backdrop = ui_modal_backdrop(move || open.set(false));
            let card = ui_modal_card()
                .w(move || px((window().width.0 - 40.0).clamp(520.0, 760.0)))
                .h(move || px((window().height.0 - 64.0).clamp(360.0, 560.0)))
                .child(settings_modal_header(open))
                .child(
                    view()
                        .flex_1()
                        .min_h(px(0.0))
                        .w_full()
                        .child(settings_shell(props, true)),
                );
            overlay()
                .inset_0()
                .on_cancel(move || open.set(false))
                .child(
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

#[cfg(target_arch = "wasm32")]
fn settings_modal_header(open: Signal<bool>) -> impl IntoElement {
    view()
        .w_full()
        .px_4()
        .py_2p5()
        .flex_row()
        .items_center()
        .gap_2()
        .border_b_1()
        .border_b_color(border_subtle())
        .bg(bg_chrome())
        .child(glyph_px(
            14.0,
            || Glyph::Sliders,
            || color(accent_blue_text()),
        ))
        .child(
            text(tr("设置", "Settings"))
                .flex_1()
                .text_size(px(TEXT_LG))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_primary()),
        )
        .child(ui_modal_close_button(move || open.set(false)))
}

pub(crate) fn settings_shell(props: SettingsWindowProps, is_modal: bool) -> impl IntoElement {
    let tab = props.profiles.settings_tab;
    let content_props = props.clone();
    view()
        .w_full()
        .h_full()
        .flex_row()
        .overflow_hidden()
        .child(settings_sidebar(tab))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .flex_col()
                .bg(bg_card())
                .child(settings_tab_header(tab))
                .child(
                    view().flex_1().min_h(px(0.0)).w_full().child(
                        scroll().w_full().h_full().child(
                            view()
                                .w_full()
                                .p_4()
                                .gap_3()
                                .flex_col()
                                .child(dynamic(move || {
                                    let p = content_props.clone();
                                    match tab.get() {
                                        SettingsTab::General => {
                                            Some(settings_general_tab(p).into_element())
                                        }
                                        SettingsTab::Certificate => {
                                            Some(settings_certificate_tab(p).into_element())
                                        }
                                        SettingsTab::Network => {
                                            Some(settings_network_tab(p).into_element())
                                        }
                                        SettingsTab::Security => {
                                            Some(settings_security_tab(p, is_modal).into_element())
                                        }
                                        SettingsTab::Relay => {
                                            Some(settings_relay_tab(p).into_element())
                                        }
                                        SettingsTab::Updates => {
                                            Some(settings_updates_tab(p).into_element())
                                        }
                                    }
                                })),
                        ),
                    ),
                ),
        )
}

fn settings_sidebar(tab: Signal<SettingsTab>) -> impl IntoElement {
    view()
        .w(px(168.0))
        .h_full()
        .flex_shrink_0()
        .p_2p5()
        .gap_1()
        .flex_col()
        .bg(bg_chrome())
        .border_r_1()
        .border_r_color(border_subtle())
        .children(SettingsTab::ALL.into_iter().map(move |item| {
            let (zh, en) = item.label();
            let icon = item.glyph();
            let active = move || tab.get() == item;
            view()
                .role(AccessibilityRole::Tab)
                .w_full()
                .h(px(ControlSize::Md.height()))
                .px_2p5()
                .gap_2()
                .rounded(px(RADIUS_MD))
                .flex_row()
                .items_center()
                .border_1()
                .border_color(move || {
                    if active() {
                        accent_blue_border()
                    } else {
                        AdaptiveColor::TRANSPARENT
                    }
                })
                .bg(move || {
                    if active() {
                        accent_blue_soft()
                    } else {
                        AdaptiveColor::TRANSPARENT
                    }
                })
                .hover(|s| {
                    s.bg(move || {
                        if active() {
                            accent_blue_soft_hover()
                        } else {
                            bg_control_hover()
                        }
                    })
                })
                .active(|s| s.bg(bg_control_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .on_press(move || tab.set(item))
                .child(glyph_px(
                    14.0,
                    move || icon,
                    move || {
                        color(if active() {
                            accent_blue_text()
                        } else {
                            fg_secondary()
                        })
                    },
                ))
                .child(control_text(tr(zh, en)).flex_1().text_color(move || {
                    if active() {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }))
        }))
}

fn settings_tab_header(tab: Signal<SettingsTab>) -> impl IntoElement {
    let langs = ui_langs();
    view()
        .w_full()
        .px_4()
        .py_3()
        .border_b_1()
        .border_b_color(border_subtle())
        .child(
            text(move || -> SharedString {
                let (zh, en) = tab.get().label();
                t(ui_lang(langs), zh, en).into()
            })
            .text_size(px(TEXT_LG))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(fg_primary()),
        )
}

fn settings_general_tab(props: SettingsWindowProps) -> impl IntoElement {
    let controls = props.controls;
    let pretty = props.pretty;
    let profiles = props.profiles;
    #[cfg(not(target_arch = "wasm32"))]
    let proxy_on = props.proxy_on;
    #[cfg(not(target_arch = "wasm32"))]
    let toggle_proxy = props.toggle_proxy;

    let appearance = settings_card(
        "外观与语言",
        "Appearance & Language",
        view()
            .w_full()
            .gap_1()
            .flex_col()
            .child(menu_lang_row(controls.clone()))
            .child(menu_theme_row(controls.clone()))
            .child(menu_zoom_row(controls.clone()))
            .child(menu_mono_font_row(controls.clone())),
    );

    let reading = settings_card(
        "阅读与布局",
        "Reading & Layout",
        view()
            .w_full()
            .gap_1()
            .flex_col()
            .child(settings_toggle_row(
                "默认美化",
                "Pretty by default",
                move || pretty.get(),
                {
                    let c = controls.clone();
                    move || {
                        let next = !pretty.get();
                        pretty.set(next);
                        save_config_or_notify(&c, PrefName::Pretty, |cfg| {
                            cfg.default_pretty = next
                        });
                    }
                },
            ))
            .child(settings_toggle_row(
                "JSON 默认树形",
                "JSON tree by default",
                move || profiles.default_json_tree.get(),
                {
                    let c = controls.clone();
                    move || {
                        let next = !profiles.default_json_tree.get();
                        profiles.default_json_tree.set(next);
                        if next && !pretty.get() {
                            pretty.set(true);
                        }
                        save_config_or_notify(&c, PrefName::Pretty, |cfg| {
                            cfg.default_json_tree = next
                        });
                    }
                },
            ))
            .child(settings_layout_row(profiles.layout_mode, controls.clone())),
    );

    let root = view()
        .w_full()
        .gap_3()
        .flex_col()
        .child(appearance)
        .child(reading);

    let root = desktop_child!(
        root,
        settings_card(
            "系统代理",
            "System Proxy",
            settings_toggle_row(
                "自动接管系统 HTTP(S) 代理",
                "Manage system HTTP(S) proxy automatically",
                move || proxy_on.get(),
                move || (toggle_proxy)(),
            ),
        )
    );

    desktop_child!(
        root,
        settings_card(
            "窗口与托盘",
            "Window & Tray",
            view()
                .w_full()
                .gap_1()
                .child(settings_toggle_row(
                    "启用系统托盘图标",
                    "Show system tray icon",
                    move || profiles.show_tray.get(),
                    {
                        let c = controls.clone();
                        move || {
                            let next = !profiles.show_tray.get();
                            profiles.show_tray.set(next);
                            if !next {
                                profiles.minimize_to_tray.set(false);
                            }
                            save_config_or_notify(&c, PrefName::Window, |cfg| {
                                cfg.show_tray = next;
                                if !next {
                                    cfg.minimize_to_tray = false;
                                }
                            });
                        }
                    },
                ))
                .child(when(move || profiles.show_tray.get(), {
                    let c = controls.clone();
                    move || {
                        let c = c.clone();
                        settings_toggle_row(
                            "关闭窗口时最小化到系统托盘",
                            "Minimize to system tray when closing window",
                            move || profiles.minimize_to_tray.get(),
                            move || {
                                let next = !profiles.minimize_to_tray.get();
                                profiles.minimize_to_tray.set(next);
                                save_config_or_notify(&c, PrefName::Window, |cfg| {
                                    cfg.minimize_to_tray = next;
                                });
                            },
                        )
                    }
                },)),
        )
    )
}

fn settings_layout_row(
    layout_mode: Signal<config::LayoutMode>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let c_split = controls.clone();
    let c_tb = controls;
    view()
        .h(px(ControlSize::Md.height()))
        .px_2p5()
        .gap_2()
        .rounded(px(RADIUS_MD))
        .flex_row()
        .items_center()
        .child(
            control_text(tr("主界面布局", "Layout"))
                .flex_1()
                .text_color(fg_secondary()),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(menu_inline_pill(
                    tr("左右分栏", "Split"),
                    move || layout_mode.get() == config::LayoutMode::Split,
                    move || {
                        layout_mode.set(config::LayoutMode::Split);
                        save_config_or_notify(&c_split, PrefName::List, |cfg| {
                            cfg.layout_mode = config::LayoutMode::Split;
                        });
                    },
                ))
                .child(menu_inline_pill(
                    tr("上下分栏", "Top / Bottom"),
                    move || layout_mode.get() == config::LayoutMode::TopBottom,
                    move || {
                        layout_mode.set(config::LayoutMode::TopBottom);
                        save_config_or_notify(&c_tb, PrefName::List, |cfg| {
                            cfg.layout_mode = config::LayoutMode::TopBottom;
                        });
                    },
                )),
        )
}

fn settings_updates_tab(props: SettingsWindowProps) -> impl IntoElement {
    let profiles = props.profiles;
    let version_card = settings_card(
        "当前版本",
        "Version",
        view()
            .w_full()
            .px_2p5()
            .py_1()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
                mono(format!("zniff v{}", env!("CARGO_PKG_VERSION")))
                    .text_size(px(TEXT_MD))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(fg_primary()),
            )
            .child(menu_inline_pill(
                tr("关于 zniff", "About zniff"),
                || false,
                move || {
                    profiles.about_open.set(true);
                    #[cfg(not(target_arch = "wasm32"))]
                    nsh_core::element::desktop::main_window().focus();
                    #[cfg(target_arch = "wasm32")]
                    profiles.settings_open.set(false);
                },
            )),
    );
    let root = view().w_full().gap_3().flex_col().child(version_card);
    desktop_child!(
        root,
        settings_card(
            "软件更新",
            "Software Update",
            update_menu_section(None, props.update_state, props.controls),
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn settings_tabs_cover_six_unique_sections() {
        assert_eq!(SettingsTab::ALL.len(), 6);
        let zh: HashSet<&str> = SettingsTab::ALL.iter().map(|t| t.label().0).collect();
        let en: HashSet<&str> = SettingsTab::ALL.iter().map(|t| t.label().1).collect();
        assert_eq!(zh.len(), 6);
        assert_eq!(en.len(), 6);
        assert_eq!(SettingsTab::default(), SettingsTab::General);
    }
}
