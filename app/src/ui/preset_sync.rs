//! Preset unsaved-state tracking and quick-save footer bar shared across Filter and Rules popovers.

#[allow(unused_imports)]
use super::*;

pub(crate) const PRESET_SYNC_BAR_HEIGHT: f32 = 42.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PresetSaveState {
    Clean,
    UpdateExisting(String),
    SaveNamed(String),
    SaveAsNew,
}

impl PresetSaveState {
    pub(crate) fn is_dirty(&self) -> bool {
        matches!(self, Self::UpdateExisting(_) | Self::SaveNamed(_))
    }

    pub(crate) fn show_bar(&self) -> bool {
        !matches!(self, Self::Clean)
    }
}

pub(crate) struct PresetSnapshotRef<'a> {
    pub(crate) saved: &'a [config::SavedProfile],
    pub(crate) typed_name: &'a str,
    pub(crate) filter: &'a str,
    pub(crate) bypass: &'a str,
    pub(crate) rules: &'a str,
    pub(crate) upstream: &'a str,
    pub(crate) rules_enabled: bool,
    pub(crate) breaks_enabled: bool,
    pub(crate) show_tunnel_failures: bool,
}

/// Classifies the live setup relative to saved presets to drive top-bar badges and quick-save actions.
pub(crate) fn preset_save_state(snap: &PresetSnapshotRef<'_>) -> PresetSaveState {
    let filter = snap.filter.trim();
    let bypass = snap.bypass.trim();
    let rules = snap.rules.trim();
    let upstream = snap.upstream.trim();
    if filter.is_empty() && bypass.is_empty() && rules.is_empty() && upstream.is_empty() {
        return PresetSaveState::Clean;
    }
    let typed = snap.typed_name.trim();
    if !typed.is_empty() {
        let Some(preset) = snap
            .saved
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(typed))
        else {
            return PresetSaveState::SaveNamed(typed.to_string());
        };
        let differs = preset.name != typed
            || preset.filter.trim() != filter
            || preset.bypass_hosts.trim() != bypass
            || preset.rules.trim() != rules
            || preset.upstream_proxy.trim() != upstream
            || preset.rules_enabled != snap.rules_enabled
            || preset.breaks_enabled != snap.breaks_enabled
            || preset.show_tunnel_failures != snap.show_tunnel_failures;
        return if differs {
            PresetSaveState::UpdateExisting(typed.to_string())
        } else {
            PresetSaveState::Clean
        };
    }
    let Some(preset) = snap.saved.iter().find(|preset| {
        preset.filter.trim() == filter
            && preset.bypass_hosts.trim() == bypass
            && preset.rules.trim() == rules
            && preset.upstream_proxy.trim() == upstream
    }) else {
        return PresetSaveState::SaveAsNew;
    };
    if preset.rules_enabled != snap.rules_enabled
        || preset.breaks_enabled != snap.breaks_enabled
        || preset.show_tunnel_failures != snap.show_tunnel_failures
    {
        PresetSaveState::UpdateExisting(preset.name.clone())
    } else {
        PresetSaveState::Clean
    }
}

/// Evaluates `PresetSaveState` against live workspace signals including any filled quick-rule draft.
pub(crate) fn live_preset_save_state(
    profiles: ProfileSignals,
    draft: QuickRuleDraftSignals,
) -> PresetSaveState {
    let kind = draft.kind.get();
    let effective_rules = draft_rules_with_op(
        &profiles.rules.get(),
        &draft.host.get(),
        &draft.path.get(),
        &draft.op.get(),
        &draft.from.get(),
        &draft.to.get(),
        draft.modes.mode_for(kind),
        kind,
    );
    let typed_name = profiles.name.get();
    let filter = profiles.filter.get();
    let bypass = profiles.bypass.get();
    let upstream = profiles.upstream.get();
    let rules_enabled = profiles.rules_on.get();
    let breaks_enabled = profiles.breaks_on.get();
    let show_tunnel_failures = profiles.show_tunnels.get();
    profiles.saved.with(|saved| {
        preset_save_state(&PresetSnapshotRef {
            saved,
            typed_name: &typed_name,
            filter: &filter,
            bypass: &bypass,
            rules: &effective_rules,
            upstream: &upstream,
            rules_enabled,
            breaks_enabled,
            show_tunnel_failures,
        })
    })
}

pub(crate) fn sync_status_text(state: &PresetSaveState, lang: Lang) -> String {
    match (state, lang) {
        (PresetSaveState::UpdateExisting(name), Lang::Zh) => {
            format!("预设「{name}」有未保存修改")
        }
        (PresetSaveState::UpdateExisting(name), Lang::En) => {
            format!("Preset \"{name}\" has unsaved changes")
        }
        (PresetSaveState::SaveNamed(name), Lang::Zh) => {
            format!("预设「{name}」尚未保存")
        }
        (PresetSaveState::SaveNamed(name), Lang::En) => {
            format!("Preset \"{name}\" not saved yet")
        }
        (PresetSaveState::SaveAsNew, Lang::Zh) => "当前配置已生效，可存为预设方便复用".to_string(),
        (PresetSaveState::SaveAsNew, Lang::En) => {
            "Active now — save as a preset to reuse".to_string()
        }
        (PresetSaveState::Clean, _) => String::new(),
    }
}

pub(crate) fn sync_button_label(state: &PresetSaveState, lang: Lang) -> &'static str {
    match (state, lang) {
        (PresetSaveState::UpdateExisting(_), Lang::Zh) => "更新预设",
        (PresetSaveState::UpdateExisting(_), Lang::En) => "Update preset",
        (PresetSaveState::SaveNamed(_), Lang::Zh) => "保存预设",
        (PresetSaveState::SaveNamed(_), Lang::En) => "Save preset",
        (PresetSaveState::SaveAsNew, Lang::Zh) => "存为预设…",
        (PresetSaveState::SaveAsNew, Lang::En) => "Save as preset…",
        (PresetSaveState::Clean, _) => "",
    }
}

/// Flushes any filled quick-rule draft into workspace rules and stores the named preset snapshot.
pub(crate) fn commit_and_save_profile(
    profiles: ProfileSignals,
    draft: QuickRuleDraftSignals,
    target_name: Option<&str>,
    controls: &Controls,
) {
    let host = draft.host.get();
    let path = draft.path.get();
    let op = draft.op.get();
    let from = draft.from.get();
    let to = draft.to.get();
    let kind_now = draft.kind.get();
    let current = profiles.rules.get();
    let rules_now = draft_rules_with_op(
        &current,
        &host,
        &path,
        &op,
        &from,
        &to,
        draft.modes.mode_for(kind_now),
        kind_now,
    );
    if rules_now != current {
        profiles.rules.set(rules_now.clone());
        controls.hub.set_rules(&rules_now);
        draft.host.set(String::new());
        draft.path.set(String::new());
        draft.op.set(String::new());
        draft.from.set(String::new());
        draft.to.set(String::new());
    }
    let name_seed = target_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| profiles.name.get());
    let saved_name =
        resolve_profile_name(&name_seed, profiles.filter.get().trim(), rules_now.trim());
    profiles.name.set(saved_name.clone());
    match store_saved_profile(
        &saved_name,
        &profiles.filter.get(),
        &profiles.bypass.get(),
        &rules_now,
        &profiles.upstream.get(),
        profiles.rules_on.get(),
        profiles.breaks_on.get(),
        profiles.show_tunnels.get(),
        profiles.saved,
        profiles.history,
    ) {
        Ok(()) => set_status(controls, StatusMsg::ProfileSaved(saved_name)),
        Err(err) => set_status(
            controls,
            StatusMsg::PrefUnsaved {
                name: PrefName::Profiles,
                err: err.to_string(),
            },
        ),
    }
}

/// Bottom footer bar rendered inside Filter and Rules popovers when the active setup can be saved or updated.
pub(crate) fn preset_sync_bar(
    profiles: ProfileSignals,
    draft: QuickRuleDraftSignals,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let langs = ui_langs();
    let state = computed(move || live_preset_save_state(profiles, draft));
    when(
        move || state.with(PresetSaveState::show_bar),
        move || {
            let save_controls = controls.clone();
            view()
                .w_full()
                .h(px(40.0))
                .px_3()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .border_t_1()
                .border_t_color(border_subtle())
                .bg(bg_canvas())
                .child(preset_sync_status(state, langs))
                .child(preset_sync_action_button(
                    state,
                    profiles,
                    draft,
                    save_controls,
                    langs,
                ))
        },
    )
}

fn preset_sync_status(
    state: Memo<PresetSaveState>,
    langs: Signal<Option<Lang>>,
) -> impl IntoElement {
    let is_dirty = move || state.with(PresetSaveState::is_dirty);
    view()
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .gap_1p5()
        .overflow_hidden()
        .tooltip(
            tooltip().child(
                view()
                    .max_w(px(360.0))
                    .px_2p5()
                    .py_1p5()
                    .rounded(px(6.0))
                    .bg(bg_card())
                    .border_1()
                    .border_color(border_strong())
                    .child(
                        text(move || -> SharedString {
                            state.with(|s| sync_status_text(s, ui_lang(langs))).into()
                        })
                        .text_size(px(11.5))
                        .text_color(fg_primary()),
                    ),
            ),
        )
        .child(when(is_dirty, move || {
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .flex_shrink_0()
                .bg(status_warn_fg())
        }))
        .child(
            text(move || -> SharedString {
                state.with(|s| sync_status_text(s, ui_lang(langs))).into()
            })
            .flex_1()
            .min_w(px(0.0))
            .number_of_lines(1)
            .text_overflow(EllipsizeMode::Tail)
            .text_size(px(11.5))
            .text_color(move || {
                if is_dirty() {
                    fg_secondary()
                } else {
                    fg_muted()
                }
            }),
        )
}

fn preset_sync_action_button(
    state: Memo<PresetSaveState>,
    profiles: ProfileSignals,
    draft: QuickRuleDraftSignals,
    controls: Arc<Controls>,
    langs: Signal<Option<Lang>>,
) -> impl IntoElement {
    let is_dirty = move || state.with(PresetSaveState::is_dirty);
    view()
        .h(px(26.0))
        .px_2p5()
        .flex_shrink_0()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || {
            if is_dirty() {
                accent_blue()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if is_dirty() {
                accent_blue()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if is_dirty() {
                    accent_blue_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(move || if is_dirty() { 0.85_f32 } else { 0.8_f32 }))
        .flex_row()
        .items_center()
        .justify_center()
        .cursor(Cursor::Pointer)
        .focus_visible(apply_focus_ring)
        .label(move || -> SharedString {
            state.with(|s| sync_button_label(s, ui_lang(langs))).into()
        })
        .on_press(move || match state.get() {
            PresetSaveState::UpdateExisting(name) | PresetSaveState::SaveNamed(name) => {
                commit_and_save_profile(profiles, draft, Some(&name), &controls);
            }
            PresetSaveState::SaveAsNew => {
                profiles.filter_panel_open.set(false);
                profiles.panel_open.set(false);
                profiles.preset_panel_open.set(true);
            }
            PresetSaveState::Clean => {}
        })
        .child(
            text(move || -> SharedString {
                state.with(|s| sync_button_label(s, ui_lang(langs))).into()
            })
            .text_size(px(11.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || {
                if is_dirty() {
                    AdaptiveColor::rgb(0xffffff, 0xffffff)
                } else {
                    fg_primary()
                }
            }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_profile(name: &str) -> config::SavedProfile {
        config::SavedProfile {
            name: name.to_string(),
            filter: "host:api.example.com".to_string(),
            bypass_hosts: String::new(),
            rules: "break api.example.com/v1".to_string(),
            upstream_proxy: String::new(),
            rules_enabled: true,
            breaks_enabled: true,
            show_tunnel_failures: false,
        }
    }

    #[test]
    fn preset_save_state_covers_clean_dirty_named_and_unnamed_cases() {
        let saved = vec![sample_profile("test")];

        // All empty -> Clean even if typed name lingers.
        assert_eq!(
            preset_save_state(&PresetSnapshotRef {
                saved: &saved,
                typed_name: "test",
                filter: "  ",
                bypass: "",
                rules: "",
                upstream: "",
                rules_enabled: true,
                breaks_enabled: true,
                show_tunnel_failures: false,
            }),
            PresetSaveState::Clean
        );

        // Exact match with saved preset -> Clean.
        assert_eq!(
            preset_save_state(&PresetSnapshotRef {
                saved: &saved,
                typed_name: "test",
                filter: "host:api.example.com",
                bypass: "",
                rules: "break api.example.com/v1",
                upstream: "",
                rules_enabled: true,
                breaks_enabled: true,
                show_tunnel_failures: false,
            }),
            PresetSaveState::Clean
        );

        // Modified filter, rules, bypass, upstream, switch, or casing -> UpdateExisting.
        for (name, filter, bypass, rules, upstream, rules_en) in [
            (
                "test",
                "host:api.example.com path:/v2",
                "",
                "break api.example.com/v1",
                "",
                true,
            ),
            (
                "test",
                "host:api.example.com",
                "cdn.example.com",
                "break api.example.com/v1",
                "",
                true,
            ),
            (
                "test",
                "host:api.example.com",
                "",
                "break api.example.com/v2",
                "",
                true,
            ),
            (
                "test",
                "host:api.example.com",
                "",
                "break api.example.com/v1",
                "http://127.0.0.1:7890",
                true,
            ),
            (
                "test",
                "host:api.example.com",
                "",
                "break api.example.com/v1",
                "",
                false,
            ),
            (
                "Test",
                "host:api.example.com",
                "",
                "break api.example.com/v1",
                "",
                true,
            ),
        ] {
            assert_eq!(
                preset_save_state(&PresetSnapshotRef {
                    saved: &saved,
                    typed_name: name,
                    filter,
                    bypass,
                    rules,
                    upstream,
                    rules_enabled: rules_en,
                    breaks_enabled: true,
                    show_tunnel_failures: false,
                }),
                PresetSaveState::UpdateExisting(name.to_string())
            );
        }

        // New typed name not in saved -> SaveNamed.
        assert_eq!(
            preset_save_state(&PresetSnapshotRef {
                saved: &saved,
                typed_name: "staging",
                filter: "host:staging.example.com",
                bypass: "",
                rules: "",
                upstream: "",
                rules_enabled: true,
                breaks_enabled: true,
                show_tunnel_failures: false,
            }),
            PresetSaveState::SaveNamed("staging".to_string())
        );

        // Empty typed name matching saved text with identical switches -> Clean; modified switch -> UpdateExisting.
        assert_eq!(
            preset_save_state(&PresetSnapshotRef {
                saved: &saved,
                typed_name: "",
                filter: "host:api.example.com",
                bypass: "",
                rules: "break api.example.com/v1",
                upstream: "",
                rules_enabled: true,
                breaks_enabled: true,
                show_tunnel_failures: false,
            }),
            PresetSaveState::Clean
        );
        assert_eq!(
            preset_save_state(&PresetSnapshotRef {
                saved: &saved,
                typed_name: "",
                filter: "host:api.example.com",
                bypass: "",
                rules: "break api.example.com/v1",
                upstream: "",
                rules_enabled: false,
                breaks_enabled: true,
                show_tunnel_failures: false,
            }),
            PresetSaveState::UpdateExisting("test".to_string())
        );

        // Empty typed name with novel content -> SaveAsNew.
        assert_eq!(
            preset_save_state(&PresetSnapshotRef {
                saved: &saved,
                typed_name: "",
                filter: "type:json",
                bypass: "",
                rules: "",
                upstream: "",
                rules_enabled: true,
                breaks_enabled: true,
                show_tunnel_failures: false,
            }),
            PresetSaveState::SaveAsNew
        );
    }

    #[test]
    fn sync_bar_labels_and_flags_reflect_state_and_language() {
        let dirty = PresetSaveState::UpdateExisting("test".into());
        assert!(dirty.is_dirty() && dirty.show_bar());
        assert_eq!(
            sync_status_text(&dirty, Lang::Zh),
            "预设「test」有未保存修改"
        );
        assert_eq!(
            sync_status_text(&dirty, Lang::En),
            "Preset \"test\" has unsaved changes"
        );
        assert_eq!(sync_button_label(&dirty, Lang::Zh), "更新预设");
        assert_eq!(sync_button_label(&dirty, Lang::En), "Update preset");

        let named = PresetSaveState::SaveNamed("prod".into());
        assert!(named.is_dirty() && named.show_bar());
        assert_eq!(sync_status_text(&named, Lang::Zh), "预设「prod」尚未保存");
        assert_eq!(sync_button_label(&named, Lang::Zh), "保存预设");

        let save_new = PresetSaveState::SaveAsNew;
        assert!(!save_new.is_dirty() && save_new.show_bar());
        assert_eq!(sync_button_label(&save_new, Lang::Zh), "存为预设…");
        assert_eq!(sync_button_label(&save_new, Lang::En), "Save as preset…");

        assert!(!PresetSaveState::Clean.is_dirty() && !PresetSaveState::Clean.show_bar());
    }
}
