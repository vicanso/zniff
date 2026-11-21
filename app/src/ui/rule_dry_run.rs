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
    let msg = match (lang, is_draft) {
        (Lang::Zh, true) => format!(
            "草稿试跑：将命中 {} / {} 条已有流量",
            summary.matched_flows, summary.total_flows
        ),
        (Lang::Zh, false) => format!(
            "试跑预览：将命中 {} / {} 条已有流量",
            summary.matched_flows, summary.total_flows
        ),
        (Lang::En, true) => format!(
            "Draft dry-run: will match {} of {} existing flows",
            summary.matched_flows, summary.total_flows
        ),
        (Lang::En, false) => format!(
            "Dry-run: will match {} of {} existing flows",
            summary.matched_flows, summary.total_flows
        ),
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

/// Compact pre-save dry-run preview bar shown below the rules editor/strip.
pub(crate) fn rule_dry_run_strip(
    target: RuleEditTarget,
    controls: Arc<Controls>,
    draft_line: impl Fn() -> Option<String> + 'static,
) -> impl IntoElement {
    let langs = ui_langs();
    let catalog = filter_catalog_context();
    let preview = computed(move || {
        // Subscribe to session changes via the live filter catalog.
        let _ = catalog.get();
        let draft = draft_line();
        let (eval_text, is_draft) = match draft {
            Some(line) if !line.trim().is_empty() => (line, true),
            _ => (target.current_text(), false),
        };
        let summary = controls
            .hub
            .with_session(|session| crate::rules::dry_run_rules(&eval_text, session.iter()));
        (summary, is_draft)
    });

    when(
        move || preview.with(|(summary, _)| summary.rule_count > 0),
        move || {
            let is_warn = move || {
                preview.with(|(summary, _)| summary.total_flows > 0 && summary.matched_flows == 0)
            };
            let is_matched = move || preview.with(|(summary, _)| summary.matched_flows > 0);
            view()
                .w_full()
                .px_2()
                .py_1()
                .rounded(px(6.0))
                .border_1()
                .border_color(move || {
                    if is_warn() {
                        status_warn_fg()
                    } else if is_matched() {
                        accent_blue_border()
                    } else {
                        border_subtle()
                    }
                })
                .bg(move || {
                    if is_warn() {
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
                            .with(|(summary, is_draft)| {
                                format_dry_run_message(summary, *is_draft, lang).0
                            })
                            .into()
                    })
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail)
                    .text_size(px(11.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || {
                        if is_warn() {
                            status_warn_fg()
                        } else if is_matched() {
                            accent_blue_text()
                        } else {
                            fg_muted()
                        }
                    }),
                )
                .child(when(
                    move || preview.with(|(summary, _)| !summary.sample_targets.is_empty()),
                    move || {
                        mono(move || -> SharedString {
                            let lang = ui_lang(langs);
                            preview
                                .with(|(summary, _)| format_dry_run_samples(summary, lang))
                                .into()
                        })
                        .flex_1()
                        .min_w(px(0.0))
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .text_size(px(10.5))
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
