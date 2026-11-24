//! Unified `+ Add rule` menu combining common examples and form-backed rule builders (E4).

#[allow(unused_imports)]
use super::*;

/// A single built-in rule snippet that can either be appended directly or pre-filled into the form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RuleTemplate {
    pub(crate) id: &'static str,
    pub(crate) open_in_form: bool,
    pub(crate) label_zh: &'static str,
    pub(crate) label_en: &'static str,
    pub(crate) desc_zh: &'static str,
    pub(crate) desc_en: &'static str,
    pub(crate) rule_line: &'static str,
}

impl RuleTemplate {
    pub(crate) fn label(self, lang: Lang) -> &'static str {
        t(lang, self.label_zh, self.label_en)
    }

    pub(crate) fn description(self, lang: Lang) -> &'static str {
        t(lang, self.desc_zh, self.desc_en)
    }
}

/// Everyday examples shown at the top of `+ Add rule`.
/// Zero-config rules append immediately; parameterized examples pre-fill and open the form.
pub(crate) const QUICK_RULE_PRESETS: &[RuleTemplate] = &[
    RuleTemplate {
        id: "dev-cors",
        open_in_form: false,
        label_zh: "开发期 CORS",
        label_en: "Dev CORS",
        desc_zh: "一键为全部请求补全跨域响应头",
        desc_en: "Inject permissive CORS headers for all hosts",
        rule_line: "cors *",
    },
    RuleTemplate {
        id: "no-cache",
        open_in_form: false,
        label_zh: "禁用缓存",
        label_en: "No cache",
        desc_zh: "一键清除条件请求头并强制 no-cache",
        desc_en: "Strip conditional headers and force no-cache",
        rule_line: "no-cache *",
    },
    RuleTemplate {
        id: "slow-endpoint",
        open_in_form: false,
        label_zh: "慢接口 2s",
        label_en: "Slow 2s",
        desc_zh: "一键为全部请求注入 2000ms 延迟",
        desc_en: "Add 2000ms latency to all requests",
        rule_line: "delay * 2000ms",
    },
    RuleTemplate {
        id: "throttle-3g",
        open_in_form: false,
        label_zh: "弱网限速 (3G)",
        label_en: "Throttle (3G)",
        desc_zh: "一键按 3G 带宽上限模拟弱网环境",
        desc_en: "Simulate a slow mobile connection using 3G limits",
        rule_line: "throttle * 3g",
    },
    RuleTemplate {
        id: "force-500",
        open_in_form: true,
        label_zh: "强制 500",
        label_en: "Force 500",
        desc_zh: "回填表单并指定域名，模拟服务端 500 异常",
        desc_en: "Pre-fill form to return HTTP 500 for an endpoint",
        rule_line: "map-local api.example.com/v1 status=500",
    },
    RuleTemplate {
        id: "rewrite-base-url",
        open_in_form: true,
        label_zh: "替换 baseURL",
        label_en: "Rewrite baseURL",
        desc_zh: "回填表单将线上接口前缀转发到本地服务",
        desc_en: "Pre-fill form to forward an API prefix to localhost",
        rule_line: "map-remote api.example.com/v1 http://127.0.0.1:3000/v1",
    },
    RuleTemplate {
        id: "hmac-signature",
        open_in_form: true,
        label_zh: "HMAC 签名头",
        label_en: "HMAC signature",
        desc_zh: "回填表单按请求正文动态计算 HMAC-SHA256 签名",
        desc_en: "Pre-fill form with dynamic HMAC-SHA256 header signing",
        rule_line: r#"header api.example.com/v1 req set X-Signature {{hmac_sha256("dev-secret", req.body)}}"#,
    },
];

/// Protobuf/gRPC schema mount template appended directly under Advanced (no quick form).
pub(crate) const PROTO_RULE_TEMPLATE: RuleTemplate = RuleTemplate {
    id: "proto",
    open_in_form: false,
    label_zh: "Protobuf 解码",
    label_en: "Protobuf schema",
    desc_zh: "挂载 .proto 文件解析二进制消息体",
    desc_en: "Mount a .proto schema for binary decoding",
    rule_line: "proto api.example.com/grpc /path/to/schema.proto",
};

/// Returns a valid single-line example rule for the selected form kind (`0..=15`).
pub(crate) fn quick_rule_example(kind: u8) -> &'static str {
    match kind {
        0 => r#"replace api.example.com/v1 res "old" -> "new""#,
        1 => "map-remote api.example.com/v1 http://127.0.0.1:3000/v1",
        2 => "map-local api.example.com/v1 status=500",
        3 => "delay api.example.com/v1 2000ms",
        4 => "break api.example.com/v1 header=X-Debug:1",
        5 => "break-res api.example.com/v1 body=error",
        6 => {
            r#"header api.example.com/v1 req set X-Signature {{hmac_sha256("dev-secret", req.body)}}"#
        }
        7 => "query api.example.com/v1 set debug=1",
        8 => "cors api.example.com/v1",
        9 => "throttle api.example.com/v1 3g",
        10 => "cert api.example.com /path/to/cert.pem /path/to/key.pem",
        11 => "json api.example.com/v1 res set /data/enabled true",
        12 => "no-cache api.example.com/v1",
        13 => "block ads.example.com 403",
        14 => "resolve api.example.com 127.0.0.1 8080",
        15 => "hook api.example.com/v1 http://127.0.0.1:9000/hook",
        _ => r#"replace api.example.com/v1 res "old" -> "new""#,
    }
}

/// Appends `rule_line` to `current` rules text without introducing extra blank lines.
pub(crate) fn append_template_rule(current: &str, rule_line: &str) -> String {
    let trimmed_line = rule_line.trim();
    if trimmed_line.is_empty() {
        return current.to_string();
    }
    let trimmed_current = current.trim_end_matches(['\r', '\n']);
    if trimmed_current.trim().is_empty() {
        trimmed_line.to_string()
    } else {
        format!("{trimmed_current}\n{trimmed_line}")
    }
}

fn rule_templates_menu_trigger(open: Signal<bool>) -> impl IntoElement {
    view()
        .role(AccessibilityRole::Button)
        .h(px(ControlSize::Xs.height()))
        .px_1p5()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_1()
        .rounded(px(RADIUS_SM))
        .bg(move || {
            if open.get() {
                accent_blue_soft()
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .hover(|style| {
            style.bg(move || {
                if open.get() {
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
        .tooltip(label_tip(
            "插入常用示例或按类型配置规则",
            "Use a common example or configure a rule by type",
        ))
        .child(
            text(tr("+ 添加规则", "+ Add rule"))
                .number_of_lines(1)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::MEDIUM)
                .text_color(accent_blue_text()),
        )
        .child(glyph_px(
            10.0,
            || Glyph::Chevron,
            || color(accent_blue_text()),
        ))
}

fn rule_templates_menu_body(
    target: RuleEditTarget,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
    quick_add_open: Signal<bool>,
    open: Signal<bool>,
) -> impl IntoElement {
    let langs = ui_langs();
    let last_group = QUICK_KIND_GROUPS.len().saturating_sub(1);
    let preset_controls = controls.clone();
    view().w(px(330.0)).overflow_hidden().child(
        scroll().w_full().max_h(px(360.0)).child(
            view()
                .w_full()
                .p_1p5()
                .gap_0p5()
                .flex_col()
                .child(
                    text(tr("常用示例", "Common Examples"))
                        .px_2()
                        .pt_0p5()
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg_muted()),
                )
                .children(QUICK_RULE_PRESETS.iter().copied().map(move |tpl| {
                    rule_preset_menu_item(
                        tpl,
                        target,
                        preset_controls.clone(),
                        draft,
                        quick_add_open,
                        open,
                    )
                }))
                .child(view().w_full().h(px(1.0)).my_1().bg(border_subtle()))
                .child(
                    text(tr("按类型配置", "Build by Type"))
                        .px_2()
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg_muted()),
                )
                .children(QUICK_KIND_GROUPS.iter().copied().enumerate().map(
                    move |(idx, group)| {
                        let proto_ctx =
                            (idx == last_group).then_some((target, controls.clone(), langs));
                        rule_kind_form_group_section(group, draft, quick_add_open, open, proto_ctx)
                    },
                )),
        ),
    )
}

/// Single unified `+ Add rule ▾` menu: common examples on top, typed form builders below.
pub(crate) fn rule_templates_menu(
    target: RuleEditTarget,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
    quick_add_open: Signal<bool>,
) -> impl IntoElement {
    let open = signal(false);
    menu()
        .open(move || Some(open.get()))
        .on_open_change(move |event: &nsh_base::MenuEvent| {
            if let nsh_base::MenuEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(4.0))
        .margin(px(8.0))
        .popup_style(ui_popup_style)
        .popup_starting_style(ui_popup_starting_style)
        .trigger(rule_templates_menu_trigger(open))
        .child(rule_templates_menu_body(
            target,
            controls,
            draft,
            quick_add_open,
            open,
        ))
}

fn rule_preset_menu_item(
    tpl: RuleTemplate,
    target: RuleEditTarget,
    controls: Arc<Controls>,
    draft: QuickRuleDraftSignals,
    quick_add_open: Signal<bool>,
    open: Signal<bool>,
) -> impl IntoElement {
    let langs = ui_langs();
    menu_item()
        .w_full()
        .px_2()
        .py_1()
        .rounded(px(RADIUS_MD))
        .gap_0p5()
        .flex_col()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .on_select(move || {
            if tpl.open_in_form {
                fill_draft_from_rule_line(
                    tpl.rule_line,
                    draft.kind,
                    draft.host,
                    draft.path,
                    draft.op,
                    draft.from,
                    draft.to,
                    draft.modes,
                );
                quick_add_open.set(true);
            } else {
                let next = append_template_rule(&target.current_text_untracked(), tpl.rule_line);
                target.commit(&controls, next);
            }
            open.set(false);
        })
        .child(
            view()
                .w_full()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text(move || SharedString::from(tpl.label(ui_lang(langs))))
                        .flex_shrink_0()
                        .number_of_lines(1)
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg_primary()),
                )
                .child(
                    mono(tpl.rule_line)
                        .flex_1()
                        .min_w(px(0.0))
                        .text_right()
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .text_size(px(TEXT_XS))
                        .text_color(accent_blue_text()),
                ),
        )
        .child(
            text(move || SharedString::from(tpl.description(ui_lang(langs))))
                .number_of_lines(1)
                .text_overflow(EllipsizeMode::Tail)
                .text_size(px(TEXT_XS))
                .text_color(fg_muted()),
        )
}

fn rule_kind_form_group_section(
    group: QuickKindGroup,
    draft: QuickRuleDraftSignals,
    quick_add_open: Signal<bool>,
    open: Signal<bool>,
    proto_ctx: Option<(RuleEditTarget, Arc<Controls>, Signal<Option<Lang>>)>,
) -> impl IntoElement {
    view()
        .w_full()
        .gap_0p5()
        .flex_col()
        .child(
            text(tr(group.title_zh, group.title_en))
                .px_2()
                .pt_1()
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg_muted()),
        )
        .children(group.items.iter().copied().map(move |item| {
            let (zh, en) = quick_kind(item.idx).label();
            menu_item()
                .w_full()
                .h(px(26.0))
                .px_2()
                .rounded(px(RADIUS_MD))
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .hover(|style| style.bg(bg_control_hover()))
                .active(|style| style.bg(bg_control_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .on_select(move || {
                    draft.kind.set(item.idx);
                    quick_add_open.set(true);
                    open.set(false);
                })
                .child(
                    text(tr(zh, en))
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg_primary()),
                )
                .child(
                    mono(item.syntax)
                        .text_size(px(TEXT_XS))
                        .text_color(fg_muted()),
                )
        }))
        .children(proto_ctx.into_iter().map(move |(target, controls, langs)| {
            menu_item()
                .w_full()
                .h(px(26.0))
                .px_2()
                .rounded(px(RADIUS_MD))
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .hover(|style| style.bg(bg_control_hover()))
                .active(|style| style.bg(bg_control_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .on_select(move || {
                    let next = append_template_rule(
                        &target.current_text_untracked(),
                        PROTO_RULE_TEMPLATE.rule_line,
                    );
                    target.commit(&controls, next);
                    open.set(false);
                })
                .child(
                    text(move || SharedString::from(PROTO_RULE_TEMPLATE.label(ui_lang(langs))))
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg_primary()),
                )
                .child(mono("proto").text_size(px(TEXT_XS)).text_color(fg_muted()))
        }))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn all_rule_templates_parse_cleanly_with_zero_errors() {
        assert_eq!(QUICK_RULE_PRESETS.len(), 7);
        let mut ids = BTreeSet::new();
        for tpl in QUICK_RULE_PRESETS
            .iter()
            .chain(std::iter::once(&PROTO_RULE_TEMPLATE))
        {
            assert!(ids.insert(tpl.id), "duplicate template id `{}`", tpl.id);
            let parsed = crate::rules::parse_rules(tpl.rule_line);
            assert_eq!(
                parsed.len(),
                1,
                "template `{}` should parse into exactly one rule: {}",
                tpl.id,
                tpl.rule_line
            );
            let invalid = crate::rules::invalid_rule_lines(tpl.rule_line);
            assert!(
                invalid.is_empty(),
                "template `{}` reported syntax errors {:?}: {}",
                tpl.id,
                invalid,
                tpl.rule_line
            );
            assert!(!tpl.label_zh.is_empty() && !tpl.label_en.is_empty());
            assert!(!tpl.desc_zh.is_empty() && !tpl.desc_en.is_empty());
        }
    }

    #[test]
    fn all_quick_rule_examples_parse_cleanly_for_all_sixteen_kinds() {
        for kind in 0_u8..16 {
            let line = quick_rule_example(kind);
            assert_eq!(
                crate::rules::parse_rules(line).len(),
                1,
                "quick_rule_example({kind}) failed to parse: {line}"
            );
            assert!(
                crate::rules::invalid_rule_lines(line).is_empty(),
                "quick_rule_example({kind}) reported syntax error: {line}"
            );
        }
    }

    #[test]
    fn combined_templates_parse_all_lines_without_errors() {
        let mut buf = String::new();
        for tpl in QUICK_RULE_PRESETS
            .iter()
            .chain(std::iter::once(&PROTO_RULE_TEMPLATE))
        {
            buf = append_template_rule(&buf, tpl.rule_line);
        }
        assert_eq!(crate::rules::parse_rules(&buf).len(), 8);
        assert!(crate::rules::invalid_rule_lines(&buf).is_empty());
    }

    #[test]
    fn append_template_rule_handles_empty_and_trailing_newlines() {
        assert_eq!(append_template_rule("", "cors *"), "cors *");
        assert_eq!(append_template_rule("   \n", "cors *"), "cors *");
        assert_eq!(
            append_template_rule("delay * 100ms", "cors *"),
            "delay * 100ms\ncors *"
        );
        assert_eq!(
            append_template_rule("delay * 100ms\n\n", "cors *"),
            "delay * 100ms\ncors *"
        );
    }

    #[test]
    fn template_labels_and_descriptions_are_localized() {
        let tpl = QUICK_RULE_PRESETS[0];
        assert_eq!(tpl.label(Lang::Zh), tpl.label_zh);
        assert_eq!(tpl.label(Lang::En), tpl.label_en);
        assert_eq!(tpl.description(Lang::Zh), tpl.desc_zh);
        assert_eq!(tpl.description(Lang::En), tpl.desc_en);
    }
}
