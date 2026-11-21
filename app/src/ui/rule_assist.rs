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

#[allow(clippy::too_many_arguments)]
pub(crate) fn rule_filter_token_for_line(group_name: Option<&str>, line_no: usize) -> String {
    match group_name {
        Some(group) if !group.trim().is_empty() => format!("rule:{group}:{line_no}"),
        _ => format!("rule:{line_no}"),
    }
}

pub(crate) fn rule_line_strip(
    target: RuleEditTarget,
    controls: Arc<Controls>,
    draft_host: Signal<String>,
    draft_path: Signal<String>,
    draft_op: Signal<String>,
    draft_from: Signal<String>,
    draft_to: Signal<String>,
    draft_modes: QuickRuleModeSignals,
    draft_kind: Signal<u8>,
) -> impl IntoElement {
    let catalog = filter_catalog_context();
    let items = move || collect_rule_lines(&target.current_text());
    when(
        move || !items().is_empty(),
        move || {
            let toggle_controls = controls.clone();
            let remove_controls = controls.clone();
            scroll().w_full().max_h(px(62.0)).child(
                view().w_full().flex_row().flex_wrap().gap_1().child(each(
                    items,
                    |item| item.line_idx,
                    move |item, _| {
                        let tc = toggle_controls.clone();
                        let rc = remove_controls.clone();
                        let hit_count = move || {
                            let line_no = item.with(|it| it.line_idx + 1);
                            let group = target.editing.get();
                            catalog.with(|cat| cat.hits_for_rule_line(group.as_deref(), line_no))
                        };
                        let is_zero_hit =
                            move || item.with(|it| !it.commented && it.valid) && hit_count() == 0;
                        view()
                            .h(px(22.0))
                            .max_w(px(300.0))
                            .pl_1p5()
                            .pr_1()
                            .rounded(px(5.0))
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
                            .child(
                                view()
                                    .w(px(14.0))
                                    .h(px(14.0))
                                    .flex_shrink_0()
                                    .rounded(px(3.0))
                                    .items_center()
                                    .justify_center()
                                    .cursor(Cursor::Pointer)
                                    .hover(|s| s.bg(bg_control_hover()))
                                    .tooltip(label_tip(
                                        "启用 / 注释此规则 (#)",
                                        "Enable / comment this rule (#)",
                                    ))
                                    .on_press(move || {
                                        let idx = item.with(|it| it.line_idx);
                                        let next = toggle_rule_line_comment(
                                            &target.current_text_untracked(),
                                            idx,
                                        );
                                        target.commit(&tc, next);
                                    })
                                    .child(
                                        mono(move || -> SharedString {
                                            if item.with(|it| it.commented) {
                                                "#".into()
                                            } else {
                                                "✓".into()
                                            }
                                        })
                                        .text_size(px(10.0))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(
                                            move || {
                                                if item.with(|it| it.commented) || is_zero_hit() {
                                                    fg_muted()
                                                } else {
                                                    accent_blue_text()
                                                }
                                            },
                                        ),
                                    ),
                            )
                            .child(
                                view()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .cursor(Cursor::Pointer)
                                    .tooltip(label_tip(
                                        "点击回填到下方表单",
                                        "Fill rule into quick editor below",
                                    ))
                                    .on_press(move || {
                                        target.focused_line.set(None);
                                        let raw = item.with(|it| it.raw.clone());
                                        fill_draft_from_rule_line(
                                            &raw,
                                            draft_kind,
                                            draft_host,
                                            draft_path,
                                            draft_op,
                                            draft_from,
                                            draft_to,
                                            draft_modes,
                                        );
                                    })
                                    .child(
                                        mono(move || -> SharedString {
                                            item.with(|it| {
                                                format!("#{} {}", it.line_idx + 1, it.preview)
                                            })
                                            .into()
                                        })
                                        .number_of_lines(1)
                                        .text_overflow(EllipsizeMode::Tail)
                                        .text_size(px(10.5))
                                        .text_color(
                                            move || {
                                                if item.with(|it| it.commented) || is_zero_hit() {
                                                    fg_muted()
                                                } else if item.with(|it| !it.valid) {
                                                    status_err_fg()
                                                } else {
                                                    fg_primary()
                                                }
                                            },
                                        ),
                                    ),
                            )
                            .child(when(
                                move || item.with(|it| !it.commented && it.valid),
                                move || {
                                    view()
                                        .px_1()
                                        .h(px(15.0))
                                        .flex_shrink_0()
                                        .rounded(px(3.0))
                                        .items_center()
                                        .justify_center()
                                        .cursor(Cursor::Pointer)
                                        .hover(|s| s.bg(bg_control_hover()))
                                        .tooltip(label_tip(
                                            "本会话命中次数（点击按此规则行过滤）",
                                            "Session hit count (click to filter by this rule line)",
                                        ))
                                        .on_press(move || {
                                            let line_no = item.with(|it| it.line_idx + 1);
                                            let group = target.editing.get_untracked();
                                            let token = rule_filter_token_for_line(
                                                group.as_deref(),
                                                line_no,
                                            );
                                            target.filter.set(token);
                                            target.filter_enabled.set(true);
                                        })
                                        .child(
                                            mono(move || -> SharedString {
                                                let count = hit_count();
                                                if count > 0 {
                                                    format!("×{count}").into()
                                                } else {
                                                    "0".into()
                                                }
                                            })
                                            .tabular_nums()
                                            .text_size(px(9.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || {
                                                if hit_count() > 0 {
                                                    accent_blue_text()
                                                } else {
                                                    fg_muted()
                                                }
                                            }),
                                        )
                                },
                            ))
                            .child(
                                view()
                                    .w(px(14.0))
                                    .h(px(14.0))
                                    .flex_shrink_0()
                                    .rounded(px(3.0))
                                    .items_center()
                                    .justify_center()
                                    .cursor(Cursor::Pointer)
                                    .hover(|s| s.bg(bg_control_hover()))
                                    .tooltip(label_tip("删除此规则行", "Remove rule line"))
                                    .on_press(move || {
                                        let idx = item.with(|it| it.line_idx);
                                        let next =
                                            remove_rule_line(&target.current_text_untracked(), idx);
                                        target.commit(&rc, next);
                                    })
                                    .child(text("×").text_size(px(10.5)).text_color(fg_muted())),
                            )
                    },
                )),
            )
        },
    )
}
