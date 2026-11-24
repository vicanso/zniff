#[allow(unused_imports)]
use super::*;

use nsh_core::keymap::Keystroke;
use nsh_core::platform::Os;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ShortcutEntry {
    pub(crate) keys: &'static [&'static str],
    pub(crate) zh: &'static str,
    pub(crate) en: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ShortcutSection {
    pub(crate) title_zh: &'static str,
    pub(crate) title_en: &'static str,
    pub(crate) items: &'static [ShortcutEntry],
}

pub(crate) const SHORTCUT_SECTIONS: &[ShortcutSection] = &[
    ShortcutSection {
        title_zh: "导航与视图",
        title_en: "Navigation & View",
        items: &[
            ShortcutEntry {
                keys: &["up", "down"],
                zh: "在筛选列表中切换选中流量",
                en: "Select previous / next flow",
            },
            ShortcutEntry {
                keys: &["mod+f"],
                zh: "聚焦列表过滤输入框",
                en: "Focus list filter input",
            },
            ShortcutEntry {
                keys: &["escape"],
                zh: "收起弹窗 / 编辑框，或取消选中回到统计视图",
                en: "Close modal / editor, or deselect to summary",
            },
        ],
    },
    ShortcutSection {
        title_zh: "抓包与操作",
        title_en: "Capture & Actions",
        items: &[
            ShortcutEntry {
                keys: &["mod+k"],
                zh: "清空会话流量（5 秒内可撤销）",
                en: "Clear session flows (5s undo)",
            },
            ShortcutEntry {
                keys: &["mod+r"],
                zh: "重放当前选中的流量",
                en: "Replay selected flow",
            },
            ShortcutEntry {
                keys: &["mod+shift+m"],
                zh: "插入会话时间标记",
                en: "Insert session marker",
            },
            ShortcutEntry {
                keys: &["mod+enter"],
                zh: "在编辑重发 / 断点框内一键发送或放行",
                en: "Send composed request or resume breakpoint",
            },
            ShortcutEntry {
                keys: &["enter"],
                zh: "在 URL 输入框内直接发送（非 curl 粘贴）",
                en: "Submit from URL input (non-curl)",
            },
        ],
    },
    ShortcutSection {
        title_zh: "全局与帮助",
        title_en: "Global & Help",
        items: &[
            ShortcutEntry {
                keys: &["mod+shift+p"],
                zh: "打开 / 关闭命令面板",
                en: "Toggle Command Palette",
            },
            ShortcutEntry {
                keys: &["mod+,"],
                zh: "打开 / 关闭设置窗口",
                en: "Toggle Settings",
            },
            ShortcutEntry {
                keys: &["mod+=", "mod+-", "mod+0"],
                zh: "放大 / 缩小 / 重置界面与字体缩放",
                en: "Zoom in / out / reset UI & font size",
            },
            ShortcutEntry {
                keys: &["shift+/", "mod+/"],
                zh: "打开 / 关闭快捷键速查",
                en: "Toggle Keyboard Shortcuts",
            },
        ],
    },
];

pub(crate) fn format_shortcut_badge(spec: &str, apple: bool) -> String {
    match spec {
        "shift+/" => "?".to_string(),
        "up" => "↑".to_string(),
        "down" => "↓".to_string(),
        "escape" => "Esc".to_string(),
        "enter" => "Enter".to_string(),
        _ => Keystroke::parse_with(spec, apple)
            .map(|k| k.label(if apple { Os::MacOs } else { Os::Linux }, apple))
            .unwrap_or_else(|_| spec.to_string()),
    }
}

fn is_apple_platform() -> bool {
    cfg!(target_os = "macos")
}

pub(crate) fn shortcuts_modal(open: Signal<bool>) -> impl IntoElement {
    when(
        move || open.get(),
        move || {
            let langs = ui_langs();
            let apple = is_apple_platform();
            let backdrop = ui_modal_backdrop(move || open.set(false));

            let header = view()
                .w_full()
                .px_4()
                .py_3()
                .flex_row()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_b_color(border_subtle())
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            text(tr("快捷键速查", "Keyboard Shortcuts"))
                                .text_size(px(TEXT_LG))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(fg_primary()),
                        )
                        .child(keycap_badge(format_shortcut_badge("mod+/", apple))),
                )
                .child(ui_modal_close_button(move || open.set(false)));

            let lang = ui_lang(langs);
            let body = SHORTCUT_SECTIONS
                .iter()
                .fold(view().w_full().p_4().gap_4().flex_col(), |col, section| {
                    col.child(shortcut_section_view(section, lang, apple))
                });

            let footer = view()
                .w_full()
                .px_4()
                .py_2p5()
                .flex_row()
                .items_center()
                .justify_between()
                .border_t_1()
                .border_t_color(border_subtle())
                .child(
                    text(tr(
                        "非输入状态下按 ? 随时呼出，Esc 关闭",
                        "Press ? outside inputs anytime, Esc to close",
                    ))
                    .text_size(px(TEXT_XS))
                    .text_color(fg_muted()),
                )
                .child(
                    view()
                        .role(AccessibilityRole::Button)
                        .h(px(ControlSize::Sm.height()))
                        .px_3()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .rounded(px(RADIUS_MD))
                        .border_1()
                        .border_color(accent_blue_border())
                        .bg(accent_blue_soft())
                        .hover(|s| s.bg(accent_blue_soft_hover()))
                        .active(|s| s.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .on_press(move || open.set(false))
                        .child(control_text(tr("关闭", "Close")).text_color(accent_blue_text())),
                );

            let card = ui_modal_card()
                .w(move || px((window().width.0 - 32.0).clamp(320.0, 520.0)))
                .max_h(move || px((window().height.0 - 64.0).max(240.0)))
                .child(header)
                .child(
                    scroll()
                        .w_full()
                        .max_h(move || px((window().height.0 - 168.0).max(160.0)))
                        .child(body),
                )
                .child(footer);

            overlay()
                .inset_0()
                .on_cancel(move || open.set(false))
                .child(
                    view()
                        .w_full()
                        .h_full()
                        .items_center()
                        .justify_center()
                        .child(backdrop)
                        .child(card),
                )
        },
    )
}

fn shortcut_section_view(section: &ShortcutSection, lang: Lang, apple: bool) -> impl IntoElement {
    let title = text(t(lang, section.title_zh, section.title_en))
        .text_size(px(TEXT_XS))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg_muted());

    let rows = section
        .items
        .iter()
        .fold(view().w_full().gap_1p5().flex_col(), |col, item| {
            let badges = item.keys.iter().enumerate().fold(
                view().flex_row().items_center().gap_1().flex_shrink_0(),
                |row, (idx, spec)| {
                    let row = if idx > 0 {
                        row.child(text("/").text_size(px(TEXT_XS)).text_color(fg_muted()))
                    } else {
                        row
                    };
                    row.child(keycap_badge(format_shortcut_badge(spec, apple)))
                },
            );

            col.child(
                view()
                    .w_full()
                    .px_2p5()
                    .py_1p5()
                    .rounded(px(RADIUS_MD))
                    .bg(bg_code())
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        text(t(lang, item.zh, item.en))
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(px(TEXT_SM))
                            .text_color(fg_primary()),
                    )
                    .child(badges),
            )
        });

    view()
        .w_full()
        .gap_1p5()
        .flex_col()
        .child(title)
        .child(rows)
}

fn keycap_badge(label: String) -> impl IntoElement {
    mono(label)
        .text_size(px(TEXT_XS))
        .px_1p5()
        .py_0p5()
        .rounded(px(RADIUS_SM))
        .border_1()
        .border_color(border_control())
        .bg(bg_control())
        .text_color(fg_secondary())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_shortcut_specs_parse_cleanly_on_apple_and_non_apple() {
        assert!(!SHORTCUT_SECTIONS.is_empty());
        for section in SHORTCUT_SECTIONS {
            assert!(!section.title_zh.is_empty());
            assert!(!section.title_en.is_empty());
            assert!(!section.items.is_empty());
            for item in section.items {
                assert!(!item.zh.is_empty());
                assert!(!item.en.is_empty());
                assert!(!item.keys.is_empty());
                for &spec in item.keys {
                    let apple_ks = Keystroke::parse_with(spec, true).unwrap_or_else(|e| {
                        panic!("apple keystroke parse failed for `{spec}`: {e}")
                    });
                    let other_ks = Keystroke::parse_with(spec, false).unwrap_or_else(|e| {
                        panic!("non-apple keystroke parse failed for `{spec}`: {e}")
                    });
                    for os in [nsh_core::platform::Os::MacOs, nsh_core::platform::Os::Ios] {
                        assert!(
                            apple_ks.reserved(os).is_none(),
                            "shortcut `{spec}` is reserved on {os:?}: {:?}",
                            apple_ks.reserved(os)
                        );
                    }
                    for os in [
                        nsh_core::platform::Os::Linux,
                        nsh_core::platform::Os::Windows,
                        nsh_core::platform::Os::Android,
                        nsh_core::platform::Os::Web,
                    ] {
                        assert!(
                            other_ks.reserved(os).is_none(),
                            "shortcut `{spec}` is reserved on {os:?}: {:?}",
                            other_ks.reserved(os)
                        );
                    }
                    assert!(!format_shortcut_badge(spec, true).is_empty());
                    assert!(!format_shortcut_badge(spec, false).is_empty());
                }
            }
        }
    }

    #[test]
    fn format_shortcut_badge_formats_platform_labels() {
        assert_eq!(format_shortcut_badge("shift+/", true), "?");
        assert_eq!(format_shortcut_badge("shift+/", false), "?");
        assert_eq!(format_shortcut_badge("mod+/", true), "⌘/");
        assert_eq!(format_shortcut_badge("mod+/", false), "Ctrl+/");
        assert_eq!(format_shortcut_badge("mod+k", true), "⌘K");
        assert_eq!(format_shortcut_badge("mod+k", false), "Ctrl+K");
        assert_eq!(format_shortcut_badge("mod+shift+p", true), "⇧⌘P");
        assert_eq!(format_shortcut_badge("mod+shift+p", false), "Ctrl+Shift+P");
        assert_eq!(format_shortcut_badge("mod+shift+m", true), "⇧⌘M");
        assert_eq!(format_shortcut_badge("mod+shift+m", false), "Ctrl+Shift+M");
    }
}
