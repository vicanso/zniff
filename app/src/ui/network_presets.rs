//! Built-in network condition presets (E3), backed by a managed `RuleGroup`
//! using `throttle`, `delay`, and `block` rules.

#[allow(unused_imports)]
use super::*;

/// Name of the managed rule group storing the active network condition preset.
pub(crate) const NETWORK_PRESET_GROUP_NAME: &str = "Network";

/// Built-in network throttling / latency / offline condition presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NetworkPreset {
    None,
    Fast3g,
    Slow3g,
    HighLatency,
    Offline,
}

impl NetworkPreset {
    pub(crate) const ALL: &'static [Self] = &[
        Self::None,
        Self::Fast3g,
        Self::Slow3g,
        Self::HighLatency,
        Self::Offline,
    ];

    pub(crate) fn command_id(self) -> &'static str {
        match self {
            Self::None => "network_none",
            Self::Fast3g => "network_3g",
            Self::Slow3g => "network_slow_3g",
            Self::HighLatency => "network_high_latency",
            Self::Offline => "network_offline",
        }
    }

    pub(crate) fn from_command_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|p| p.command_id() == id)
    }

    pub(crate) fn label(self, lang: Lang) -> &'static str {
        match self {
            Self::None => t(lang, "无限制", "No throttling"),
            Self::Fast3g => "3G",
            Self::Slow3g => t(lang, "弱网 (Slow 3G)", "Slow 3G"),
            Self::HighLatency => t(lang, "高延迟", "High latency"),
            Self::Offline => t(lang, "离线", "Offline"),
        }
    }

    pub(crate) fn badge_label(self, lang: Lang) -> &'static str {
        match self {
            Self::None => "",
            Self::Fast3g => "3G",
            Self::Slow3g => t(lang, "弱网", "Slow 3G"),
            Self::HighLatency => t(lang, "高延迟", "1.5s"),
            Self::Offline => t(lang, "离线", "Offline"),
        }
    }

    pub(crate) fn description(self, lang: Lang) -> &'static str {
        match self {
            Self::None => t(lang, "恢复正常网络速度", "Restore normal network speed"),
            Self::Fast3g => t(
                lang,
                "下行 96 KB/s · 上行 32 KB/s · +150ms",
                "96 KB/s down · 32 KB/s up · +150ms",
            ),
            Self::Slow3g => t(
                lang,
                "下行 50 KB/s · 上行 20 KB/s · +400ms",
                "50 KB/s down · 20 KB/s up · +400ms",
            ),
            Self::HighLatency => t(lang, "全部请求增加 1500ms 延迟", "Add 1500ms delay to all"),
            Self::Offline => t(lang, "断开全部网络连接", "Drop all connections"),
        }
    }

    pub(crate) fn rules_text(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Fast3g => "throttle * 3g\ndelay * 150ms",
            Self::Slow3g => "throttle * slow-3g\ndelay * 400ms",
            Self::HighLatency => "delay * 1500ms",
            Self::Offline => "block * drop",
        }
    }
}

/// Detects which built-in [`NetworkPreset`] is currently active in `groups`.
pub(crate) fn detect_active_network_preset(groups: &[config::RuleGroup]) -> NetworkPreset {
    let Some(group) = groups
        .iter()
        .find(|g| g.name.eq_ignore_ascii_case(NETWORK_PRESET_GROUP_NAME))
    else {
        return NetworkPreset::None;
    };
    if !group.enabled {
        return NetworkPreset::None;
    }
    let normalized = normalize_preset_rules(&group.rules);
    for &preset in NetworkPreset::ALL {
        if preset != NetworkPreset::None
            && normalize_preset_rules(preset.rules_text()) == normalized
        {
            return preset;
        }
    }
    NetworkPreset::None
}

fn normalize_preset_rules(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Returns a new rule-groups list with `preset` applied to the managed `Network` group.
pub(crate) fn apply_network_preset_to_groups(
    groups: &[config::RuleGroup],
    preset: NetworkPreset,
) -> Vec<config::RuleGroup> {
    if preset == NetworkPreset::None {
        let kept: Vec<config::RuleGroup> = groups
            .iter()
            .filter(|g| !g.name.eq_ignore_ascii_case(NETWORK_PRESET_GROUP_NAME))
            .cloned()
            .collect();
        config::sanitize_rule_groups(kept)
    } else {
        config::upsert_rule_group(groups, NETWORK_PRESET_GROUP_NAME, preset.rules_text(), true)
    }
}

/// Applies `preset` to the live `Hub`, reactive UI signals, and persisted `config.toml`.
pub(crate) fn apply_network_preset(
    profiles: ProfileSignals,
    controls: &Controls,
    preset: NetworkPreset,
) {
    let next = apply_network_preset_to_groups(&profiles.rule_groups.get_untracked(), preset);
    if preset == NetworkPreset::None
        && profiles.editing_group.with_untracked(|g| {
            g.as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(NETWORK_PRESET_GROUP_NAME))
        })
    {
        profiles.editing_group.set(None);
    }
    profiles.rule_groups.set(next.clone());
    controls.hub.set_rule_groups(&next);
    if preset != NetworkPreset::None && !profiles.rules_on.get_untracked() {
        profiles.rules_on.set(true);
        controls.hub.set_rules_enabled(true);
    }
    save_config_or_notify(controls, PrefName::Rules, |cfg| {
        cfg.rule_groups = next;
    });
}

/// Toolbar network condition preset switcher popover (`3G` / `Slow 3G` / `High latency` / `Offline`).
pub(crate) fn toolbar_network_preset_popover(
    profiles: ProfileSignals,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let open = signal(false);
    let langs = ui_langs();
    let active_preset = computed(move || {
        if !profiles.rules_on.get() {
            return NetworkPreset::None;
        }
        profiles
            .rule_groups
            .with(|groups| detect_active_network_preset(groups))
    });
    let is_throttled = move || active_preset.get() != NetworkPreset::None;

    popover()
        .open(move || Some(open.get()))
        .on_open_change(move |event: &nsh_base::PopoverEvent| {
            if let nsh_base::PopoverEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .popup_style(ui_popup_style)
        .popup_starting_style(ui_popup_starting_style)
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(8.0))
        .trigger(
            view()
                .window_drag(false)
                .role(AccessibilityRole::Button)
                .h(px(CONTROL_H))
                .px_2()
                .gap_1()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .justify_center()
                .rounded(px(RADIUS_MD))
                .border_1()
                .border_color(move || {
                    if is_throttled() {
                        status_warn_fg()
                    } else if open.get() {
                        accent_blue_border()
                    } else {
                        AdaptiveColor::TRANSPARENT
                    }
                })
                .bg(move || {
                    if is_throttled() {
                        status_warn_bg()
                    } else if open.get() {
                        accent_blue_soft()
                    } else {
                        AdaptiveColor::TRANSPARENT
                    }
                })
                .hover(|style| {
                    style.bg(move || {
                        if is_throttled() {
                            status_warn_bg()
                        } else if open.get() {
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
                .label(tr("网络条件预设", "Network conditions"))
                .tooltip(label_tip(
                    "网络条件预设（3G / 弱网 / 高延迟 / 离线）",
                    "Network conditions (3G / Slow 3G / High latency / Offline)",
                ))
                .child(glyph_px(
                    ControlSize::Sm.icon_size(),
                    || Glyph::Globe,
                    move || {
                        color(if is_throttled() {
                            status_warn_fg()
                        } else if open.get() {
                            accent_blue_text()
                        } else {
                            fg_muted()
                        })
                    },
                ))
                .child(when(is_throttled, move || {
                    text(move || {
                        SharedString::from(active_preset.get().badge_label(ui_lang(langs)))
                    })
                    .number_of_lines(1)
                    .text_size(px(TEXT_SM))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(status_warn_fg())
                })),
        )
        .child(network_preset_menu(
            profiles,
            controls,
            active_preset,
            open,
            langs,
        ))
}

fn network_preset_menu(
    profiles: ProfileSignals,
    controls: Arc<Controls>,
    active_preset: Memo<NetworkPreset>,
    open: Signal<bool>,
    langs: Signal<Option<Lang>>,
) -> impl IntoElement {
    let mut menu = view().w(px(236.0)).p_1p5().gap_0p5().flex_col().child(
        view().px_2().py_1().child(
            text(tr("网络条件预设", "Network Conditions"))
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_muted()),
        ),
    );

    for &preset in NetworkPreset::ALL {
        let item_controls = controls.clone();
        let is_selected = move || active_preset.get() == preset;
        menu = menu.child(
            view()
                .w_full()
                .px_2()
                .py_1p5()
                .rounded(px(RADIUS_MD))
                .flex_row()
                .items_center()
                .gap_2()
                .bg(move || {
                    if is_selected() {
                        accent_blue_soft()
                    } else {
                        bg_card()
                    }
                })
                .hover(|style| {
                    style.bg(move || {
                        if is_selected() {
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
                .on_press(move || {
                    apply_network_preset(profiles, &item_controls, preset);
                    open.set(false);
                })
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex_col()
                        .gap_0p5()
                        .child(
                            text(move || SharedString::from(preset.label(ui_lang(langs))))
                                .number_of_lines(1)
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || {
                                    if is_selected() {
                                        accent_blue_text()
                                    } else {
                                        fg_primary()
                                    }
                                }),
                        )
                        .child(
                            text(move || SharedString::from(preset.description(ui_lang(langs))))
                                .number_of_lines(1)
                                .text_size(px(TEXT_XS))
                                .text_color(fg_muted()),
                        ),
                )
                .child(when(is_selected, move || {
                    glyph_px(
                        ControlSize::Xs.icon_size(),
                        || Glyph::Check,
                        || color(accent_blue_text()),
                    )
                })),
        );
    }

    menu
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_network_presets_parse_cleanly_into_expected_rules() {
        for &preset in NetworkPreset::ALL {
            let text = preset.rules_text();
            let invalid = crate::rules::invalid_rule_lines(text);
            assert!(
                invalid.is_empty(),
                "preset {:?} had invalid rule lines {:?}",
                preset,
                invalid
            );
            let parsed = crate::rules::parse_rules(text);
            match preset {
                NetworkPreset::None => assert!(parsed.is_empty()),
                NetworkPreset::Fast3g | NetworkPreset::Slow3g => {
                    assert_eq!(parsed.len(), 2);
                    assert!(matches!(parsed[0], crate::rules::Rule::Throttle { .. }));
                    assert!(matches!(parsed[1], crate::rules::Rule::Delay { .. }));
                }
                NetworkPreset::HighLatency => {
                    assert_eq!(parsed.len(), 1);
                    assert!(matches!(parsed[0], crate::rules::Rule::Delay { .. }));
                }
                NetworkPreset::Offline => {
                    assert_eq!(parsed.len(), 1);
                    assert!(matches!(
                        parsed[0],
                        crate::rules::Rule::Block {
                            drop_conn: true,
                            ..
                        }
                    ));
                }
            }
        }
    }

    #[test]
    fn apply_and_detect_network_presets_roundtrip_and_preserve_user_groups() {
        let user_group = config::RuleGroup {
            name: "Dev API".into(),
            enabled: true,
            rules: "cors *".into(),
        };
        let initial = vec![user_group.clone()];
        assert_eq!(detect_active_network_preset(&initial), NetworkPreset::None);

        for &preset in &[
            NetworkPreset::Fast3g,
            NetworkPreset::Slow3g,
            NetworkPreset::HighLatency,
            NetworkPreset::Offline,
        ] {
            let updated = apply_network_preset_to_groups(&initial, preset);
            assert_eq!(detect_active_network_preset(&updated), preset);
            assert!(
                updated
                    .iter()
                    .any(|g| g.name == "Dev API" && g.rules == "cors *"),
                "unrelated user group must be preserved"
            );
            let cleared = apply_network_preset_to_groups(&updated, NetworkPreset::None);
            assert_eq!(detect_active_network_preset(&cleared), NetworkPreset::None);
            assert_eq!(cleared, initial);
        }
    }

    #[test]
    fn disabled_network_group_reports_none() {
        let groups = vec![config::RuleGroup {
            name: NETWORK_PRESET_GROUP_NAME.into(),
            enabled: false,
            rules: NetworkPreset::Slow3g.rules_text().into(),
        }];
        assert_eq!(detect_active_network_preset(&groups), NetworkPreset::None);
    }

    #[test]
    fn command_ids_roundtrip_for_every_preset() {
        for &preset in NetworkPreset::ALL {
            assert_eq!(
                NetworkPreset::from_command_id(preset.command_id()),
                Some(preset)
            );
        }
        assert_eq!(NetworkPreset::from_command_id("unknown"), None);
    }
}
