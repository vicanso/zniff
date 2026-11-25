#[allow(unused_imports)]
use super::*;

#[derive(Clone, Copy)]
pub(crate) struct QuickKindItem {
    pub(crate) idx: u8,
    pub(crate) syntax: &'static str,
}

#[derive(Clone, Copy)]
pub(crate) struct QuickKindGroup {
    pub(crate) title_zh: &'static str,
    pub(crate) title_en: &'static str,
    pub(crate) items: &'static [QuickKindItem],
}

pub(crate) const QUICK_KIND_GROUPS: &[QuickKindGroup] = &[
    QuickKindGroup {
        title_zh: "内容改写",
        title_en: "Content Rewrite",
        items: &[
            QuickKindItem {
                idx: 0,
                syntax: "replace",
            },
            QuickKindItem {
                idx: 11,
                syntax: "json",
            },
            QuickKindItem {
                idx: 6,
                syntax: "header",
            },
            QuickKindItem {
                idx: 7,
                syntax: "query",
            },
            QuickKindItem {
                idx: 8,
                syntax: "cors",
            },
            QuickKindItem {
                idx: 12,
                syntax: "no-cache",
            },
        ],
    },
    QuickKindGroup {
        title_zh: "映射与阻断",
        title_en: "Map & Block",
        items: &[
            QuickKindItem {
                idx: 1,
                syntax: "map-remote",
            },
            QuickKindItem {
                idx: 2,
                syntax: "map-local",
            },
            QuickKindItem {
                idx: 13,
                syntax: "block",
            },
        ],
    },
    QuickKindGroup {
        title_zh: "断点与弱网",
        title_en: "Break & Network",
        items: &[
            QuickKindItem {
                idx: 4,
                syntax: "break",
            },
            QuickKindItem {
                idx: 5,
                syntax: "break-res",
            },
            QuickKindItem {
                idx: 3,
                syntax: "delay",
            },
            QuickKindItem {
                idx: 9,
                syntax: "throttle",
            },
        ],
    },
    QuickKindGroup {
        title_zh: "高级代理",
        title_en: "Advanced",
        items: &[
            QuickKindItem {
                idx: 14,
                syntax: "resolve",
            },
            QuickKindItem {
                idx: 10,
                syntax: "cert",
            },
            QuickKindItem {
                idx: 15,
                syntax: "hook",
            },
        ],
    },
];

fn quick_kind_syntax(idx: u8) -> &'static str {
    QUICK_KIND_GROUPS
        .iter()
        .flat_map(|g| g.items.iter())
        .find(|item| item.idx == idx)
        .map_or("", |item| item.syntax)
}

pub(crate) fn quick_kind_dropdown(draft: QuickRuleDraftSignals) -> impl IntoElement {
    let kind = draft.kind;
    let open = signal(false);
    bind_escape_while(
        move || open.get(),
        nsh_core::keymap::When::Always,
        move || open.set(false),
    );
    let langs = ui_langs();
    let items = move || -> Vec<SelectItem> {
        let lang = ui_lang(langs);
        QUICK_KIND_GROUPS
            .iter()
            .flat_map(|group| {
                let group_title: SharedString = t(lang, group.title_zh, group.title_en).into();
                group.items.iter().map(move |item| {
                    let (zh, en) = quick_kind(item.idx).label();
                    SelectItem::new(item.idx.to_string(), t(lang, zh, en))
                        .group(group_title.clone())
                })
            })
            .collect()
    };
    select()
        .items(items)
        .value(move || Some(SharedString::from(kind.get().to_string())))
        .open(move || Some(open.get()))
        .on_open_change(move |event: &SelectEvent| {
            if let SelectEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .on_change(move |value: &Option<SharedString>| {
            if let Some(raw) = value.as_ref()
                && let Ok(idx) = raw.parse::<u8>()
            {
                switch_quick_kind(draft, idx);
            }
        })
        .cover(false)
        .placement(Placement::Bottom)
        .align(Align::Start)
        .offset(px(4.0))
        .margin(px(8.0))
        .accessibility_label(tr_opt("选择规则类型", "Select rule type"))
        .popup_style(|s| ui_popup_style(s).w(px(276.0)))
        .popup_starting_style(ui_popup_starting_style)
        .popup_ending_style(ui_popup_ending_style)
        .list_style(|s| s.max_h(px(320.0)).p_1p5())
        .item_class("focus:bg-accent hover:bg-accent")
        .item_style(|s| {
            s.w_full()
                .h(px(26.0))
                .px_2()
                .rounded(px(RADIUS_MD))
                .flex_row()
                .items_center()
                .selected(|sel| sel.bg(accent_blue_soft()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
        })
        .render_group(|group: &SelectGroup| {
            text(group.title.clone())
                .px_2()
                .pt_1()
                .pb_0p5()
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_muted())
        })
        .render_item(move |entry: &SelectEntry| {
            let item_idx = entry.value.parse::<u8>().ok();
            let syntax = item_idx.map_or("", quick_kind_syntax);
            view()
                .w_full()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text(entry.title.clone())
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if item_idx == Some(kind.get()) {
                                accent_blue_text()
                            } else {
                                fg_primary()
                            }
                        }),
                )
                .child(mono(syntax).text_size(px(TEXT_XS)).text_color(fg_muted()))
        })
        .trigger(
            view()
                .h(px(CONTROL_H))
                .px_2()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_1()
                .rounded(px(RADIUS_MD))
                .border_1()
                .border_color(accent_blue_border())
                .bg(accent_blue_soft())
                .hover(|style| style.bg(accent_blue_soft_hover()))
                .active(|style| style.bg(bg_control_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .tooltip(label_tip_unless(open, "选择规则类型", "Select rule type"))
                .child(
                    text(move || {
                        let (zh, en) = quick_kind(kind.get()).label();
                        SharedString::from(t(ui_lang(langs), zh, en))
                    })
                    .number_of_lines(1)
                    .text_size(px(TEXT_SM))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(accent_blue_text()),
                )
                .child(glyph_px(
                    10.0,
                    || Glyph::Chevron,
                    || color(accent_blue_text()),
                )),
        )
}

pub(crate) fn quick_var_menu(target: Signal<String>) -> impl IntoElement {
    const VARS: &[(&str, &str, &str, &str)] = &[
        ("{{ts_ms}}", "{{ts_ms}}", "毫秒时间戳", "ms epoch"),
        ("{{ts}}", "{{ts}}", "秒级时间戳", "s epoch"),
        ("{{uuid}}", "{{uuid}}", "随机 UUID", "UUID v4"),
        ("{{req.path}}", "{{req.path}}", "请求路径", "Request path"),
        ("{{req.body}}", "{{req.body}}", "请求正文", "Request body"),
        ("sha256", "{{sha256(req.body)}}", "正文摘要", "Body SHA-256"),
        (
            "hmac_sha256",
            "{{hmac_sha256(\"secret\", req.body)}}",
            "HMAC 签名",
            "HMAC signature",
        ),
    ];
    let open = signal(false);
    bind_escape_while(
        move || open.get(),
        nsh_core::keymap::When::Always,
        move || open.set(false),
    );
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
        .popup_ending_style(ui_popup_ending_style)
        .trigger(
            view()
                .role(AccessibilityRole::Button)
                .h(px(CONTROL_H))
                .px_1p5()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_1()
                .rounded(px(RADIUS_MD))
                .bg(move || {
                    if open.get() {
                        accent_blue_soft()
                    } else {
                        AdaptiveColor::TRANSPARENT
                    }
                })
                .hover(|style| style.bg(bg_control_hover()))
                .active(|style| style.bg(bg_control_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .tooltip(label_tip_unless(
                    open,
                    "插入动态模板变量",
                    "Insert template variable",
                ))
                .child(
                    mono("{{}}")
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if open.get() {
                                accent_blue_text()
                            } else {
                                fg_muted()
                            }
                        }),
                )
                .child(glyph_px(10.0, || Glyph::Chevron, || color(fg_muted()))),
        )
        .child(
            view().w(px(268.0)).overflow_hidden().child(
                scroll().w_full().max_h(px(240.0)).child(
                    view()
                        .w_full()
                        .p_1p5()
                        .gap_0p5()
                        .flex_col()
                        .children(VARS.iter().map(|&(label, snippet, desc_zh, desc_en)| {
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
                                    target.update(|s| s.push_str(snippet));
                                    open.set(false);
                                })
                                .child(mono(label).text_size(px(TEXT_XS)).text_color(fg_primary()))
                                .child(
                                    text(tr(desc_zh, desc_en))
                                        .text_size(px(TEXT_XS))
                                        .text_color(fg_muted()),
                                )
                        })),
                ),
            ),
        )
}
