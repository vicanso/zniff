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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PresetChangeField {
    Name,
    Filter,
    Rules { incl_draft: bool },
    Bypass,
    Upstream,
    RulesToggle,
    BreaksToggle,
    TunnelFailures,
}

impl PresetChangeField {
    pub(crate) fn label(self, lang: Lang) -> &'static str {
        match self {
            Self::Name => t(lang, "名称", "Name"),
            Self::Filter => t(lang, "过滤", "Filter"),
            Self::Rules { incl_draft: false } => t(lang, "规则", "Rules"),
            Self::Rules { incl_draft: true } => t(lang, "规则（含草稿）", "Rules (incl. draft)"),
            Self::Bypass => t(lang, "直通", "Bypass"),
            Self::Upstream => t(lang, "上游", "Upstream"),
            Self::RulesToggle => t(lang, "规则开关", "Rules toggle"),
            Self::BreaksToggle => t(lang, "断点开关", "Breaks toggle"),
            Self::TunnelFailures => t(lang, "隧道失败", "Tunnel failures"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PresetChange {
    pub(crate) field: PresetChangeField,
    pub(crate) detail_zh: String,
    pub(crate) detail_en: String,
}

fn find_matched_preset<'a>(snap: &'a PresetSnapshotRef<'_>) -> Option<&'a config::SavedProfile> {
    let filter = snap.filter.trim();
    let bypass = snap.bypass.trim();
    let rules = snap.rules.trim();
    let upstream = snap.upstream.trim();
    if filter.is_empty() && bypass.is_empty() && rules.is_empty() && upstream.is_empty() {
        return None;
    }
    let typed = snap.typed_name.trim();
    if !typed.is_empty() {
        return snap
            .saved
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(typed));
    }
    snap.saved.iter().find(|preset| {
        preset.filter.trim() == filter
            && preset.bypass_hosts.trim() == bypass
            && preset.rules.trim() == rules
            && preset.upstream_proxy.trim() == upstream
    })
}

fn clip_preview(raw: &str, max_chars: usize) -> String {
    let clean = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = clean.chars();
    let clipped: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{clipped}…")
    } else {
        clipped
    }
}

fn fmt_val(raw: &str, lang: Lang) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        t(lang, "空", "empty").to_string()
    } else {
        clip_preview(trimmed, 48)
    }
}

fn push_text_change(out: &mut Vec<PresetChange>, field: PresetChangeField, old: &str, new: &str) {
    let (old_t, new_t) = (old.trim(), new.trim());
    if old_t == new_t {
        return;
    }
    out.push(PresetChange {
        field,
        detail_zh: format!(
            "{} → {}",
            fmt_val(old_t, Lang::Zh),
            fmt_val(new_t, Lang::Zh)
        ),
        detail_en: format!(
            "{} → {}",
            fmt_val(old_t, Lang::En),
            fmt_val(new_t, Lang::En)
        ),
    });
}

fn push_toggle_change(out: &mut Vec<PresetChange>, field: PresetChangeField, old: bool, new: bool) {
    if old == new {
        return;
    }
    let zh = |v| if v { "开" } else { "关" };
    let en = |v| if v { "On" } else { "Off" };
    out.push(PresetChange {
        field,
        detail_zh: format!("{} → {}", zh(old), zh(new)),
        detail_en: format!("{} → {}", en(old), en(new)),
    });
}

fn push_rules_change(
    out: &mut Vec<PresetChange>,
    old_rules: &str,
    new_rules: &str,
    has_rule_draft: bool,
) {
    if old_rules.trim() == new_rules.trim() {
        return;
    }
    let old_lines: Vec<&str> = old_rules
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let new_lines: Vec<&str> = new_rules
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let mut unmatched_old = vec![true; old_lines.len()];
    let mut added = Vec::new();
    for &line in &new_lines {
        if let Some(pos) = old_lines
            .iter()
            .enumerate()
            .position(|(i, old)| unmatched_old[i] && *old == line)
        {
            unmatched_old[pos] = false;
        } else {
            added.push(line);
        }
    }
    let removed: Vec<&str> = old_lines
        .iter()
        .enumerate()
        .filter_map(|(i, line)| unmatched_old[i].then_some(*line))
        .collect();
    let (summary_zh, summary_en) = match (added.len(), removed.len()) {
        (0, 0) => (
            format!("{} → {} 行（顺序调整）", old_lines.len(), new_lines.len()),
            format!(
                "{} → {} lines (reordered)",
                old_lines.len(),
                new_lines.len()
            ),
        ),
        (a, 0) => (
            format!("+{a} 行"),
            format!("+{a} {}", if a == 1 { "line" } else { "lines" }),
        ),
        (0, r) => (
            format!("-{r} 行"),
            format!("-{r} {}", if r == 1 { "line" } else { "lines" }),
        ),
        (a, r) => (format!("+{a} / -{r} 行"), format!("+{a} / -{r} lines")),
    };
    let mut previews = Vec::new();
    for line in added.iter().take(3) {
        previews.push(format!("+ {}", clip_preview(line, 48)));
    }
    for line in removed.iter().take(3_usize.saturating_sub(previews.len())) {
        previews.push(format!("- {}", clip_preview(line, 48)));
    }
    let suffix = if previews.is_empty() {
        String::new()
    } else {
        format!(" ({})", previews.join("; "))
    };
    out.push(PresetChange {
        field: PresetChangeField::Rules {
            incl_draft: has_rule_draft,
        },
        detail_zh: format!("{summary_zh}{suffix}"),
        detail_en: format!("{summary_en}{suffix}"),
    });
}

/// Lists every field where the live setup diverges from the matched saved preset.
pub(crate) fn preset_unsaved_changes(
    snap: &PresetSnapshotRef<'_>,
    has_rule_draft: bool,
) -> Vec<PresetChange> {
    let Some(preset) = find_matched_preset(snap) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let typed = snap.typed_name.trim();
    if !typed.is_empty() {
        push_text_change(&mut out, PresetChangeField::Name, &preset.name, typed);
    }
    push_text_change(
        &mut out,
        PresetChangeField::Filter,
        &preset.filter,
        snap.filter,
    );
    push_rules_change(&mut out, &preset.rules, snap.rules, has_rule_draft);
    push_text_change(
        &mut out,
        PresetChangeField::Bypass,
        &preset.bypass_hosts,
        snap.bypass,
    );
    push_text_change(
        &mut out,
        PresetChangeField::Upstream,
        &preset.upstream_proxy,
        snap.upstream,
    );
    push_toggle_change(
        &mut out,
        PresetChangeField::RulesToggle,
        preset.rules_enabled,
        snap.rules_enabled,
    );
    push_toggle_change(
        &mut out,
        PresetChangeField::BreaksToggle,
        preset.breaks_enabled,
        snap.breaks_enabled,
    );
    push_toggle_change(
        &mut out,
        PresetChangeField::TunnelFailures,
        preset.show_tunnel_failures,
        snap.show_tunnel_failures,
    );
    out
}

fn effective_live_rules(profiles: ProfileSignals, draft: QuickRuleDraftSignals) -> (String, bool) {
    let raw_rules = profiles.rules.get();
    if profiles.editing_group.with(|g| g.is_some()) {
        return (raw_rules, false);
    }
    let kind = draft.kind.get();
    let effective = draft_rules_with_op(
        &raw_rules,
        &draft.host.get(),
        &draft.path.get(),
        &draft.op.get(),
        &draft.from.get(),
        &draft.to.get(),
        draft.modes.mode_for(kind),
        kind,
    );
    let has_rule_draft = effective.trim() != raw_rules.trim();
    (effective, has_rule_draft)
}

/// Evaluates `PresetSaveState` against live workspace signals including any filled quick-rule draft.
pub(crate) fn live_preset_save_state(
    profiles: ProfileSignals,
    draft: QuickRuleDraftSignals,
) -> PresetSaveState {
    let (effective_rules, _) = effective_live_rules(profiles, draft);
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

/// Evaluates detailed field diffs between live workspace signals and the matched saved preset.
pub(crate) fn live_preset_unsaved_changes(
    profiles: ProfileSignals,
    draft: QuickRuleDraftSignals,
) -> Vec<PresetChange> {
    let (effective_rules, has_rule_draft) = effective_live_rules(profiles, draft);
    let typed_name = profiles.name.get();
    let filter = profiles.filter.get();
    let bypass = profiles.bypass.get();
    let upstream = profiles.upstream.get();
    let rules_enabled = profiles.rules_on.get();
    let breaks_enabled = profiles.breaks_on.get();
    let show_tunnel_failures = profiles.show_tunnels.get();
    profiles.saved.with(|saved| {
        preset_unsaved_changes(
            &PresetSnapshotRef {
                saved,
                typed_name: &typed_name,
                filter: &filter,
                bypass: &bypass,
                rules: &effective_rules,
                upstream: &upstream,
                rules_enabled,
                breaks_enabled,
                show_tunnel_failures,
            },
            has_rule_draft,
        )
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

pub(crate) fn sync_status_text_with_changes(
    state: &PresetSaveState,
    changes: &[PresetChange],
    lang: Lang,
) -> String {
    if let PresetSaveState::UpdateExisting(name) = state
        && !changes.is_empty()
    {
        let labels: Vec<&str> = changes.iter().map(|c| c.field.label(lang)).collect();
        return match lang {
            Lang::Zh => format!("预设「{name}」未保存：{}", labels.join("、")),
            Lang::En => format!("Preset \"{name}\" unsaved: {}", labels.join(", ")),
        };
    }
    sync_status_text(state, lang)
}

pub(crate) fn sync_tooltip_text(
    state: &PresetSaveState,
    changes: &[PresetChange],
    lang: Lang,
) -> String {
    if let PresetSaveState::UpdateExisting(name) = state
        && !changes.is_empty()
    {
        let header = match lang {
            Lang::Zh => format!("预设「{name}」有 {} 项未保存修改：", changes.len()),
            Lang::En => format!(
                "Preset \"{name}\" has {} unsaved {}:",
                changes.len(),
                if changes.len() == 1 {
                    "change"
                } else {
                    "changes"
                }
            ),
        };
        let mut lines = Vec::with_capacity(changes.len() + 1);
        lines.push(header);
        for change in changes {
            let label = change.field.label(lang);
            let line = match lang {
                Lang::Zh => format!("• {label}：{}", change.detail_zh),
                Lang::En => format!("• {label}: {}", change.detail_en),
            };
            lines.push(line);
        }
        return lines.join("\n");
    }
    sync_status_text(state, lang)
}

/// Discards all unsaved workspace edits and quick-rule draft back to the saved `target_name` preset.
pub(crate) fn discard_preset_changes(
    profiles: ProfileSignals,
    draft: QuickRuleDraftSignals,
    target_name: &str,
    controls: &Controls,
) -> bool {
    let found = profiles.saved.with_untracked(|saved| {
        saved
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(target_name.trim()))
            .cloned()
    });
    let Some(preset) = found else {
        return false;
    };
    profiles.name.set(preset.name.clone());
    profiles.editing_group.set(None);
    profiles.focused_rule_line.set(None);
    clear_quick_draft(draft);
    if apply_saved_profile(
        &preset,
        profiles.filter,
        profiles.bypass,
        profiles.rules,
        profiles.upstream,
        profiles.rules_on,
        profiles.breaks_on,
        profiles.show_tunnels,
        controls,
    ) {
        set_status(controls, StatusMsg::ProfileReverted(preset.name));
    }
    true
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
    let current = profiles.rules.get();
    let rules_now = if profiles.editing_group.with_untracked(|g| g.is_some()) {
        current.clone()
    } else {
        let host = draft.host.get();
        let path = draft.path.get();
        let op = draft.op.get();
        let from = draft.from.get();
        let to = draft.to.get();
        let kind_now = draft.kind.get();
        draft_rules_with_op(
            &current,
            &host,
            &path,
            &op,
            &from,
            &to,
            draft.modes.mode_for(kind_now),
            kind_now,
        )
    };
    if rules_now != current {
        profiles.rules.set(rules_now.clone());
        controls.hub.set_rules(&rules_now);
        draft.editing_line.set(None);
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

#[derive(Clone)]
struct SyncBarCtx {
    state: Memo<PresetSaveState>,
    changes: Memo<Vec<PresetChange>>,
    profiles: ProfileSignals,
    draft: QuickRuleDraftSignals,
    controls: Arc<Controls>,
    langs: Signal<Option<Lang>>,
    confirm_discard: Signal<bool>,
    confirm_job: Stored<Option<Spawned<()>>>,
}

/// Bottom footer bar rendered inside Filter and Rules popovers when the active setup can be saved or updated.
pub(crate) fn preset_sync_bar(
    profiles: ProfileSignals,
    draft: QuickRuleDraftSignals,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let langs = ui_langs();
    let state = computed(move || live_preset_save_state(profiles, draft));
    let changes = computed(move || live_preset_unsaved_changes(profiles, draft));
    let confirm_discard = signal(false);
    let confirm_job = stored(None::<Spawned<()>>);
    effect(move || {
        let _ = state.get();
        confirm_job.replace(None);
        confirm_discard.set(false);
    });
    let ctx = SyncBarCtx {
        state,
        changes,
        profiles,
        draft,
        controls,
        langs,
        confirm_discard,
        confirm_job,
    };
    when(
        move || state.with(PresetSaveState::show_bar),
        move || {
            let status_ctx = ctx.clone();
            let discard_ctx = ctx.clone();
            let action_ctx = ctx.clone();
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
                .child(preset_sync_status(&status_ctx))
                .child(
                    view()
                        .flex_shrink_0()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .child(when(
                            move || matches!(state.get(), PresetSaveState::UpdateExisting(_)),
                            move || preset_sync_discard_button(discard_ctx.clone()),
                        ))
                        .child(preset_sync_action_button(action_ctx)),
                )
        },
    )
}

fn preset_sync_status(ctx: &SyncBarCtx) -> impl IntoElement {
    let state = ctx.state;
    let changes = ctx.changes;
    let langs = ctx.langs;
    let is_dirty = move || state.with(PresetSaveState::is_dirty);
    view()
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .gap_1p5()
        .overflow_hidden()
        .tooltip(
            tooltip().child(ui_tooltip_surface(
                text(move || -> SharedString {
                    state
                        .with(|s| changes.with(|c| sync_tooltip_text(s, c, ui_lang(langs))))
                        .into()
                })
                .max_w(move || px((window().width.0 - 32.0).clamp(220.0, 380.0)))
                .text_size(px(TEXT_SM))
                .text_color(fg_primary()),
            )),
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
                state
                    .with(|s| changes.with(|c| sync_status_text_with_changes(s, c, ui_lang(langs))))
                    .into()
            })
            .flex_1()
            .min_w(px(0.0))
            .number_of_lines(1)
            .text_overflow(EllipsizeMode::Tail)
            .text_size(px(TEXT_XS))
            .text_color(move || {
                if is_dirty() {
                    fg_secondary()
                } else {
                    fg_muted()
                }
            }),
        )
}

fn preset_sync_discard_button(ctx: SyncBarCtx) -> impl IntoElement {
    let state = ctx.state;
    let profiles = ctx.profiles;
    let draft = ctx.draft;
    let controls = ctx.controls;
    let langs = ctx.langs;
    let confirm = ctx.confirm_discard;
    let confirm_job = ctx.confirm_job;
    view()
        .role(AccessibilityRole::Button)
        .h(px(CONTROL_H))
        .px_2p5()
        .flex_shrink_0()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(move || {
            if confirm.get() {
                status_err_fg()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if confirm.get() {
                status_err_bg()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if confirm.get() {
                    status_err_bg()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .flex_row()
        .items_center()
        .justify_center()
        .focus_visible(apply_focus_ring)
        .label(move || -> SharedString {
            let lang = ui_lang(langs);
            if confirm.get() {
                t(lang, "确认丢弃", "Confirm").into()
            } else {
                t(lang, "丢弃", "Discard").into()
            }
        })
        .tooltip(
            tooltip().child(ui_tooltip_surface(
                text(move || -> SharedString {
                    let lang = ui_lang(langs);
                    if confirm.get() {
                        t(
                            lang,
                            "再次点击以丢弃全部未保存修改并恢复当前预设",
                            "Click again to discard all unsaved changes and restore preset",
                        )
                        .into()
                    } else {
                        t(
                            lang,
                            "丢弃未保存修改并恢复至已保存的预设",
                            "Discard unsaved changes and restore saved preset",
                        )
                        .into()
                    }
                })
                .text_size(px(TEXT_SM))
                .text_color(fg_primary()),
            )),
        )
        .on_press(move || {
            if confirm.get_untracked() {
                confirm_job.replace(None);
                confirm.set(false);
                if let PresetSaveState::UpdateExisting(name) = state.get_untracked() {
                    discard_preset_changes(profiles, draft, &name, &controls);
                }
            } else {
                confirm.set(true);
                confirm_job.replace(Some(set_timeout(Duration::from_millis(3500), move || {
                    confirm.set(false);
                })));
            }
        })
        .child(
            text(move || -> SharedString {
                let lang = ui_lang(langs);
                if confirm.get() {
                    t(lang, "确认丢弃", "Confirm").into()
                } else {
                    t(lang, "丢弃", "Discard").into()
                }
            })
            .text_size(px(TEXT_SM))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || {
                if confirm.get() {
                    status_err_fg()
                } else {
                    fg_secondary()
                }
            }),
        )
}

fn preset_sync_action_button(ctx: SyncBarCtx) -> impl IntoElement {
    let state = ctx.state;
    let profiles = ctx.profiles;
    let draft = ctx.draft;
    let controls = ctx.controls;
    let langs = ctx.langs;
    let confirm_discard = ctx.confirm_discard;
    let confirm_job = ctx.confirm_job;
    let is_dirty = move || state.with(PresetSaveState::is_dirty);
    view()
        .role(AccessibilityRole::Button)
        .h(px(CONTROL_H))
        .px_2p5()
        .flex_shrink_0()
        .rounded(px(RADIUS_MD))
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
        .active(|style| {
            style.bg(move || {
                if is_dirty() {
                    accent_blue_active()
                } else {
                    bg_control_active()
                }
            })
        })
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .flex_row()
        .items_center()
        .justify_center()
        .focus_visible(apply_focus_ring)
        .label(move || -> SharedString {
            state.with(|s| sync_button_label(s, ui_lang(langs))).into()
        })
        .on_press(move || {
            confirm_job.replace(None);
            confirm_discard.set(false);
            match state.get() {
                PresetSaveState::UpdateExisting(name) | PresetSaveState::SaveNamed(name) => {
                    commit_and_save_profile(profiles, draft, Some(&name), &controls);
                    dismiss_keyboard();
                }
                PresetSaveState::SaveAsNew => {
                    dismiss_keyboard();
                    profiles.filter_panel_open.set(false);
                    profiles.panel_open.set(false);
                    profiles.preset_panel_open.set(true);
                }
                PresetSaveState::Clean => {}
            }
        })
        .child(
            text(move || -> SharedString {
                state.with(|s| sync_button_label(s, ui_lang(langs))).into()
            })
            .text_size(px(TEXT_SM))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || {
                if is_dirty() {
                    fg_on_accent()
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

    #[test]
    fn preset_unsaved_changes_details_and_labels_cover_all_fields() {
        let mut base = sample_profile("test");
        base.filter = "host:api.vicanso.dev".to_string();
        base.rules = "cors *\nbreak api.vicanso.dev/v1".to_string();
        let saved = vec![base];

        let changes = preset_unsaved_changes(
            &PresetSnapshotRef {
                saved: &saved,
                typed_name: "Test",
                filter: "host:cdn.vicanso.dev",
                bypass: "*.local",
                rules: "cors *\ndelay api.vicanso.dev 200ms",
                upstream: "http://127.0.0.1:7890",
                rules_enabled: false,
                breaks_enabled: false,
                show_tunnel_failures: true,
            },
            true,
        );
        let fields: Vec<PresetChangeField> = changes.iter().map(|c| c.field).collect();
        assert_eq!(
            fields,
            vec![
                PresetChangeField::Name,
                PresetChangeField::Filter,
                PresetChangeField::Rules { incl_draft: true },
                PresetChangeField::Bypass,
                PresetChangeField::Upstream,
                PresetChangeField::RulesToggle,
                PresetChangeField::BreaksToggle,
                PresetChangeField::TunnelFailures,
            ]
        );
        let state = PresetSaveState::UpdateExisting("Test".into());
        let zh_inline = sync_status_text_with_changes(&state, &changes, Lang::Zh);
        assert_eq!(
            zh_inline,
            "预设「Test」未保存：名称、过滤、规则（含草稿）、直通、上游、规则开关、断点开关、隧道失败"
        );
        let en_tip = sync_tooltip_text(&state, &changes, Lang::En);
        assert!(en_tip.contains("Preset \"Test\" has 8 unsaved changes:"));
        assert!(en_tip.contains("• Name: test → Test"));
        assert!(en_tip.contains("• Filter: host:api.vicanso.dev → host:cdn.vicanso.dev"));
        assert!(en_tip.contains("• Rules (incl. draft): +1 / -1 lines (+ delay api.vicanso.dev 200ms; - break api.vicanso.dev/v1)"));
        assert!(en_tip.contains("• Bypass: empty → *.local"));
        assert!(en_tip.contains("• Breaks toggle: On → Off"));
        assert!(en_tip.contains("• Tunnel failures: Off → On"));
    }
}
