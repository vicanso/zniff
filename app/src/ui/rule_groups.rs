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
        return;
    }
    let next = config::upsert_rule_group(&existing, &name, "", true);
    let selected = next
        .iter()
        .find(|item| item.name.eq_ignore_ascii_case(&name))
        .map(|item| item.name.clone())
        .unwrap_or(name);
    commit_groups(target, controls, next);
    target.editing.set(Some(selected));
}

pub(crate) fn rule_groups_bar(target: RuleEditTarget, controls: Arc<Controls>) -> impl IntoElement {
    let creating = signal(false);
    let draft_name = signal(String::new());
    let group_controls = controls.clone();
    let add_controls = controls;
    view()
        .w_full()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_1()
        .child(workspace_pill(target))
        .child(each(
            move || target.groups.get(),
            |group| group.name.to_ascii_lowercase(),
            move |group, _| group_pill(target, group, group_controls.clone()),
        ))
        .child(
            when(
                move || creating.get(),
                move || new_group_inline(target, add_controls.clone(), draft_name, creating),
            )
            .otherwise(move || {
                view()
                    .role(AccessibilityRole::Button)
                    .h(px(ControlSize::Xs.height()))
                    .px_1p5()
                    .flex_shrink_0()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .rounded(px(RADIUS_SM))
                    .bg(AdaptiveColor::TRANSPARENT)
                    .hover(|s| s.bg(bg_control_hover()))
                    .active(|s| s.bg(bg_control_active()))
                    .transition_colors()
                    .duration(MOTION_INSTANT_MS)
                    .ease_out()
                    .focus_visible(apply_focus_ring)
                    .tooltip(label_tip(
                        "新建可独立开关的规则分组",
                        "Create a toggleable rule group",
                    ))
                    .on_press(move || {
                        draft_name.set(String::new());
                        creating.set(true);
                    })
                    .child(
                        text(tr("+ 分组", "+ Group"))
                            .number_of_lines(1)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg_muted()),
                    )
            }),
        )
}

fn workspace_pill(target: RuleEditTarget) -> impl IntoElement {
    let is_selected = move || target.editing.get().is_none();
    let has_groups = move || !target.groups.with(Vec::is_empty);
    view()
        .role(AccessibilityRole::Button)
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
        .on_press(move || target.editing.set(None))
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

fn group_pill(
    target: RuleEditTarget,
    group: ReadSignal<config::RuleGroup>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let toggle_controls = controls.clone();
    let remove_controls = controls;
    let is_selected = move || {
        let name = group.with(|g| g.name.clone());
        target.editing.with(|sel| {
            sel.as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case(&name))
        })
    };
    let is_enabled = move || group.with(|g| g.enabled);
    view()
        .h(px(ControlSize::Xs.height()))
        .pl_1p5()
        .pr_1()
        .max_w(px(220.0))
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
                .role(AccessibilityRole::Checkbox)
                .w(px(14.0))
                .h(px(14.0))
                .flex_shrink_0()
                .rounded(px(RADIUS_SM))
                .border_1()
                .border_color(move || {
                    if is_enabled() {
                        accent_blue_border()
                    } else {
                        border_control()
                    }
                })
                .bg(move || {
                    if is_enabled() {
                        accent_blue_soft()
                    } else {
                        bg_code()
                    }
                })
                .hover(|s| s.bg(bg_control_hover()))
                .active(|s| s.bg(bg_control_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .items_center()
                .justify_center()
                .tooltip(label_tip("启用 / 停用此分组", "Enable / disable group"))
                .on_press(move || {
                    let name = group.with_untracked(|g| g.name.clone());
                    let next = toggle_rule_group_enabled(&target.groups.get_untracked(), &name);
                    commit_groups(target, &toggle_controls, next);
                })
                .child(
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
        )
        .child(
            view()
                .role(AccessibilityRole::Button)
                .min_w(px(0.0))
                .rounded(px(RADIUS_SM))
                .hover(|s| s.bg(bg_control_hover()))
                .active(|s| s.bg(bg_control_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .on_press(move || {
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
        .child(
            view()
                .role(AccessibilityRole::Button)
                .w(px(14.0))
                .h(px(14.0))
                .flex_shrink_0()
                .rounded(px(RADIUS_SM))
                .items_center()
                .justify_center()
                .hover(|s| s.bg(bg_control_hover()))
                .active(|s| s.bg(bg_control_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .tooltip(label_tip("删除此规则分组", "Delete rule group"))
                .on_press(move || {
                    let name = group.with_untracked(|g| g.name.clone());
                    if target.editing.with_untracked(|sel| {
                        sel.as_deref()
                            .is_some_and(|s| s.eq_ignore_ascii_case(&name))
                    }) {
                        target.editing.set(None);
                    }
                    let next = remove_rule_group(&target.groups.get_untracked(), &name);
                    commit_groups(target, &remove_controls, next);
                })
                .child(text("×").text_size(px(TEXT_XS)).text_color(fg_muted())),
        )
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
            input()
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
                .on_change(move |text: &str| draft_name.set(text.to_string()))
                .on_submit(move |_: &str| {
                    confirm_new_group(target, &submit_controls, draft_name, creating);
                }),
        )
        .child(
            view()
                .role(AccessibilityRole::Button)
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
                .on_press(move || confirm_new_group(target, &controls, draft_name, creating))
                .child(
                    text(tr("创建", "Add"))
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(accent_blue_text()),
                ),
        )
        .child(
            view()
                .role(AccessibilityRole::Button)
                .w(px(20.0))
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
                .on_press(move || {
                    draft_name.set(String::new());
                    creating.set(false);
                })
                .child(text("×").text_size(px(TEXT_XS)).text_color(fg_muted())),
        )
}
