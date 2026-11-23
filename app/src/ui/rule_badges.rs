#[allow(unused_imports)]
use super::*;

/// Maps a rule verb to a compact vector icon for the traffic list and detail badges.
pub(crate) fn rule_glyph_for_verb(verb: &str) -> Glyph {
    match verb {
        "break" | "break-res" => Glyph::RuleBreak,
        "block" => Glyph::RuleBlock,
        "map-local" | "map-remote" | "resolve" => Glyph::RuleMap,
        "delay" | "throttle" => Glyph::RuleTiming,
        _ => Glyph::RuleRewrite,
    }
}

/// Converts a compact flow rule marker string (e.g. `"✎⏱"`) into ordered vector glyphs.
pub(crate) fn rule_marker_glyphs(marker: &str) -> Vec<Glyph> {
    marker
        .chars()
        .filter_map(|ch| match ch {
            '⏸' => Some(Glyph::RuleBreak),
            '⛔' => Some(Glyph::RuleBlock),
            '↪' => Some(Glyph::RuleMap),
            '✎' => Some(Glyph::RuleRewrite),
            '⏱' => Some(Glyph::RuleTiming),
            _ => None,
        })
        .collect()
}

/// Builds a compact, deduplicated icon string (e.g. `✎`, `↪⏱`, `⏸`) for a flow row.
/// Returns an empty string when no rules matched.
pub(crate) fn rule_marker_for_flow(hits: &[session::RuleHit], fallback_summary: &str) -> String {
    let mut has_break = false;
    let mut has_block = false;
    let mut has_map = false;
    let mut has_rewrite = false;
    let mut has_timing = false;

    let mut classify = |verb: &str| match verb {
        "" => {}
        "break" | "break-res" => has_break = true,
        "block" => has_block = true,
        "map-local" | "map-remote" | "resolve" => has_map = true,
        "delay" | "throttle" => has_timing = true,
        _ => has_rewrite = true,
    };

    if !hits.is_empty() {
        for hit in hits {
            classify(&hit.verb);
        }
    } else if !fallback_summary.is_empty() {
        for token in fallback_summary.split_whitespace() {
            classify(token);
        }
    } else {
        return String::new();
    }

    let mut out = String::new();
    if has_break {
        out.push('⏸');
    }
    if has_block {
        out.push('⛔');
    }
    if has_map {
        out.push('↪');
    }
    if has_rewrite {
        out.push('✎');
    }
    if has_timing {
        out.push('⏱');
    }
    out
}

/// Formats a compact hover summary of matched rules for the path cell's hover bubble.
pub(crate) fn rule_hover_summary(hits: &[session::RuleHit], fallback_summary: &str) -> String {
    if !hits.is_empty() {
        hits.iter()
            .map(session::RuleHit::format_badge)
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        fallback_summary.to_string()
    }
}

/// Opens the rules panel switched to `hit`'s source group (or workspace) and highlights its line.
pub(crate) fn focus_rule_hit(profiles: ProfileSignals, hit: &session::RuleHit) {
    profiles.editing_group.set(hit.group_name.clone());
    profiles.focused_rule_line.set(if hit.line_no > 0 {
        Some((hit.group_name.clone(), hit.line_no))
    } else {
        None
    });
    profiles.panel_open.set(true);
}

fn rule_badge_colors(verb: &str) -> (AdaptiveColor, AdaptiveColor, AdaptiveColor) {
    match verb {
        "block" => (status_err_bg(), status_err_fg(), status_err_fg()),
        "break" | "break-res" | "delay" | "throttle" => {
            (status_warn_bg(), status_warn_fg(), status_warn_fg())
        }
        _ => (accent_blue_soft(), accent_blue_border(), accent_blue_text()),
    }
}

/// Renders clickable per-rule hit badges (plus optional GraphQL op badge) in the detail summary.
pub(crate) fn detail_rule_badges(shown: Shown, profiles: ProfileSignals) -> impl IntoElement {
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_1()
        .child(when(
            move || shown.rule_hits.with(|hits| !hits.is_empty()),
            move || {
                each(
                    move || {
                        shown
                            .rule_hits
                            .get()
                            .into_iter()
                            .enumerate()
                            .collect::<Vec<_>>()
                    },
                    |(idx, hit)| {
                        format!(
                            "{idx}:{}:{}:{}",
                            hit.group_name.as_deref().unwrap_or(""),
                            hit.line_no,
                            hit.verb
                        )
                    },
                    move |entry, _| {
                        let (_, hit) = entry.get_untracked();
                        let (bg_col, border_col, fg_col) = rule_badge_colors(&hit.verb);
                        let glyph = rule_glyph_for_verb(&hit.verb);
                        let label = hit.format_badge();
                        let click_hit = hit.clone();
                        view()
                            .max_w(px(220.0))
                            .overflow_hidden()
                            .px_2()
                            .py_0p5()
                            .rounded(px(5.0))
                            .border_1()
                            .border_color(border_col)
                            .bg(bg_col)
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .hover(|style| style.opacity(0.85))
                            .active(|style| style.opacity(0.7))
                            .cursor(Cursor::Pointer)
                            .tooltip(label_tip(
                                "点击在规则编辑器中定位此规则",
                                "Click to jump to this rule in editor",
                            ))
                            .on_press(move || focus_rule_hit(profiles, &click_hit))
                            .child(glyph_px(11.0, move || glyph, move || color(fg_col)))
                            .child(
                                mono(label)
                                    .min_w(px(0.0))
                                    .flex_shrink(1.0)
                                    .number_of_lines(1)
                                    .text_overflow(EllipsizeMode::Tail)
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(fg_col),
                            )
                    },
                )
            },
        ))
        .child(when(
            move || {
                shown.rule_hits.with(Vec::is_empty)
                    && !shown.rules.get().is_empty()
                    && shown.rules.get() != shown.gql_badge.get()
            },
            move || {
                view()
                    .max_w(px(180.0))
                    .overflow_hidden()
                    .px_2()
                    .py_0p5()
                    .rounded(px(5.0))
                    .bg(accent_blue_soft())
                    .child(
                        mono(move || {
                            let all = shown.rules.get();
                            let gql = shown.gql_badge.get();
                            if gql.is_empty() {
                                all
                            } else {
                                all.split_whitespace()
                                    .filter(|tok| *tok != gql)
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            }
                        })
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(accent_blue_text()),
                    )
            },
        ))
        .child(when(
            move || !shown.gql_badge.get().is_empty(),
            move || {
                view()
                    .max_w(px(180.0))
                    .overflow_hidden()
                    .px_2()
                    .py_0p5()
                    .rounded(px(5.0))
                    .bg(accent_blue_soft())
                    .child(
                        mono(move || shown.gql_badge.get())
                            .number_of_lines(1)
                            .text_overflow(EllipsizeMode::Tail)
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(accent_blue_text()),
                    )
            },
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_markers_and_badges_format_correctly() {
        assert_eq!(rule_marker_for_flow(&[], ""), "");
        assert_eq!(rule_marker_for_flow(&[], "replace header"), "✎");
        assert_eq!(rule_marker_for_flow(&[], "map-local delay"), "↪⏱");

        let hits = vec![
            session::RuleHit {
                group_name: None,
                line_no: 3,
                verb: "replace".into(),
            },
            session::RuleHit {
                group_name: Some("qa".into()),
                line_no: 2,
                verb: "break".into(),
            },
            session::RuleHit {
                group_name: None,
                line_no: 5,
                verb: "delay".into(),
            },
        ];
        assert_eq!(rule_marker_for_flow(&hits, ""), "⏸✎⏱");
        assert_eq!(
            rule_marker_glyphs("⏸⛔↪✎⏱"),
            vec![
                Glyph::RuleBreak,
                Glyph::RuleBlock,
                Glyph::RuleMap,
                Glyph::RuleRewrite,
                Glyph::RuleTiming,
            ]
        );
        assert_eq!(
            rule_marker_glyphs("✎⏱"),
            vec![Glyph::RuleRewrite, Glyph::RuleTiming]
        );
        assert_eq!(rule_glyph_for_verb("break"), Glyph::RuleBreak);
        assert_eq!(rule_glyph_for_verb("block"), Glyph::RuleBlock);
        assert_eq!(rule_glyph_for_verb("map-local"), Glyph::RuleMap);
        assert_eq!(rule_glyph_for_verb("delay"), Glyph::RuleTiming);
        assert_eq!(rule_glyph_for_verb("replace"), Glyph::RuleRewrite);
        assert_eq!(icon::glyph_strip_width(0, 12.0, 3.5), 0.0);
        assert_eq!(icon::glyph_strip_width(1, 12.0, 3.5), 12.0);
        assert_eq!(icon::glyph_strip_width(2, 12.0, 3.5), 27.5);
        assert_eq!(icon::glyph_strip_width(3, 12.0, 3.5), 43.0);
        assert_eq!(
            rule_hover_summary(&hits, ""),
            "replace #3, [qa] break #2, delay #5"
        );
        assert_eq!(hits[0].format_badge(), "replace #3");
        assert_eq!(hits[1].format_badge(), "[qa] break #2");
        assert_eq!(
            session::RuleHit {
                group_name: None,
                line_no: 0,
                verb: "client-cert".into(),
            }
            .format_badge(),
            "client-cert"
        );
    }

    #[test]
    fn rule_line_hit_counts_and_filter_autocomplete_work() {
        let mut s = session::Session::default();
        let f1 = s.open(session::FlowDraft {
            protocol: session::Protocol::Http11,
            method: "GET".into(),
            host: "api.test".into(),
            path: "/a".into(),
            url: "https://api.test/a".into(),
            request_headers: vec![],
        });
        let f2 = s.open(session::FlowDraft {
            protocol: session::Protocol::Http11,
            method: "GET".into(),
            host: "api.test".into(),
            path: "/b".into(),
            url: "https://api.test/b".into(),
            request_headers: vec![],
        });
        s.set_rule_hit_details(
            f1,
            "replace",
            vec![session::RuleHit {
                group_name: None,
                line_no: 2,
                verb: "replace".into(),
            }],
        );
        s.set_rule_hit_details(
            f2,
            "replace map-local",
            vec![
                session::RuleHit {
                    group_name: None,
                    line_no: 2,
                    verb: "replace".into(),
                },
                session::RuleHit {
                    group_name: Some("Mock".into()),
                    line_no: 4,
                    verb: "map-local".into(),
                },
            ],
        );

        let facets = s.facets();
        let cat = FilterSuggestionsCatalog::from_flows(s.iter(), &facets);
        assert_eq!(cat.hits_for_rule_line(None, 2), 2);
        assert_eq!(cat.hits_for_rule_line(None, 1), 0);
        assert_eq!(cat.hits_for_rule_line(Some("mock"), 4), 1);
        assert_eq!(cat.hits_for_rule_line(Some("MOCK"), 4), 1);
        assert_eq!(cat.hits_for_rule_line(Some("mock"), 9), 0);

        // Filter syntax validation accepts `rule:`, `rule:2`, `rule:replace`, `-rule:4`
        assert!(validate_filter_syntax("rule: ").is_empty());
        assert!(validate_filter_syntax("rule:2 ").is_empty());
        assert!(validate_filter_syntax("rule:replace ").is_empty());
        assert!(validate_filter_syntax("-rule:mock:4 ").is_empty());

        // Autocomplete suggests `rule:` prefix and session-derived rule tokens
        let prefix_sugs = suggest_filter_completions("ru", &cat, Lang::En);
        assert!(prefix_sugs.iter().any(|item| item.insert_token == "rule:"));
        let val_sugs = suggest_filter_completions("rule:", &cat, Lang::En);
        assert!(val_sugs.iter().any(|item| item.label == "replace"));
        assert!(val_sugs.iter().any(|item| item.label == "2"));

        assert_eq!(rule_filter_token_for_line(None, 3), "rule:3");
        assert_eq!(rule_filter_token_for_line(Some("mock"), 2), "rule:mock:2");
    }
}
