//! Timeline session marker insertion modal and selected-marker detail card (E5).

#[allow(unused_imports)]
use super::*;

const PRESETS_ZH: &[&str] = &["开始测试", "登录流程", "下单支付", "复现问题"];
const PRESETS_EN: &[&str] = &["Start test", "Login", "Checkout", "Repro"];

/// Builds a valid whitespace-free `mark:<token>` filter term for a marker label.
pub(crate) fn marker_filter_token(label: &str) -> String {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return "mark:".to_string();
    }
    let token = trimmed
        .split_whitespace()
        .max_by_key(|part| part.chars().count())
        .unwrap_or(trimmed);
    format!("mark:{token}")
}

fn submit_marker(controls: &Arc<Controls>, open: Signal<bool>, draft: Signal<String>, lang: Lang) {
    if guard_read_only_tab(controls) {
        open.set(false);
        return;
    }
    let raw = draft.get_untracked();
    let label = raw.trim();
    let final_label = if label.is_empty() {
        t(lang, "时间标记", "Marker").to_string()
    } else {
        label.to_string()
    };
    controls.hub.add_marker(final_label);
    draft.set(String::new());
    open.set(false);
}

pub(crate) fn marker_modal(open: Signal<bool>, controls: Arc<Controls>) -> impl IntoElement {
    let draft = signal(String::new());
    let focus = element_ref();
    let focus_job = stored(None::<Spawned<()>>);
    let langs = ui_langs();

    effect(move || {
        if open.get() {
            draft.set(String::new());
            focus_job.replace(Some(set_timeout(Duration::from_millis(20), move || {
                focus.focus();
            })));
        } else {
            focus_job.replace(None);
        }
    });

    when(
        move || open.get(),
        move || {
            let controls_submit = controls.clone();
            let controls_enter = controls.clone();
            let controls_presets = controls.clone();
            let apple = is_apple_platform();
            let badge = format_shortcut_badge("mod+shift+m", apple);

            overlay()
                .inset_0()
                .on_cancel(move || open.set(false))
                .child(
                    view()
                        .w_full()
                        .h_full()
                        .items_center()
                        .justify_center()
                        .child(ui_modal_backdrop(move || open.set(false)))
                        .child(
                            ui_modal_card()
                                .w(move || px((window().width.0 - 32.0).clamp(280.0, 420.0)))
                                .p_4()
                                .gap_3()
                                .child(marker_modal_header(open, badge))
                                .child(
                                    view().gap_1p5().child(
                                        input()
                                            .element_ref(&focus)
                                            .w_full()
                                            .h(px(ControlSize::Md.height()))
                                            .px_2p5()
                                            .rounded(px(RADIUS_MD))
                                            .border_1()
                                            .border_color(border_control())
                                            .focus(|s| s.border_color(accent_blue()))
                                            .bg(bg_canvas())
                                            .text_size(px(TEXT_SM))
                                            .text_color(fg_primary())
                                            .placeholder(tr(
                                                "输入标记名称（如：开始测登录）",
                                                "Marker name (e.g. Start login test)",
                                            ))
                                            .value(move || draft.get())
                                            .on_change(move |v: &str| draft.set(v.to_string()))
                                            .on_submit(move |_: &str| {
                                                submit_marker(
                                                    &controls_enter,
                                                    open,
                                                    draft,
                                                    ui_lang(langs),
                                                );
                                            }),
                                    ),
                                )
                                .child(marker_preset_chips(controls_presets, open, draft))
                                .child(marker_modal_footer(controls_submit, open, draft)),
                        ),
                )
        },
    )
}

fn marker_modal_header(open: Signal<bool>, badge: String) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(tr("插入会话标记", "Insert session marker"))
                        .text_size(px(TEXT_LG))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg_primary()),
                )
                .child(
                    view()
                        .px_1p5()
                        .py_0p5()
                        .rounded(px(RADIUS_SM))
                        .bg(bg_control())
                        .child(mono(badge).text_size(px(TEXT_XS)).text_color(fg_muted())),
                ),
        )
        .child(ui_modal_close_button(move || open.set(false)))
}

fn marker_preset_chips(
    controls: Arc<Controls>,
    open: Signal<bool>,
    draft: Signal<String>,
) -> impl IntoElement {
    let langs = ui_langs();
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_1p5()
        .child(
            text(tr("常用：", "Quick:"))
                .text_size(px(TEXT_XS))
                .text_color(fg_muted()),
        )
        .children((0..PRESETS_ZH.len()).map(move |idx| {
            let controls = controls.clone();
            view()
                .role(AccessibilityRole::Button)
                .h(px(24.0))
                .px_2()
                .rounded(px(RADIUS_MD))
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
                    if guard_read_only_tab(&controls) {
                        open.set(false);
                        return;
                    }
                    let lang = ui_lang(langs);
                    let label = match lang {
                        Lang::Zh => PRESETS_ZH[idx],
                        Lang::En => PRESETS_EN[idx],
                    };
                    controls.hub.add_marker(label);
                    draft.set(String::new());
                    open.set(false);
                })
                .child(
                    text(tr(PRESETS_ZH[idx], PRESETS_EN[idx]))
                        .text_size(px(TEXT_XS))
                        .text_color(fg_secondary()),
                )
        }))
}

fn marker_modal_footer(
    controls: Arc<Controls>,
    open: Signal<bool>,
    draft: Signal<String>,
) -> impl IntoElement {
    let langs = ui_langs();
    view()
        .flex_row()
        .items_center()
        .justify_end()
        .gap_2()
        .pt_1()
        .child(
            view()
                .role(AccessibilityRole::Button)
                .h(px(ControlSize::Sm.height()))
                .px_3()
                .rounded(px(RADIUS_MD))
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
                .on_press(move || open.set(false))
                .child(
                    control_text(tr("取消", "Cancel"))
                        .text_size(px(TEXT_SM))
                        .text_color(fg_secondary()),
                ),
        )
        .child(
            view()
                .role(AccessibilityRole::Button)
                .h(px(ControlSize::Sm.height()))
                .px_3()
                .rounded(px(RADIUS_MD))
                .border_1()
                .border_color(accent_blue_border())
                .bg(accent_blue())
                .hover(|s| s.bg(accent_blue_hover()))
                .active(|s| s.bg(accent_blue_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .items_center()
                .justify_center()
                .on_press(move || submit_marker(&controls, open, draft, ui_lang(langs)))
                .child(
                    control_text(tr("插入标记", "Add marker"))
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg_on_accent()),
                ),
        )
}

/// Detail card rendered in the right pane when a timeline marker row is selected.
pub(crate) fn marker_detail_card(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    list_filter: Signal<String>,
) -> impl IntoElement {
    let langs = ui_langs();
    let edit_label = signal(String::new());
    effect(move || {
        let cur = shown.url.get();
        edit_label.set(cur);
    });
    let controls_count = controls.clone();
    let controls_ro = controls.clone();
    let controls_ro_del = controls.clone();
    let controls_ro_note = controls.clone();
    let controls_rename = controls.clone();
    let controls_rename_enter = controls.clone();
    let controls_copy = controls.clone();
    let controls_del = controls;

    uncull_nested_scroll(view())
        .pl_3p5()
        .pr_5()
        .py_3p5()
        .gap_3()
        .child(
            view()
                .p_4()
                .gap_3()
                .rounded(px(RADIUS_LG))
                .border_1()
                .border_color(accent_blue_border())
                .bg(bg_card())
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(
                                    view()
                                        .px_2()
                                        .py_0p5()
                                        .rounded(px(RADIUS_SM))
                                        .bg(accent_blue_soft())
                                        .child(
                                            mono("MARK")
                                                .text_size(px(TEXT_XS))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(accent_blue_text()),
                                        ),
                                )
                                .child(
                                    text(move || -> SharedString {
                                        let label = shown.url.get();
                                        if label.is_empty() {
                                            t(ui_lang(langs), "时间标记", "Marker").into()
                                        } else {
                                            label.into()
                                        }
                                    })
                                    .text_size(px(TEXT_LG))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(fg_primary()),
                                ),
                        )
                        .child(
                            mono(move || -> SharedString { shown.metrics.get().into() })
                                .text_size(px(TEXT_XS))
                                .text_color(fg_muted()),
                        ),
                )
                .child(
                    text(move || -> SharedString {
                        let lang = ui_lang(langs);
                        let count = selected
                            .get()
                            .map(|id| {
                                with_active_session(&controls_count, |s| {
                                    s.flows_in_marker_section(id)
                                })
                            })
                            .unwrap_or(0);
                        match lang {
                            Lang::Zh => format!("此标记段包含 {count} 条流量"),
                            Lang::En => format!("{count} flows in this marker section"),
                        }
                        .into()
                    })
                    .text_size(px(TEXT_SM))
                    .text_color(fg_secondary()),
                )
                .child(when(
                    move || !is_active_tab_read_only(&controls_ro),
                    move || {
                        let controls_rename_enter = controls_rename_enter.clone();
                        let controls_rename = controls_rename.clone();
                        let rename_empty = move || edit_label.get().trim().is_empty();
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                input()
                                    .flex_1()
                                    .h(px(ControlSize::Sm.height()))
                                    .px_2()
                                    .rounded(px(RADIUS_MD))
                                    .border_1()
                                    .border_color(border_control())
                                    .bg(bg_canvas())
                                    .text_size(px(TEXT_SM))
                                    .text_color(fg_primary())
                                    .placeholder(tr("重命名标记…", "Rename marker…"))
                                    .placeholder_color(move || color(fg_muted()))
                                    .value(move || edit_label.get())
                                    .on_change(move |v: &str| edit_label.set(v.to_string()))
                                    .on_submit(move |_: &str| {
                                        if guard_read_only_tab(&controls_rename_enter) {
                                            return;
                                        }
                                        let next = edit_label.get_untracked();
                                        if next.trim().is_empty() {
                                            return;
                                        }
                                        if let Some(id) = selected.get_untracked() {
                                            controls_rename_enter.hub.rename_marker(id, next);
                                        }
                                    }),
                            )
                            .child(
                                view()
                                    .role(AccessibilityRole::Button)
                                    .h(px(ControlSize::Sm.height()))
                                    .px_2p5()
                                    .rounded(px(RADIUS_MD))
                                    .border_1()
                                    .border_color(border_control())
                                    .bg(bg_control())
                                    .hover(|s| {
                                        s.bg(move || {
                                            if rename_empty() {
                                                bg_control()
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
                                    .disabled(rename_empty)
                                    .disabled_style(|s| s.opacity(0.5))
                                    .items_center()
                                    .justify_center()
                                    .on_press(move || {
                                        if guard_read_only_tab(&controls_rename) {
                                            return;
                                        }
                                        let next = edit_label.get_untracked();
                                        if next.trim().is_empty() {
                                            return;
                                        }
                                        if let Some(id) = selected.get_untracked() {
                                            controls_rename.hub.rename_marker(id, next);
                                        }
                                    })
                                    .child(
                                        control_text(tr("重命名", "Rename"))
                                            .text_size(px(TEXT_SM))
                                            .text_color(fg_secondary()),
                                    ),
                            )
                    },
                ))
                .child(when(
                    move || is_active_tab_read_only(&controls_ro_note),
                    move || {
                        text(tr(
                            "只读导入会话：无法重命名或删除时间标记",
                            "Read-only imported tab: markers cannot be renamed or deleted",
                        ))
                        .text_size(px(TEXT_XS))
                        .text_color(fg_muted())
                    },
                ))
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .pt_1()
                        .child(
                            view()
                                .role(AccessibilityRole::Button)
                                .h(px(ControlSize::Sm.height()))
                                .px_3()
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
                                .items_center()
                                .justify_center()
                                .on_press(move || {
                                    let tok = marker_filter_token(&shown.url.get_untracked());
                                    list_filter.set(tok);
                                })
                                .child(
                                    control_text(tr("只看此标记段", "Filter this section"))
                                        .text_size(px(TEXT_SM))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(accent_blue_text()),
                                ),
                        )
                        .child(
                            view()
                                .role(AccessibilityRole::Button)
                                .h(px(ControlSize::Sm.height()))
                                .px_3()
                                .rounded(px(RADIUS_MD))
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
                                    let label = shown.url.get_untracked();
                                    set_clipboard(label);
                                    set_status(&controls_copy, StatusMsg::CopiedMarker);
                                })
                                .child(
                                    control_text(tr("复制标记名称", "Copy marker label"))
                                        .text_size(px(TEXT_SM))
                                        .text_color(fg_secondary()),
                                ),
                        )
                        .child(when(
                            move || !is_active_tab_read_only(&controls_ro_del),
                            move || {
                                let controls_del = controls_del.clone();
                                view()
                                    .role(AccessibilityRole::Button)
                                    .h(px(ControlSize::Sm.height()))
                                    .px_3()
                                    .rounded(px(RADIUS_MD))
                                    .border_1()
                                    .border_color(status_err_bg())
                                    .bg(bg_control())
                                    .hover(|s| s.bg(status_err_bg()))
                                    .active(|s| s.bg(bg_control_active()))
                                    .transition_colors()
                                    .duration(MOTION_INSTANT_MS)
                                    .ease_out()
                                    .focus_visible(apply_focus_ring)
                                    .items_center()
                                    .justify_center()
                                    .on_press(move || {
                                        if guard_read_only_tab(&controls_del) {
                                            return;
                                        }
                                        if let Some(id) = selected.get_untracked() {
                                            controls_del.hub.remove_marker(id);
                                            selected.set(None);
                                        }
                                    })
                                    .child(
                                        control_text(tr("删除标记", "Delete marker"))
                                            .text_size(px(TEXT_SM))
                                            .text_color(status_err_fg()),
                                    )
                            },
                        )),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_filter_token_picks_valid_token() {
        assert_eq!(marker_filter_token(""), "mark:");
        assert_eq!(marker_filter_token("开始测登录"), "mark:开始测登录");
        assert_eq!(marker_filter_token("Start login test"), "mark:login");
    }

    #[test]
    fn filter_autocomplete_supports_mark_prefix_and_values() {
        let mut s = session::Session::default();
        s.add_marker("login");
        s.add_marker("checkout");
        let flows = s.flows();
        let facets = s.facets();
        let catalog = FilterSuggestionsCatalog::from_flows(flows.iter(), &facets);
        assert_eq!(catalog.markers, vec!["login", "checkout"]);
        assert!(validate_filter_syntax("mark:").is_empty());
        assert!(validate_filter_syntax("-mark:").is_empty());
        assert!(validate_filter_syntax("mark:login").is_empty());
        let suggestions = suggest_filter_completions("mark:lo", &catalog, Lang::En);
        assert!(
            suggestions
                .iter()
                .any(|item| item.insert_token == "mark:login")
        );
    }
}
