//! Pre-save rule dry-run preview strip against current session flows (B3).

#[allow(unused_imports)]
use super::*;

/// Formats the dry-run preview message and whether it should be styled as a warning (0 matches).
pub(crate) fn format_dry_run_message(
    summary: &crate::rules::DryRunSummary,
    is_draft: bool,
    lang: Lang,
) -> (String, bool) {
    if summary.total_flows == 0 {
        let msg = match lang {
            Lang::Zh => "当前会话暂无流量，收到流量后可自动试跑".to_string(),
            Lang::En => "No session flows yet for dry-run preview".to_string(),
        };
        return (msg, false);
    }
    if summary.matched_flows == 0 {
        let msg = match (lang, is_draft) {
            (Lang::Zh, true) => "草稿 0 条命中 — 检查域名或路径模式".to_string(),
            (Lang::Zh, false) => "0 条命中 — 检查域名或路径模式".to_string(),
            (Lang::En, true) => "Draft: 0 matches — check host pattern".to_string(),
            (Lang::En, false) => "0 matches — check host pattern".to_string(),
        };
        return (msg, true);
    }
    let matched = present::format_thousands(summary.matched_flows as u64);
    let total = present::format_thousands(summary.total_flows as u64);
    let msg = match (lang, is_draft) {
        (Lang::Zh, true) => format!("草稿试跑：将命中 {matched} / {total} 条已有流量"),
        (Lang::Zh, false) => format!("试跑预览：将命中 {matched} / {total} 条已有流量"),
        (Lang::En, true) => {
            format!("Draft dry-run: will match {matched} of {total} existing flows")
        }
        (Lang::En, false) => format!("Dry-run: will match {matched} of {total} existing flows"),
    };
    (msg, false)
}

pub(crate) fn format_dry_run_samples(summary: &crate::rules::DryRunSummary, lang: Lang) -> String {
    if summary.sample_targets.is_empty() {
        return String::new();
    }
    let joined = summary.sample_targets.join(" · ");
    match lang {
        Lang::Zh => format!("示例：{joined}"),
        Lang::En => format!("Samples: {joined}"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DryRunInput {
    Existing,
    Draft(String),
    IncompleteDraft,
}

fn target_rules_paused(target: RuleEditTarget, rules_on: Option<Signal<bool>>) -> bool {
    if rules_on.is_some_and(|on| !on.get()) {
        return true;
    }
    let Some(editing_name) = target.editing.get() else {
        return false;
    };
    target.groups.with(|groups| {
        groups
            .iter()
            .find(|g| g.name.eq_ignore_ascii_case(&editing_name))
            .is_some_and(|g| !g.enabled)
    })
}

/// Compact pre-save dry-run preview bar shown below the rules editor/strip.
pub(crate) fn rule_dry_run_strip(
    target: RuleEditTarget,
    controls: Arc<Controls>,
    draft_line: impl Fn() -> Option<String> + 'static,
) -> impl IntoElement {
    rule_dry_run_strip_live(target, None, controls, move || match draft_line() {
        Some(line) if !line.trim().is_empty() => DryRunInput::Draft(line),
        _ => DryRunInput::Existing,
    })
}

pub(crate) fn rule_dry_run_strip_live(
    target: RuleEditTarget,
    rules_on: Option<Signal<bool>>,
    controls: Arc<Controls>,
    draft_state: impl Fn() -> DryRunInput + 'static,
) -> impl IntoElement {
    let langs = ui_langs();
    let catalog = filter_catalog_context();
    let preview = computed(move || {
        // Subscribe to session changes via the live filter catalog.
        let _ = catalog.get();
        let state = draft_state();
        match state {
            DryRunInput::IncompleteDraft => (None, false, true),
            DryRunInput::Draft(line) => {
                let summary = controls
                    .hub
                    .with_session(|session| crate::rules::dry_run_rules(&line, session.iter()));
                (Some(summary), true, false)
            }
            DryRunInput::Existing => {
                let current = target.current_text();
                let summary = controls
                    .hub
                    .with_session(|session| crate::rules::dry_run_rules(&current, session.iter()));
                (Some(summary), false, false)
            }
        }
    });

    when(
        move || {
            preview.with(|(summary, _, incomplete)| {
                *incomplete || summary.as_ref().is_some_and(|s| s.rule_count > 0)
            })
        },
        move || {
            let is_incomplete = move || preview.with(|(_, _, inc)| *inc);
            let is_paused = move || !is_incomplete() && target_rules_paused(target, rules_on);
            let is_warn = move || {
                !is_incomplete()
                    && preview.with(|(summary, _, _)| {
                        summary
                            .as_ref()
                            .is_some_and(|s| s.total_flows > 0 && s.matched_flows == 0)
                    })
            };
            let is_matched = move || {
                !is_incomplete()
                    && preview.with(|(summary, _, _)| {
                        summary.as_ref().is_some_and(|s| s.matched_flows > 0)
                    })
            };
            view()
                .w_full()
                .px_2()
                .py_1()
                .rounded(px(RADIUS_MD))
                .border_1()
                .border_color(move || {
                    if is_warn() || is_paused() {
                        status_warn_fg()
                    } else if is_matched() {
                        accent_blue_border()
                    } else {
                        border_subtle()
                    }
                })
                .bg(move || {
                    if is_warn() || is_paused() {
                        status_warn_bg()
                    } else if is_matched() {
                        accent_blue_soft()
                    } else {
                        bg_control()
                    }
                })
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(
                    text(move || -> SharedString {
                        let lang = ui_lang(langs);
                        preview
                            .with(|(summary, is_draft, incomplete)| {
                                if *incomplete {
                                    t(
                                        lang,
                                        "草稿未填写完整，完成后将实时试跑",
                                        "Draft incomplete — finish required fields to preview",
                                    )
                                    .to_string()
                                } else if let Some(s) = summary {
                                    let mut msg = format_dry_run_message(s, *is_draft, lang).0;
                                    if target_rules_paused(target, rules_on) {
                                        msg.push_str(t(
                                            lang,
                                            "（当前未启用）",
                                            " (currently paused)",
                                        ));
                                    }
                                    msg
                                } else {
                                    String::new()
                                }
                            })
                            .into()
                    })
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail)
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || {
                        if is_warn() || is_paused() {
                            status_warn_fg()
                        } else if is_matched() {
                            accent_blue_text()
                        } else {
                            fg_muted()
                        }
                    }),
                )
                .child(when(
                    move || {
                        !is_incomplete()
                            && preview.with(|(summary, _, _)| {
                                summary
                                    .as_ref()
                                    .is_some_and(|s| !s.sample_targets.is_empty())
                            })
                    },
                    move || {
                        mono(move || -> SharedString {
                            let lang = ui_lang(langs);
                            preview
                                .with(|(summary, _, _)| {
                                    summary
                                        .as_ref()
                                        .map(|s| format_dry_run_samples(s, lang))
                                        .unwrap_or_default()
                                })
                                .into()
                        })
                        .flex_1()
                        .min_w(px(0.0))
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .text_size(px(TEXT_XS))
                        .text_color(fg_muted())
                    },
                ))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_messages_cover_matched_zero_and_empty_session() {
        let empty = crate::rules::DryRunSummary {
            rule_count: 1,
            total_flows: 0,
            matched_flows: 0,
            sample_targets: vec![],
        };
        let (msg_en, warn) = format_dry_run_message(&empty, false, Lang::En);
        assert!(!warn);
        assert!(msg_en.contains("No session flows"));

        let zero = crate::rules::DryRunSummary {
            rule_count: 1,
            total_flows: 10,
            matched_flows: 0,
            sample_targets: vec![],
        };
        let (msg_zero_en, warn_zero) = format_dry_run_message(&zero, false, Lang::En);
        assert!(warn_zero);
        assert!(msg_zero_en.contains("0 matches"));
        let (msg_zero_zh, warn_draft) = format_dry_run_message(&zero, true, Lang::Zh);
        assert!(warn_draft);
        assert!(msg_zero_zh.contains("0 条命中"));

        let hit = crate::rules::DryRunSummary {
            rule_count: 2,
            total_flows: 10,
            matched_flows: 3,
            sample_targets: vec!["api.test/a".into(), "api.test/b".into()],
        };
        let (msg_hit_en, warn_hit) = format_dry_run_message(&hit, false, Lang::En);
        assert!(!warn_hit);
        assert!(msg_hit_en.contains("will match 3 of 10 existing flows"));
        let (msg_draft_zh, _) = format_dry_run_message(&hit, true, Lang::Zh);
        assert!(msg_draft_zh.contains("3 / 10"));
        assert!(format_dry_run_samples(&hit, Lang::En).contains("api.test/a"));
    }
}
