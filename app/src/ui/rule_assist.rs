#[allow(unused_imports)]
use super::*;

#[derive(Clone, PartialEq)]
pub(crate) struct RuleLineItem {
    pub(crate) line_idx: usize,
    pub(crate) commented: bool,
    pub(crate) valid: bool,
    pub(crate) preview: String,
    pub(crate) raw: String,
}

pub(crate) fn collect_rule_lines(rules: &str) -> Vec<RuleLineItem> {
    rules
        .lines()
        .enumerate()
        .filter_map(|(line_idx, raw_line)| {
            let trimmed = raw_line.trim();
            if trimmed.is_empty() {
                return None;
            }
            let commented = trimmed.starts_with('#');
            let uncommented = trimmed.trim_start_matches('#').trim_start();
            let valid = if commented {
                !uncommented.is_empty() && crate::rules::parse_single_rule(uncommented).is_some()
            } else {
                crate::rules::parse_single_rule(trimmed).is_some()
            };
            let source = if commented { uncommented } else { trimmed };
            let preview = if source.is_empty() {
                trimmed.to_string()
            } else {
                source.to_string()
            };
            Some(RuleLineItem {
                line_idx,
                commented,
                valid,
                preview,
                raw: raw_line.to_string(),
            })
        })
        .collect()
}

/// Toggles a leading `# ` comment marker on the specified 0-based line index.
pub(crate) fn toggle_rule_line_comment(rules: &str, target_idx: usize) -> String {
    let mut lines: Vec<String> = rules.lines().map(str::to_string).collect();
    if let Some(line) = lines.get_mut(target_idx) {
        let trimmed_start = line.trim_start();
        let indent_len = line.len() - trimmed_start.len();
        let indent = &line[..indent_len];
        if let Some(rest) = trimmed_start.strip_prefix("# ") {
            *line = format!("{indent}{rest}");
        } else if let Some(rest) = trimmed_start.strip_prefix('#') {
            *line = format!("{indent}{}", rest.trim_start());
        } else if !trimmed_start.is_empty() {
            *line = format!("{indent}# {trimmed_start}");
        }
    }
    lines.join("\n")
}

/// Replaces the specified 0-based line index with `replacement`.
pub(crate) fn replace_rule_line(rules: &str, target_idx: usize, replacement: &str) -> String {
    let clean = replacement.trim();
    let mut lines: Vec<String> = rules.lines().map(str::to_string).collect();
    if let Some(slot) = lines.get_mut(target_idx) {
        *slot = clean.to_string();
        lines.join("\n")
    } else if rules.trim().is_empty() {
        clean.to_string()
    } else {
        format!("{rules}\n{clean}")
    }
}

/// Removes the specified 0-based line index from the rules text.
pub(crate) fn remove_rule_line(rules: &str, target_idx: usize) -> String {
    rules
        .lines()
        .enumerate()
        .filter(|(idx, _)| *idx != target_idx)
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn clean_draft_path(path_prefix: &str) -> String {
    if path_prefix == "/" || path_prefix == "*" {
        String::new()
    } else {
        path_prefix.to_string()
    }
}

fn format_bps_for_input(bps: u64) -> String {
    if (bps * 8).is_multiple_of(1000) {
        format!("{}kbps", (bps * 8) / 1000)
    } else {
        format!("{bps}bps")
    }
}

#[allow(clippy::too_many_arguments)]
/// Parses one rule line into the bottom 15-tab Quick Add Rule signals for quick editing.
pub(crate) fn fill_draft_from_rule_line(
    raw_line: &str,
    draft_kind: Signal<u8>,
    draft_host: Signal<String>,
    draft_path: Signal<String>,
    draft_op: Signal<String>,
    draft_from: Signal<String>,
    draft_to: Signal<String>,
    draft_modes: QuickRuleModeSignals,
) -> bool {
    let uncommented = raw_line.trim().trim_start_matches('#').trim_start();
    let Some(rule) = crate::rules::parse_single_rule(uncommented) else {
        return false;
    };
    use crate::rules::Rule;
    match rule {
        Rule::Replace {
            host,
            path_prefix,
            response,
            from,
            to,
            op,
        } => {
            draft_kind.set(0);
            draft_modes.req_res.set(response);
            draft_host.set(host);
            draft_path.set(clean_draft_path(&path_prefix));
            draft_op.set(op.unwrap_or_default());
            draft_from.set(from);
            draft_to.set(to);
        }
        Rule::MapRemote {
            host,
            path_prefix,
            url,
            op,
        } => {
            draft_kind.set(1);
            draft_host.set(host);
            draft_path.set(clean_draft_path(&path_prefix));
            draft_op.set(op.unwrap_or_default());
            draft_from.set(url);
            draft_to.set(String::new());
        }
        Rule::MapLocal {
            host,
            path_prefix,
            file,
            status,
            op,
            fallback,
            ..
        } => {
            draft_kind.set(2);
            draft_modes.local_fallback.set(fallback);
            draft_host.set(host);
            draft_path.set(clean_draft_path(&path_prefix));
            draft_op.set(op.unwrap_or_default());
            draft_from.set(file.map(|p| p.display().to_string()).unwrap_or_default());
            draft_to.set(status.map(|s| s.to_string()).unwrap_or_default());
        }
        Rule::Delay {
            host,
            path_prefix,
            delay,
            op,
        } => {
            draft_kind.set(3);
            draft_host.set(host);
            draft_path.set(clean_draft_path(&path_prefix));
            draft_op.set(op.unwrap_or_default());
            draft_from.set(format!("{}ms", delay.as_millis()));
            draft_to.set(String::new());
        }
        Rule::Break {
            host,
            path_prefix,
            op,
            cond,
        } => {
            draft_kind.set(4);
            draft_host.set(host);
            draft_path.set(clean_draft_path(&path_prefix));
            draft_op.set(op.unwrap_or_default());
            draft_from.set(cond.format_header_spec());
            draft_to.set(cond.body.unwrap_or_default());
        }
        Rule::BreakResponse {
            host,
            path_prefix,
            op,
            cond,
        } => {
            draft_kind.set(5);
            draft_host.set(host);
            draft_path.set(clean_draft_path(&path_prefix));
            draft_op.set(op.unwrap_or_default());
            draft_from.set(cond.format_header_spec());
            draft_to.set(cond.body.unwrap_or_default());
        }
        Rule::Header {
            host,
            path_prefix,
            response,
            remove,
            name,
            value,
            op,
        } => {
            draft_kind.set(6);
            draft_modes.req_res.set(response);
            draft_host.set(host);
            draft_path.set(clean_draft_path(&path_prefix));
            draft_op.set(op.unwrap_or_default());
            draft_from.set(name);
            draft_to.set(if remove { String::new() } else { value });
        }
        Rule::Query {
            host,
            path_prefix,
            remove,
            name,
            value,
            op,
        } => {
            draft_kind.set(7);
            draft_host.set(host);
            draft_path.set(clean_draft_path(&path_prefix));
            draft_op.set(op.unwrap_or_default());
            draft_from.set(name);
            draft_to.set(if remove { String::new() } else { value });
        }
        Rule::Cors { host, path_prefix } => {
            draft_kind.set(8);
            draft_host.set(host);
            draft_path.set(clean_draft_path(&path_prefix));
            draft_op.set(String::new());
            draft_from.set(String::new());
            draft_to.set(String::new());
        }
        Rule::Throttle {
            host,
            path_prefix,
            down_bps,
            up_bps,
        } => {
            draft_kind.set(9);
            draft_host.set(host);
            draft_path.set(clean_draft_path(&path_prefix));
            draft_op.set(String::new());
            draft_from.set(down_bps.map(format_bps_for_input).unwrap_or_default());
            draft_to.set(up_bps.map(format_bps_for_input).unwrap_or_default());
        }
        Rule::Cert {
            host,
            cert_path,
            key_path,
        } => {
            draft_kind.set(10);
            draft_modes.mitm_mtls.set(false);
            draft_host.set(host);
            draft_path.set(String::new());
            draft_op.set(String::new());
            draft_from.set(cert_path.display().to_string());
            draft_to.set(
                key_path
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            );
        }
        Rule::ClientCert {
            host,
            cert_path,
            key_path,
        } => {
            draft_kind.set(10);
            draft_modes.mitm_mtls.set(true);
            draft_host.set(host);
            draft_path.set(String::new());
            draft_op.set(String::new());
            draft_from.set(cert_path.display().to_string());
            draft_to.set(
                key_path
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            );
        }
        Rule::Json {
            host,
            path_prefix,
            response,
            remove,
            json_path,
            value,
            op,
        } => {
            draft_kind.set(11);
            draft_modes.req_res.set(response);
            draft_host.set(host);
            draft_path.set(clean_draft_path(&path_prefix));
            draft_op.set(op.unwrap_or_default());
            draft_from.set(json_path);
            draft_to.set(if remove {
                String::new()
            } else {
                serde_json::to_string(&value).unwrap_or_default()
            });
        }
        Rule::NoCache {
            host,
            path_prefix,
            op,
        } => {
            draft_kind.set(12);
            draft_host.set(host);
            draft_path.set(clean_draft_path(&path_prefix));
            draft_op.set(op.unwrap_or_default());
            draft_from.set(String::new());
            draft_to.set(String::new());
        }
        Rule::Block {
            host,
            path_prefix,
            drop_conn,
            status,
            op,
        } => {
            draft_kind.set(13);
            draft_modes.status_drop.set(drop_conn);
            draft_host.set(host);
            draft_path.set(clean_draft_path(&path_prefix));
            draft_op.set(op.unwrap_or_default());
            draft_from.set(if drop_conn {
                String::new()
            } else {
                status.to_string()
            });
            draft_to.set(String::new());
        }
        Rule::Resolve {
            host,
            target_host,
            port,
        } => {
            draft_kind.set(14);
            draft_host.set(host);
            draft_path.set(String::new());
            draft_op.set(String::new());
            draft_from.set(target_host);
            draft_to.set(port.map(|p| p.to_string()).unwrap_or_default());
        }
        Rule::Hook {
            host,
            path_prefix,
            url,
            phase,
            mode,
            timeout,
            op,
        } => {
            use crate::rules::{HookMode, HookPhase};
            draft_kind.set(15);
            draft_modes
                .req_res
                .set(matches!(phase, HookPhase::Response));
            draft_host.set(host);
            draft_path.set(clean_draft_path(&path_prefix));
            draft_op.set(op.unwrap_or_default());
            draft_from.set(url);
            let mut to_parts = Vec::new();
            if matches!(phase, HookPhase::Both) {
                to_parts.push("both".to_string());
            }
            if matches!(mode, HookMode::Preview) {
                to_parts.push("preview".to_string());
            }
            let ms = timeout.as_millis();
            if ms != 2000 {
                to_parts.push(format!("{ms}ms"));
            }
            draft_to.set(to_parts.join(" "));
        }
        Rule::Proto { .. } => return false,
    }
    true
}

pub(crate) fn rule_filter_token_for_line(group_name: Option<&str>, line_no: usize) -> String {
    match group_name.map(str::trim).filter(|g| !g.is_empty()) {
        Some(group) if group.chars().any(char::is_whitespace) || group.contains('"') => {
            let escaped = group.replace('"', "\\\"");
            format!("rule:\"{escaped}\":{line_no}")
        }
        Some(group) => format!("rule:{group}:{line_no}"),
        None => format!("rule:{line_no}"),
    }
}

fn format_rule_line_summary(
    items: &[RuleLineItem],
    group: Option<&str>,
    catalog: &FilterSuggestionsCatalog,
    lang: Lang,
) -> (String, bool) {
    let mut active = 0usize;
    let mut commented = 0usize;
    let mut invalid = 0usize;
    let mut total_hits = 0usize;
    for item in items {
        if item.commented {
            commented += 1;
        } else if item.valid {
            active += 1;
            total_hits += catalog.hits_for_rule_line(group, item.line_idx + 1);
        } else {
            invalid += 1;
        }
    }
    let active_s = present::format_thousands(active as u64);
    let hits_s = present::format_thousands(total_hits as u64);
    let mut text = match lang {
        Lang::Zh => format!("{active_s} 条规则 · 命中 {hits_s} 次"),
        Lang::En => {
            let r_word = if active == 1 { "rule" } else { "rules" };
            let h_word = if total_hits == 1 { "hit" } else { "hits" };
            format!("{active_s} {r_word} · {hits_s} {h_word}")
        }
    };
    if commented > 0 {
        let com_s = present::format_thousands(commented as u64);
        match lang {
            Lang::Zh => text.push_str(&format!(" · {com_s} 条已注释")),
            Lang::En => text.push_str(&format!(" · {com_s} disabled")),
        }
    }
    if invalid > 0 {
        let inv_s = present::format_thousands(invalid as u64);
        match lang {
            Lang::Zh => text.push_str(&format!(" · {inv_s} 条语法有误")),
            Lang::En => text.push_str(&format!(" · {inv_s} invalid")),
        }
    }
    (text, invalid > 0)
}

fn rule_line_summary_bar(
    target: RuleEditTarget,
    catalog: Signal<FilterSuggestionsCatalog>,
    manual_open: Signal<bool>,
    show_chips: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    let langs = ui_langs();
    let parsed_lines = computed(move || collect_rule_lines(&target.current_text()));
    view()
        .w_full()
        .h(px(22.0))
        .px_1()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            text(move || {
                let group = target.editing.get();
                let (summary, _) = parsed_lines.with(|items| {
                    catalog.with(|cat| {
                        format_rule_line_summary(items, group.as_deref(), cat, ui_lang(langs))
                    })
                });
                SharedString::from(summary)
            })
            .number_of_lines(1)
            .text_overflow(EllipsizeMode::Tail)
            .tabular_nums()
            .text_size(px(TEXT_XS))
            .text_color(move || {
                let has_err =
                    parsed_lines.with(|items| items.iter().any(|it| !it.commented && !it.valid));
                if has_err {
                    status_warn_fg()
                } else {
                    fg_muted()
                }
            }),
        )
        .child(
            button()
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
                .duration(move || motion_ms(MOTION_INSTANT_MS))
                .ease_out()
                .focus_visible(apply_focus_ring)
                .on_press(move |_: &()| {
                    if show_chips() {
                        manual_open.set(false);
                        target.focused_line.set(None);
                    } else {
                        manual_open.set(true);
                    }
                })
                .child(
                    text(move || {
                        if show_chips() {
                            tr("收起", "Hide")()
                        } else {
                            tr("管理单条", "Manage")()
                        }
                    })
                    .number_of_lines(1)
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg_secondary()),
                )
                .child(
                    view()
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .rotate(move || if show_chips() { deg(180.0) } else { deg(0.0) })
                        .transition_transform()
                        .duration(move || motion_ms(MOTION_INSTANT_MS))
                        .ease_bezier(0.22, 1.0, 0.36, 1.0)
                        .child(glyph_px(9.0, || Glyph::Chevron, || color(fg_muted()))),
                ),
        )
}

fn rule_chip_toggle_btn(
    item: ReadSignal<RuleLineItem>,
    target: RuleEditTarget,
    controls: Arc<Controls>,
    is_zero_hit: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    view()
        .flex_shrink_0()
        .tooltip(label_tip(
            "启用 / 注释此规则（#）",
            "Enable / comment this rule (#)",
        ))
        .child(
            toggle()
                .pressed(move || item.with(|it| !it.commented))
                .accessibility_label(tr_opt("启用或注释规则", "Toggle rule comment"))
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
                .on_change(move |_: &bool| {
                    let idx = item.with(|it| it.line_idx);
                    let next = toggle_rule_line_comment(&target.current_text_untracked(), idx);
                    target.commit(&controls, next);
                })
                .child(
                    mono(move || -> SharedString {
                        if item.with(|it| it.commented) {
                            "#".into()
                        } else {
                            "✓".into()
                        }
                    })
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::BOLD)
                    .text_color(move || {
                        if item.with(|it| it.commented) || is_zero_hit() {
                            fg_muted()
                        } else {
                            accent_blue_text()
                        }
                    }),
                ),
        )
}

fn fill_line_into_quick_editor(
    raw: &str,
    line_idx: usize,
    target: RuleEditTarget,
    draft: QuickRuleDraftSignals,
    quick_add_open: Signal<bool>,
    controls: &Controls,
) {
    target.focused_line.set(None);
    let uncommented = raw.trim().trim_start_matches('#').trim_start();
    if matches!(
        crate::rules::parse_single_rule(uncommented),
        Some(crate::rules::Rule::Proto { .. })
    ) {
        set_status(controls, StatusMsg::ProtoEditInEditor);
        return;
    }
    if fill_draft_from_rule_line(
        raw,
        draft.kind,
        draft.host,
        draft.path,
        draft.op,
        draft.from,
        draft.to,
        draft.modes,
    ) {
        draft.editing_line.set(Some(line_idx));
        quick_add_open.set(true);
    }
}

fn rule_chip_label(
    item: ReadSignal<RuleLineItem>,
    target: RuleEditTarget,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
    quick_add_open: Signal<bool>,
    is_zero_hit: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    let langs = ui_langs();
    let is_proto = move || {
        item.with(|it| {
            let uncommented = it.raw.trim().trim_start_matches('#').trim_start();
            matches!(
                crate::rules::parse_single_rule(uncommented),
                Some(crate::rules::Rule::Proto { .. })
            )
        })
    };
    view()
        .flex_1()
        .min_w(px(0.0))
        .tooltip(
            tooltip().child(ui_tooltip_surface(
                text(move || -> SharedString {
                    let lang = ui_lang(langs);
                    if item.with(|it| !it.commented && !it.valid) {
                        t(
                            lang,
                            "语法有误，请在上方文本编辑器中修改",
                            "Invalid syntax — edit in text editor above",
                        )
                    } else if is_proto() {
                        t(
                            lang,
                            "请在上方纯文本编辑器中修改 proto 规则",
                            "Edit proto rules in text editor above",
                        )
                    } else {
                        t(
                            lang,
                            "点击回填到下方表单原地编辑此行",
                            "Fill rule into quick editor to update this line",
                        )
                    }
                    .into()
                })
                .text_size(px(TEXT_SM))
                .text_color(fg_primary()),
            )),
        )
        .child(
            button()
                .w_full()
                .min_w(px(0.0))
                .rounded(px(RADIUS_SM))
                .hover(|s| s.bg(bg_control_hover()))
                .active(|s| s.bg(bg_control_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .on_press(move |_: &()| {
                    let (raw, line_idx) = item.with(|it| (it.raw.clone(), it.line_idx));
                    fill_line_into_quick_editor(
                        &raw,
                        line_idx,
                        target,
                        draft,
                        quick_add_open,
                        &controls,
                    );
                })
                .child(
                    mono(move || -> SharedString {
                        item.with(|it| format!("#{} {}", it.line_idx + 1, it.preview))
                            .into()
                    })
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail)
                    .text_size(px(TEXT_XS))
                    .text_color(move || {
                        if item.with(|it| it.commented) || is_zero_hit() {
                            fg_muted()
                        } else if item.with(|it| !it.valid) {
                            status_err_fg()
                        } else {
                            fg_primary()
                        }
                    }),
                ),
        )
}

fn rule_chip_hit_badge(
    item: ReadSignal<RuleLineItem>,
    target: RuleEditTarget,
    hit_count: impl Fn() -> usize + Copy + 'static,
) -> impl IntoElement {
    let langs = ui_langs();
    when(
        move || item.with(|it| !it.commented && it.valid),
        move || {
            view()
                .flex_shrink_0()
                .tooltip(
                    tooltip().child(ui_tooltip_surface(
                        text(move || -> SharedString {
                            let lang = ui_lang(langs);
                            if hit_count() > 0 {
                                t(
                                    lang,
                                    "本会话命中次数（点击关闭面板并按此规则行过滤）",
                                    "Session hits (click to filter by this rule line)",
                                )
                            } else {
                                t(lang, "本会话暂无命中", "0 hits in current session")
                            }
                            .into()
                        })
                        .text_size(px(TEXT_SM))
                        .text_color(fg_primary()),
                    )),
                )
                .child(
                    button()
                        .px_1()
                        .h(px(15.0))
                        .rounded(px(RADIUS_SM))
                        .items_center()
                        .justify_center()
                        .hover(|s| {
                            s.bg(move || {
                                if hit_count() > 0 {
                                    bg_control_hover()
                                } else {
                                    AdaptiveColor::TRANSPARENT
                                }
                            })
                        })
                        .active(|s| s.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .disabled(move || hit_count() == 0)
                        .disabled_style(|s| s.opacity(0.45))
                        .on_press(move |_: &()| {
                            if hit_count() == 0 {
                                return;
                            }
                            let line_no = item.with(|it| it.line_idx + 1);
                            let group = target.editing.get_untracked();
                            let token = rule_filter_token_for_line(group.as_deref(), line_no);
                            target.filter.set(token);
                            target.filter_enabled.set(true);
                            target.panel_open.set(false);
                        })
                        .child(
                            mono(move || -> SharedString {
                                let count = hit_count();
                                if count > 0 {
                                    format!("×{}", present::format_thousands(count as u64)).into()
                                } else {
                                    "0".into()
                                }
                            })
                            .tabular_nums()
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || {
                                if hit_count() > 0 {
                                    accent_blue_text()
                                } else {
                                    fg_muted()
                                }
                            }),
                        ),
                )
        },
    )
}

fn rule_chip_remove_btn(
    item: ReadSignal<RuleLineItem>,
    target: RuleEditTarget,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let confirm = signal(false);
    let reset_job = stored(None::<Spawned<()>>);
    view()
        .flex_shrink_0()
        .tooltip(label_tip(
            "删除此规则行（需二次确认）",
            "Remove rule line (click twice)",
        ))
        .child(
            button()
                .h(px(18.0))
                .min_w(px(18.0))
                .px(move || if confirm.get() { px(4.0) } else { px(0.0) })
                .rounded(px(RADIUS_SM))
                .items_center()
                .justify_center()
                .bg(move || {
                    if confirm.get() {
                        status_err_bg()
                    } else {
                        AdaptiveColor::TRANSPARENT
                    }
                })
                .hover(|s| {
                    s.bg(move || {
                        if confirm.get() {
                            status_err_bg_hover()
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
                .accessibility_label(move || -> Option<SharedString> {
                    let langs = ui_langs();
                    Some(
                        if confirm.get() {
                            t(
                                ui_lang(langs),
                                "再次点击确认删除规则行",
                                "Click again to confirm rule removal",
                            )
                        } else {
                            t(ui_lang(langs), "删除规则行", "Remove rule line")
                        }
                        .into(),
                    )
                })
                .on_press(move |_: &()| {
                    if confirm.get_untracked() {
                        reset_job.replace(None);
                        confirm.set(false);
                        let idx = item.with(|it| it.line_idx);
                        let next = remove_rule_line(&target.current_text_untracked(), idx);
                        target.commit(&controls, next);
                    } else {
                        confirm.set(true);
                        reset_job.replace(Some(set_timeout(
                            Duration::from_millis(3500),
                            move || {
                                confirm.set(false);
                            },
                        )));
                    }
                })
                .child(
                    text(move || -> SharedString {
                        let langs = ui_langs();
                        if confirm.get() {
                            t(ui_lang(langs), "删?", "Del?").into()
                        } else {
                            "×".into()
                        }
                    })
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || {
                        if confirm.get() {
                            status_err_fg()
                        } else {
                            fg_muted()
                        }
                    }),
                ),
        )
}

fn rule_line_chip(
    item: ReadSignal<RuleLineItem>,
    target: RuleEditTarget,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
    quick_add_open: Signal<bool>,
    catalog: Signal<FilterSuggestionsCatalog>,
) -> impl IntoElement {
    let tc = controls.clone();
    let lc = controls.clone();
    let rc = controls;
    let hit_count = move || {
        let line_no = item.with(|it| it.line_idx + 1);
        let group = target.editing.get();
        catalog.with(|cat| cat.hits_for_rule_line(group.as_deref(), line_no))
    };
    let is_zero_hit = move || item.with(|it| !it.commented && it.valid) && hit_count() == 0;
    view()
        .h(px(ControlSize::Xs.height()))
        .max_w(px(300.0))
        .pl_1p5()
        .pr_1()
        .rounded(px(RADIUS_SM))
        .border_1()
        .border_color(move || {
            let line_no = item.with(|it| it.line_idx + 1);
            if target.is_line_focused(line_no) {
                status_warn_fg()
            } else if item.with(|it| !it.commented && !it.valid) {
                status_err_fg()
            } else if item.with(|it| it.commented) || is_zero_hit() {
                border_control()
            } else {
                accent_blue_border()
            }
        })
        .bg(move || {
            let line_no = item.with(|it| it.line_idx + 1);
            if target.is_line_focused(line_no) {
                status_warn_bg()
            } else if item.with(|it| it.commented) || is_zero_hit() {
                bg_control()
            } else if item.with(|it| !it.valid) {
                status_err_bg()
            } else {
                accent_blue_soft()
            }
        })
        .opacity(move || if is_zero_hit() { 0.72_f32 } else { 1.0_f32 })
        .flex_row()
        .items_center()
        .gap_1()
        .child(rule_chip_toggle_btn(item, target, tc, is_zero_hit))
        .child(rule_chip_label(
            item,
            target,
            lc,
            draft,
            quick_add_open,
            is_zero_hit,
        ))
        .child(rule_chip_hit_badge(item, target, hit_count))
        .child(rule_chip_remove_btn(item, target, rc))
}

fn rule_focused_banner(
    target: RuleEditTarget,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
    quick_add_open: Signal<bool>,
) -> impl IntoElement {
    let langs = ui_langs();
    let focused_entry = computed(move || {
        collect_rule_lines(&target.current_text())
            .into_iter()
            .find(|it| target.is_line_focused(it.line_idx + 1))
            .map(|it| (it.line_idx, it.raw))
    });
    when(
        move || focused_entry.with(Option::is_some),
        move || {
            let edit_controls = controls.clone();
            view()
                .w_full()
                .px_2()
                .py_1()
                .rounded(px(RADIUS_SM))
                .border_1()
                .border_color(status_warn_fg())
                .bg(status_warn_bg())
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text(move || -> SharedString {
                        let n =
                            focused_entry.with(|opt| opt.as_ref().map(|(i, _)| i + 1).unwrap_or(0));
                        match ui_lang(langs) {
                            Lang::Zh => format!("已定位到第 {n} 行（已在下方置顶高亮）"),
                            Lang::En => format!("Focused rule line #{n} (pinned below)"),
                        }
                        .into()
                    })
                    .flex_1()
                    .min_w(px(0.0))
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail)
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(status_warn_fg()),
                )
                .child(
                    view()
                        .flex_shrink_0()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(
                            button()
                                .h(px(ControlSize::Xs.height()))
                                .px_1p5()
                                .rounded(px(RADIUS_SM))
                                .hover(|s| s.bg(status_warn_bg_hover()))
                                .active(|s| s.bg(bg_control_active()))
                                .focus_visible(apply_focus_ring)
                                .items_center()
                                .justify_center()
                                .on_press(move |_: &()| {
                                    if let Some((idx, raw)) = focused_entry.get() {
                                        fill_line_into_quick_editor(
                                            &raw,
                                            idx,
                                            target,
                                            draft,
                                            quick_add_open,
                                            &edit_controls,
                                        );
                                    }
                                })
                                .child(
                                    text(tr("编辑此行", "Edit line"))
                                        .text_size(px(TEXT_XS))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(status_warn_fg()),
                                ),
                        )
                        .child(
                            button()
                                .w(px(ControlSize::Xs.height()))
                                .h(px(ControlSize::Xs.height()))
                                .rounded(px(RADIUS_SM))
                                .hover(|s| s.bg(status_warn_bg_hover()))
                                .active(|s| s.bg(bg_control_active()))
                                .focus_visible(apply_focus_ring)
                                .items_center()
                                .justify_center()
                                .accessibility_label(tr_opt(
                                    "清除定位高亮",
                                    "Clear focus highlight",
                                ))
                                .on_press(move |_: &()| target.focused_line.set(None))
                                .child(
                                    text("×")
                                        .text_size(px(TEXT_XS))
                                        .text_color(status_warn_fg()),
                                ),
                        ),
                )
        },
    )
}

pub(crate) fn rule_line_strip(
    target: RuleEditTarget,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
    quick_add_open: Signal<bool>,
) -> impl IntoElement {
    let catalog = filter_catalog_context();
    let manual_open = signal(false);
    let show_chips = move || manual_open.get() || target.focused_line.with(Option::is_some);
    let items = move || {
        let mut all = collect_rule_lines(&target.current_text());
        all.sort_by_key(|it| !target.is_line_focused(it.line_idx + 1));
        all
    };
    when(
        move || !items().is_empty(),
        move || {
            let banner_controls = controls.clone();
            let strip_controls = controls.clone();
            view()
                .w_full()
                .gap_1p5()
                .flex_col()
                .child(rule_line_summary_bar(
                    target,
                    catalog,
                    manual_open,
                    show_chips,
                ))
                .child(rule_focused_banner(
                    target,
                    banner_controls,
                    draft,
                    quick_add_open,
                ))
                .child(when(show_chips, move || {
                    let item_controls = strip_controls.clone();
                    scroll().w_full().max_h(px(76.0)).child(
                        view().w_full().flex_row().flex_wrap().gap_1().child(each(
                            items,
                            |item| item.line_idx,
                            move |item, _| {
                                rule_line_chip(
                                    item,
                                    target,
                                    item_controls.clone(),
                                    draft,
                                    quick_add_open,
                                    catalog,
                                )
                            },
                        )),
                    )
                }))
        },
    )
}
