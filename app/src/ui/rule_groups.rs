#[allow(unused_imports)]
use super::*;

#[derive(Clone, Copy)]
pub(crate) struct RuleEditTarget {
    pub(crate) workspace: Signal<String>,
    pub(crate) groups: Signal<Vec<config::RuleGroup>>,
    pub(crate) editing: Signal<Option<String>>,
    pub(crate) focused_line: Signal<Option<(Option<String>, usize)>>,
    pub(crate) filter: Signal<String>,
    pub(crate) filter_enabled: Signal<bool>,
    pub(crate) panel_open: Signal<bool>,
}

impl RuleEditTarget {
    pub(crate) fn from_profiles(profiles: ProfileSignals) -> Self {
        Self {
            workspace: profiles.rules,
            groups: profiles.rule_groups,
            editing: profiles.editing_group,
            focused_line: profiles.focused_rule_line,
            filter: profiles.filter,
            filter_enabled: profiles.filter_enabled,
            panel_open: profiles.panel_open,
        }
    }

    pub(crate) fn is_line_focused(&self, line_no: usize) -> bool {
        let editing = self.editing.get();
        self.focused_line.with(|focused| {
            let Some((group, focused_no)) = focused else {
                return false;
            };
            if *focused_no != line_no {
                return false;
            }
            match (editing.as_deref(), group.as_deref()) {
                (None, None) => true,
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                _ => false,
            }
        })
    }

    pub(crate) fn current_text(&self) -> String {
        let workspace = self.workspace.get();
        let groups = self.groups.get();
        let editing = self.editing.get();
        active_rules_buffer_text(&workspace, &groups, editing.as_deref())
    }

    pub(crate) fn current_text_untracked(&self) -> String {
        let workspace = self.workspace.get_untracked();
        let groups = self.groups.get_untracked();
        let editing = self.editing.get_untracked();
        active_rules_buffer_text(&workspace, &groups, editing.as_deref())
    }

    pub(crate) fn commit(&self, controls: &Controls, next: String) {
        let editing = self.editing.get_untracked();
        if let Some(group_name) = editing.as_deref() {
            let current_groups = self.groups.get_untracked();
            if let Some(updated) =
                update_rule_group_rules(&current_groups, group_name, next.clone())
            {
                if updated == current_groups {
                    return;
                }
                commit_groups(*self, controls, updated);
                return;
            }
            self.editing.set(None);
        }
        if self.workspace.with_untracked(|cur| cur == &next) {
            return;
        }
        self.workspace.set(next.clone());
        controls.hub.set_rules(&next);
        save_config_or_notify(controls, PrefName::Rules, |config| config.rules = next);
    }
}

/// Returns the rules text currently shown in the editor for `editing` (`None` = workspace).
pub(crate) fn active_rules_buffer_text(
    workspace: &str,
    groups: &[config::RuleGroup],
    editing: Option<&str>,
) -> String {
    if let Some(name) = editing
        && let Some(group) = groups
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(name))
    {
        return group.rules.clone();
    }
    workspace.to_string()
}

/// Combines workspace rules (highest precedence) with all enabled rule groups.
pub(crate) fn effective_rules_text(workspace: &str, groups: &[config::RuleGroup]) -> String {
    crate::rules::combine_effective_rules_text(groups, workspace)
}

pub(crate) fn has_enabled_group_rules(groups: &[config::RuleGroup]) -> bool {
    groups
        .iter()
        .any(|group| group.enabled && !group.rules.trim().is_empty())
}

pub(crate) fn toggle_rule_group_enabled(
    groups: &[config::RuleGroup],
    name: &str,
) -> Vec<config::RuleGroup> {
    groups
        .iter()
        .cloned()
        .map(|mut item| {
            if item.name.eq_ignore_ascii_case(name) {
                item.enabled = !item.enabled;
            }
            item
        })
        .collect()
}

pub(crate) fn remove_rule_group(
    groups: &[config::RuleGroup],
    name: &str,
) -> Vec<config::RuleGroup> {
    groups
        .iter()
        .filter(|item| !item.name.eq_ignore_ascii_case(name))
        .cloned()
        .collect()
}

pub(crate) fn rule_group_exists(groups: &[config::RuleGroup], name: Option<&str>) -> bool {
    let Some(group_name) = name else {
        return true;
    };
    groups
        .iter()
        .any(|item| item.name.eq_ignore_ascii_case(group_name))
}

pub(crate) fn rename_rule_group(
    groups: &[config::RuleGroup],
    old_name: &str,
    new_name: &str,
) -> Option<(Vec<config::RuleGroup>, String)> {
    let trimmed = new_name.trim();
    if trimmed.is_empty() {
        return None;
    }
    let idx = groups
        .iter()
        .position(|item| item.name.eq_ignore_ascii_case(old_name))?;
    if groups
        .iter()
        .enumerate()
        .any(|(i, item)| i != idx && item.name.eq_ignore_ascii_case(trimmed))
    {
        return None;
    }
    let mut next = groups.to_vec();
    next[idx].name = trimmed.to_string();
    let clean = config::sanitize_rule_groups(next);
    let resolved = clean
        .iter()
        .find(|item| item.name.eq_ignore_ascii_case(trimmed))
        .map(|item| item.name.clone())
        .unwrap_or_else(|| trimmed.to_string());
    Some((clean, resolved))
}

pub(crate) fn update_rule_group_rules(
    groups: &[config::RuleGroup],
    name: &str,
    next_rules: String,
) -> Option<Vec<config::RuleGroup>> {
    let idx = groups
        .iter()
        .position(|item| item.name.eq_ignore_ascii_case(name))?;
    let mut updated = groups.to_vec();
    updated[idx].rules = next_rules;
    Some(updated)
}

fn commit_groups(target: RuleEditTarget, controls: &Controls, next: Vec<config::RuleGroup>) {
    let clean = config::sanitize_rule_groups(next);
    target.groups.set(clean.clone());
    controls.hub.set_rule_groups(&clean);
    save_config_or_notify(controls, PrefName::Rules, |config| {
        config.rule_groups = clean;
    });
}

fn confirm_new_group(
    target: RuleEditTarget,
    controls: &Controls,
    draft_name: Signal<String>,
    creating: Signal<bool>,
) {
    let name = draft_name.get_untracked().trim().to_string();
    draft_name.set(String::new());
    creating.set(false);
    if name.is_empty() {
        return;
    }
    let existing = target.groups.get_untracked();
    if let Some(found) = existing
        .iter()
        .find(|item| item.name.eq_ignore_ascii_case(&name))
    {
        target.editing.set(Some(found.name.clone()));
        set_status(controls, StatusMsg::RuleGroupExists(found.name.clone()));
        return;
    }
    if existing.len() >= config::RULE_GROUP_CAP {
        set_status(
            controls,
            StatusMsg::RuleGroupLimitReached(config::RULE_GROUP_CAP),
        );
        return;
    }
    let next = config::upsert_rule_group(&existing, &name, "", true);
    let Some(selected) = next
        .iter()
        .find(|item| item.name.eq_ignore_ascii_case(&name))
        .map(|item| item.name.clone())
    else {
        return;
    };
    commit_groups(target, controls, next);
    target.editing.set(Some(selected));
}

pub(crate) fn rule_groups_bar(target: RuleEditTarget, controls: Arc<Controls>) -> impl IntoElement {
    let langs = ui_langs();
    let creating = signal(false);
    let draft_name = signal(String::new());
    let confirm_delete = signal(None::<String>);
    let group_controls = controls.clone();
    let add_controls = controls;
    let at_cap = move || {
        target
            .groups
            .with(|groups| groups.len() >= config::RULE_GROUP_CAP)
    };
    view()
        .w_full()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_1()
        .child(workspace_pill(target, confirm_delete))
        .child(each(
            move || target.groups.get(),
            |group| group.name.to_ascii_lowercase(),
            move |group, _| group_pill(target, group, group_controls.clone(), confirm_delete),
        ))
        .child(
            when(
                move || creating.get(),
                move || new_group_inline(target, add_controls.clone(), draft_name, creating),
            )
            .otherwise(move || {
                view()
                    .flex_shrink_0()
                    .tooltip(
                        tooltip().child(ui_tooltip_surface(
                            text(move || -> SharedString {
                                let lang = ui_lang(langs);
                                if at_cap() {
                                    match lang {
                                        Lang::Zh => format!(
                                            "最多支持 {} 个规则分组，请先删除不需要的分组",
                                            config::RULE_GROUP_CAP
                                        ),
                                        Lang::En => format!(
                                            "Maximum {} rule groups reached",
                                            config::RULE_GROUP_CAP
                                        ),
                                    }
                                    .into()
                                } else {
                                    t(
                                        lang,
                                        "新建可独立开关的规则分组",
                                        "Create a toggleable rule group",
                                    )
                                    .into()
                                }
                            })
                            .text_size(px(TEXT_SM))
                            .text_color(fg_primary()),
                        )),
                    )
                    .child(
                        button()
                            .h(px(ControlSize::Xs.height()))
                            .px_1p5()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .rounded(px(RADIUS_SM))
                            .bg(AdaptiveColor::TRANSPARENT)
                            .hover(|s| {
                                s.bg(move || {
                                    if at_cap() {
                                        AdaptiveColor::TRANSPARENT
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
                            .disabled(at_cap)
                            .disabled_style(|s| s.opacity(0.45))
                            .accessibility_label(tr_opt("新建分组", "New group"))
                            .on_press(move |_: &()| {
                                if at_cap() {
                                    return;
                                }
                                confirm_delete.set(None);
                                draft_name.set(String::new());
                                creating.set(true);
                            })
                            .child(
                                text(tr("+ 分组", "+ Group"))
                                    .number_of_lines(1)
                                    .text_size(px(TEXT_XS))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(fg_muted()),
                            ),
                    )
            }),
        )
}

fn workspace_pill(
    target: RuleEditTarget,
    confirm_delete: Signal<Option<String>>,
) -> impl IntoElement {
    let is_selected = move || target.editing.get().is_none();
    let has_groups = move || !target.groups.with(Vec::is_empty);
    button()
        .selected(is_selected)
        .h(px(ControlSize::Xs.height()))
        .px_2()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_1()
        .rounded(px(RADIUS_SM))
        .border_1()
        .border_color(move || {
            if is_selected() && has_groups() {
                accent_blue_border()
            } else {
                border_subtle()
            }
        })
        .bg(move || {
            if is_selected() && has_groups() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|s| {
            s.bg(move || {
                if is_selected() && has_groups() {
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
        .accessibility_label(tr_opt("当前规则", "Workspace"))
        .on_press(move |_: &()| {
            confirm_delete.set(None);
            target.editing.set(None);
        })
        .child(
            text(move || -> SharedString {
                let count = target
                    .workspace
                    .with(|text| crate::rules::parse_rules(text).len());
                let base = tr("当前规则", "Workspace")();
                if count > 0 {
                    format!("{base} ({})", present::format_thousands(count as u64)).into()
                } else {
                    base.into()
                }
            })
            .number_of_lines(1)
            .text_size(px(TEXT_XS))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || {
                if is_selected() && has_groups() {
                    accent_blue_text()
                } else if is_selected() {
                    fg_primary()
                } else {
                    fg_secondary()
                }
            }),
        )
}

fn group_pill_rename_input(
    target: RuleEditTarget,
    group: ReadSignal<config::RuleGroup>,
    controls: Arc<Controls>,
    renaming: Signal<bool>,
    draft_name: Signal<String>,
) -> impl IntoElement {
    let submit_controls = controls.clone();
    let save_controls = controls;
    let commit_rename = Rc::new(move || {
        let old_name = group.with_untracked(|g| g.name.clone());
        let candidate = draft_name.get_untracked();
        let trimmed = candidate.trim();
        renaming.set(false);
        if trimmed.is_empty() || trimmed == old_name {
            return;
        }
        let current = target.groups.get_untracked();
        let Some((next, resolved)) = rename_rule_group(&current, &old_name, trimmed) else {
            if let Some(found) = current.iter().find(|item| {
                !item.name.eq_ignore_ascii_case(&old_name)
                    && item.name.eq_ignore_ascii_case(trimmed)
            }) {
                set_status(
                    &save_controls,
                    StatusMsg::RuleGroupExists(found.name.clone()),
                );
            }
            return;
        };
        if target.editing.with_untracked(|sel| {
            sel.as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case(&old_name))
        }) {
            target.editing.set(Some(resolved));
        }
        commit_groups(target, &save_controls, next);
    });
    let commit_enter = commit_rename.clone();
    let commit_btn = commit_rename;
    view()
        .h(px(ControlSize::Xs.height()))
        .flex_row()
        .items_center()
        .gap_1()
        .child(
            ui_input_a11y(input(), tr("重命名分组", "Rename group"))
                .value(draft_name)
                .placeholder(tr("重命名…", "Rename…"))
                .placeholder_color(move || color(fg_muted()))
                .w(px(96.0))
                .h(px(ControlSize::Xs.height()))
                .px_1p5()
                .rounded(px(RADIUS_SM))
                .bg(bg_code())
                .border_1()
                .border_color(accent_blue_border())
                .text_size(px(TEXT_XS))
                .text_color(fg_primary())
                .multiline(false)
                .auto_focus(true)
                .on_key_press(move |key: &str| {
                    if key == "Escape" {
                        renaming.set(false);
                    }
                })
                .on_change(move |text: &str| draft_name.set(text.to_string()))
                .on_submit(move |_: &str| {
                    let _ = &submit_controls;
                    commit_enter();
                }),
        )
        .child(
            view()
                .flex_shrink_0()
                .tooltip(label_tip(
                    "保存分组名称（Enter）",
                    "Save group name (Enter)",
                ))
                .child(
                    button()
                        .w(px(ControlSize::Xs.height()))
                        .h(px(ControlSize::Xs.height()))
                        .rounded(px(RADIUS_SM))
                        .border_1()
                        .border_color(accent_blue_border())
                        .bg(accent_blue_soft())
                        .hover(|s| s.bg(accent_blue_soft_hover()))
                        .active(|s| s.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .items_center()
                        .justify_center()
                        .accessibility_label(tr_opt("保存名称", "Save group name"))
                        .on_press(move |_: &()| commit_btn())
                        .child(glyph_px(
                            10.0,
                            || Glyph::Check,
                            || color(accent_blue_text()),
                        )),
                ),
        )
        .child(
            view()
                .flex_shrink_0()
                .tooltip(label_tip("取消重命名（Esc）", "Cancel rename (Esc)"))
                .child(
                    button()
                        .w(px(ControlSize::Xs.height()))
                        .h(px(ControlSize::Xs.height()))
                        .rounded(px(RADIUS_SM))
                        .border_1()
                        .border_color(border_control())
                        .bg(bg_control())
                        .hover(|s| s.bg(bg_control_hover()))
                        .active(|s| s.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .items_center()
                        .justify_center()
                        .accessibility_label(tr_opt("取消重命名", "Cancel rename"))
                        .on_press(move |_: &()| renaming.set(false))
                        .child(text("×").text_size(px(TEXT_XS)).text_color(fg_muted())),
                ),
        )
}

fn group_pill(
    target: RuleEditTarget,
    group: ReadSignal<config::RuleGroup>,
    controls: Arc<Controls>,
    confirm_delete: Signal<Option<String>>,
) -> impl IntoElement {
    let langs = ui_langs();
    let toggle_controls = controls.clone();
    let rename_controls = controls.clone();
    let remove_controls = controls;
    let renaming = signal(false);
    let rename_draft = signal(String::new());
    let is_selected = move || {
        let name = group.with(|g| g.name.clone());
        target.editing.with(|sel| {
            sel.as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case(&name))
        })
    };
    let is_enabled = move || group.with(|g| g.enabled);
    when(
        move || renaming.get(),
        move || {
            group_pill_rename_input(
                target,
                group,
                rename_controls.clone(),
                renaming,
                rename_draft,
            )
        },
    )
    .otherwise(move || {
        let remove_controls = remove_controls.clone();
        let toggle_controls = toggle_controls.clone();
        view()
            .h(px(ControlSize::Xs.height()))
            .pl_1p5()
            .pr_1()
            .max_w(px(240.0))
            .flex_shrink_0()
            .flex_row()
            .items_center()
            .gap_1()
            .rounded(px(RADIUS_SM))
            .border_1()
            .border_color(move || {
                if is_selected() {
                    accent_blue_border()
                } else {
                    border_control()
                }
            })
            .bg(move || {
                if is_selected() {
                    accent_blue_soft()
                } else {
                    bg_control()
                }
            })
            .child(
                view()
                    .flex_shrink_0()
                    .tooltip(label_tip("启用 / 停用此分组", "Enable / disable group"))
                    .child(
                        checkbox()
                            .checked(is_enabled)
                            .w(px(18.0))
                            .h(px(18.0))
                            .rounded(px(RADIUS_SM))
                            .hover(|s| s.bg(bg_control_hover()))
                            .active(|s| s.bg(bg_control_active()))
                            .transition_colors()
                            .duration(MOTION_INSTANT_MS)
                            .ease_out()
                            .focus_visible(apply_focus_ring)
                            .accessibility_label(move || -> Option<SharedString> {
                                let name = group.with(|g| g.name.clone());
                                Some(
                                    match (ui_lang(langs), is_enabled()) {
                                        (Lang::Zh, true) => format!("停用规则分组 {name}"),
                                        (Lang::Zh, false) => format!("启用规则分组 {name}"),
                                        (Lang::En, true) => format!("Disable rule group {name}"),
                                        (Lang::En, false) => format!("Enable rule group {name}"),
                                    }
                                    .into(),
                                )
                            })
                            .on_change(move |_: &bool| {
                                let name = group.with_untracked(|g| g.name.clone());
                                let next = toggle_rule_group_enabled(
                                    &target.groups.get_untracked(),
                                    &name,
                                );
                                commit_groups(target, &toggle_controls, next);
                            })
                            .indicator_style(|s| {
                                s.w(px(18.0))
                                    .h(px(18.0))
                                    .rounded(px(RADIUS_SM))
                                    .border_1()
                                    .border_color(border_control())
                                    .bg(bg_code())
                                    .items_center()
                                    .justify_center()
                                    .checked(|c| {
                                        c.border_color(accent_blue_border()).bg(accent_blue_soft())
                                    })
                            })
                            .indicator(
                                mono(move || -> SharedString {
                                    if is_enabled() {
                                        "✓".into()
                                    } else {
                                        "".into()
                                    }
                                })
                                .text_size(px(TEXT_XS))
                                .font_weight(FontWeight::BOLD)
                                .text_color(accent_blue_text()),
                            ),
                    ),
            )
            .child(
                button()
                    .selected(is_selected)
                    .min_w(px(0.0))
                    .rounded(px(RADIUS_SM))
                    .hover(|s| s.bg(bg_control_hover()))
                    .active(|s| s.bg(bg_control_active()))
                    .transition_colors()
                    .duration(MOTION_INSTANT_MS)
                    .ease_out()
                    .focus_visible(apply_focus_ring)
                    .accessibility_label(move || Some(group.with(|g| g.name.clone()).into()))
                    .on_press(move |_: &()| {
                        confirm_delete.set(None);
                        let name = group.with_untracked(|g| g.name.clone());
                        target.editing.set(Some(name));
                    })
                    .child(
                        text(move || -> SharedString {
                            group
                                .with(|g| {
                                    let count = crate::rules::parse_rules(&g.rules).len();
                                    if count > 0 {
                                        format!(
                                            "{} ({})",
                                            g.name,
                                            present::format_thousands(count as u64)
                                        )
                                    } else {
                                        g.name.clone()
                                    }
                                })
                                .into()
                        })
                        .max_w(px(150.0))
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if is_selected() {
                                accent_blue_text()
                            } else if is_enabled() {
                                fg_primary()
                            } else {
                                fg_muted()
                            }
                        }),
                    ),
            )
            .child(when(is_selected, move || {
                view()
                    .flex_shrink_0()
                    .tooltip(label_tip("重命名此规则分组", "Rename rule group"))
                    .child(
                        button()
                            .w(px(18.0))
                            .h(px(18.0))
                            .rounded(px(RADIUS_SM))
                            .items_center()
                            .justify_center()
                            .hover(|s| s.bg(bg_control_hover()))
                            .active(|s| s.bg(bg_control_active()))
                            .transition_colors()
                            .duration(MOTION_INSTANT_MS)
                            .ease_out()
                            .focus_visible(apply_focus_ring)
                            .accessibility_label(tr_opt("重命名分组", "Rename group"))
                            .on_press(move |_: &()| {
                                confirm_delete.set(None);
                                rename_draft.set(group.with_untracked(|g| g.name.clone()));
                                renaming.set(true);
                            })
                            .child(glyph_px(10.0, || Glyph::RuleRewrite, || color(fg_muted()))),
                    )
            }))
            .child(group_delete_button(
                target,
                group,
                remove_controls,
                confirm_delete,
            ))
    })
}

fn group_delete_button(
    target: RuleEditTarget,
    group: ReadSignal<config::RuleGroup>,
    controls: Arc<Controls>,
    confirm_delete: Signal<Option<String>>,
) -> impl IntoElement {
    let is_confirming = move || {
        group.with(|g| {
            confirm_delete.with(|pending| {
                pending
                    .as_deref()
                    .is_some_and(|p| p.eq_ignore_ascii_case(&g.name))
            })
        })
    };
    when(is_confirming, move || {
        let remove_controls = controls.clone();
        view()
            .flex_shrink_0()
            .tooltip(label_tip(
                "再次点击确认删除此分组",
                "Click again to delete rule group",
            ))
            .child(
                button()
                    .h(px(20.0))
                    .px_1()
                    .rounded(px(RADIUS_SM))
                    .items_center()
                    .justify_center()
                    .bg(danger_button_bg())
                    .hover(|s| s.bg(danger_button_hover()))
                    .active(|s| s.bg(danger_button_active()))
                    .transition_colors()
                    .duration(MOTION_INSTANT_MS)
                    .ease_out()
                    .focus_visible(apply_focus_ring)
                    .accessibility_label(tr_opt(
                        "再次点击确认删除分组",
                        "Click again to delete rule group",
                    ))
                    .on_press(move |_: &()| {
                        let name = group.with_untracked(|g| g.name.clone());
                        confirm_delete.set(None);
                        if target.editing.with_untracked(|sel| {
                            sel.as_deref()
                                .is_some_and(|s| s.eq_ignore_ascii_case(&name))
                        }) {
                            target.editing.set(None);
                        }
                        let next = remove_rule_group(&target.groups.get_untracked(), &name);
                        commit_groups(target, &remove_controls, next);
                        set_status(&remove_controls, StatusMsg::RuleGroupDeleted(name));
                    })
                    .child(
                        text(tr("确认？", "Del?"))
                            .number_of_lines(1)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg_on_accent()),
                    ),
            )
    })
    .otherwise(move || {
        view()
            .flex_shrink_0()
            .tooltip(label_tip(
                "删除此规则分组（点击两次确认）",
                "Delete rule group (click twice to confirm)",
            ))
            .child(
                button()
                    .w(px(20.0))
                    .h(px(20.0))
                    .rounded(px(RADIUS_SM))
                    .items_center()
                    .justify_center()
                    .hover(|s| s.bg(bg_control_hover()))
                    .active(|s| s.bg(bg_control_active()))
                    .transition_colors()
                    .duration(MOTION_INSTANT_MS)
                    .ease_out()
                    .focus_visible(apply_focus_ring)
                    .accessibility_label(tr_opt("删除分组", "Delete rule group"))
                    .on_press(move |_: &()| {
                        let name = group.with_untracked(|g| g.name.clone());
                        confirm_delete.set(Some(name));
                    })
                    .child(glyph_px(10.0, || Glyph::Close, || color(fg_muted()))),
            )
    })
}

fn new_group_inline(
    target: RuleEditTarget,
    controls: Arc<Controls>,
    draft_name: Signal<String>,
    creating: Signal<bool>,
) -> impl IntoElement {
    let submit_controls = controls.clone();
    view()
        .h(px(ControlSize::Xs.height()))
        .flex_row()
        .items_center()
        .gap_1()
        .child(
            ui_input_a11y(input(), tr("新建分组名称", "New group name"))
                .value(draft_name)
                .placeholder(tr("分组名…", "Group…"))
                .placeholder_color(move || color(fg_muted()))
                .w(px(108.0))
                .h(px(ControlSize::Xs.height()))
                .px_1p5()
                .rounded(px(RADIUS_SM))
                .bg(bg_code())
                .border_1()
                .border_color(accent_blue_border())
                .text_size(px(TEXT_XS))
                .text_color(fg_primary())
                .multiline(false)
                .auto_focus(true)
                .on_key_press(move |key: &str| {
                    if key == "Escape" {
                        draft_name.set(String::new());
                        creating.set(false);
                    }
                })
                .on_change(move |text: &str| draft_name.set(text.to_string()))
                .on_submit(move |_: &str| {
                    confirm_new_group(target, &submit_controls, draft_name, creating);
                }),
        )
        .child(
            button()
                .h(px(ControlSize::Xs.height()))
                .px_1p5()
                .rounded(px(RADIUS_SM))
                .border_1()
                .border_color(accent_blue_border())
                .bg(accent_blue_soft())
                .hover(|s| s.bg(accent_blue_soft_hover()))
                .active(|s| s.bg(bg_control_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .items_center()
                .justify_center()
                .accessibility_label(tr_opt("创建", "Add"))
                .on_press(move |_: &()| confirm_new_group(target, &controls, draft_name, creating))
                .child(
                    text(tr("创建", "Add"))
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(accent_blue_text()),
                ),
        )
        .child(
            view()
                .flex_shrink_0()
                .tooltip(label_tip("取消新建分组（Esc）", "Cancel new group (Esc)"))
                .child(
                    button()
                        .w(px(ControlSize::Xs.height()))
                        .h(px(ControlSize::Xs.height()))
                        .rounded(px(RADIUS_SM))
                        .border_1()
                        .border_color(border_control())
                        .bg(bg_control())
                        .hover(|s| s.bg(bg_control_hover()))
                        .active(|s| s.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .items_center()
                        .justify_center()
                        .accessibility_label(tr_opt("取消新建分组", "Cancel new group"))
                        .on_press(move |_: &()| {
                            draft_name.set(String::new());
                            creating.set(false);
                        })
                        .child(text("×").text_size(px(TEXT_XS)).text_color(fg_muted())),
                ),
        )
}
