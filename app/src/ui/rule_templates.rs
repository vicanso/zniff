//! Built-in one-click rule template library for the Rules popover (E4).

#[allow(unused_imports)]
use super::*;

/// A single built-in rule snippet that can be inserted into the active rules buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RuleTemplate {
    pub(crate) id: &'static str,
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

/// Built-in common rule templates covering everyday debugging workflows.
pub(crate) const RULE_TEMPLATES: &[RuleTemplate] = &[
    RuleTemplate {
        id: "dev-cors",
        label_zh: "开发期 CORS",
        label_en: "Dev CORS",
        desc_zh: "为全部请求自动补全开发期跨域响应头",
        desc_en: "Inject permissive CORS headers for all hosts",
        rule_line: "cors *",
    },
    RuleTemplate {
        id: "force-500",
        label_zh: "强制 500",
        label_en: "Force 500",
        desc_zh: "直接返回 HTTP 500 模拟服务端异常",
        desc_en: "Return HTTP 500 immediately to simulate server errors",
        rule_line: "map-local api.example.com/v1 status=500",
    },
    RuleTemplate {
        id: "slow-endpoint",
        label_zh: "慢接口 2s",
        label_en: "Slow 2s",
        desc_zh: "为全部请求注入 2000ms 延迟模拟慢接口",
        desc_en: "Add 2000ms latency to simulate slow endpoints",
        rule_line: "delay * 2000ms",
    },
    RuleTemplate {
        id: "no-cache",
        label_zh: "禁用缓存",
        label_en: "No cache",
        desc_zh: "清除条件请求头并注入 no-cache 响应头",
        desc_en: "Strip conditional headers and force no-cache responses",
        rule_line: "no-cache *",
    },
    RuleTemplate {
        id: "rewrite-base-url",
        label_zh: "替换 baseURL",
        label_en: "Rewrite baseURL",
        desc_zh: "将线上接口前缀转发到本地开发服务",
        desc_en: "Forward an API baseURL prefix to a local dev server",
        rule_line: "map-remote api.example.com/v1 http://127.0.0.1:3000/v1",
    },
    RuleTemplate {
        id: "hmac-signature",
        label_zh: "HMAC 签名头",
        label_en: "HMAC signature",
        desc_zh: "按请求正文动态计算并写入 HMAC-SHA256 签名头",
        desc_en: "Compute and attach an HMAC-SHA256 request signature header",
        rule_line: r#"header api.example.com/v1 req set X-Signature {{hmac_sha256("dev-secret", req.body)}}"#,
    },
];

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

/// Compact one-click rule template picker rendered beside Quick Add in the Rules popover.
pub(crate) fn rule_templates_section(
    target: RuleEditTarget,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let langs = ui_langs();
    view()
        .gap_1()
        .w_full()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    text(tr("规则模板", "Rule Templates"))
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg_muted()),
                )
                .child(
                    text(tr("点击插入到当前规则", "Click to append"))
                        .text_size(px(10.5))
                        .text_color(fg_muted()),
                ),
        )
        .child(
            view()
                .w_full()
                .p_2()
                .rounded(px(8.0))
                .border_1()
                .border_color(border_subtle())
                .bg(bg_canvas())
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_1p5()
                .children(RULE_TEMPLATES.iter().copied().map(move |tpl| {
                    let controls = controls.clone();
                    rule_template_chip(tpl, target, controls, langs)
                })),
        )
}

fn rule_template_chip(
    tpl: RuleTemplate,
    target: RuleEditTarget,
    controls: Arc<Controls>,
    langs: Signal<Option<Lang>>,
) -> impl IntoElement {
    view()
        .h(px(26.0))
        .px_2()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_1()
        .rounded(px(6.0))
        .border_1()
        .border_color(border_control())
        .bg(bg_control())
        .hover(|style| {
            style
                .bg(accent_blue_soft())
                .border_color(accent_blue_border())
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .tooltip(rule_template_tooltip(tpl, langs))
        .on_press(move || {
            let next = append_template_rule(&target.current_text_untracked(), tpl.rule_line);
            target.commit(&controls, next);
        })
        .child(
            text("+")
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(accent_blue_text()),
        )
        .child(
            text(move || SharedString::from(tpl.label(ui_lang(langs))))
                .number_of_lines(1)
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg_primary()),
        )
}

fn rule_template_tooltip(tpl: RuleTemplate, langs: Signal<Option<Lang>>) -> impl IntoTooltip {
    tooltip().child(
        view()
            .max_w(px(340.0))
            .px_2()
            .py_1p5()
            .gap_0p5()
            .flex_col()
            .rounded(px(6.0))
            .bg(bg_card())
            .border_1()
            .border_color(border_strong())
            .child(
                text(move || SharedString::from(tpl.description(ui_lang(langs))))
                    .text_size(move || scaled_px(11.5))
                    .text_color(fg_primary()),
            )
            .child(
                mono(tpl.rule_line)
                    .text_size(move || scaled_px(10.5))
                    .text_color(fg_muted()),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_rule_templates_parse_cleanly_with_zero_errors() {
        assert_eq!(RULE_TEMPLATES.len(), 6);
        for tpl in RULE_TEMPLATES {
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
    fn combined_templates_parse_all_lines_without_errors() {
        let mut buf = String::new();
        for tpl in RULE_TEMPLATES {
            buf = append_template_rule(&buf, tpl.rule_line);
        }
        assert_eq!(crate::rules::parse_rules(&buf).len(), RULE_TEMPLATES.len());
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
        let tpl = RULE_TEMPLATES[0];
        assert_eq!(tpl.label(Lang::Zh), tpl.label_zh);
        assert_eq!(tpl.label(Lang::En), tpl.label_en);
        assert_eq!(tpl.description(Lang::Zh), tpl.desc_zh);
        assert_eq!(tpl.description(Lang::En), tpl.desc_en);
    }
}
