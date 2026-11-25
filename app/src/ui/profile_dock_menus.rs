//! Dropdown menus for saved presets and setup history inside `profile_dock`.

#[allow(unused_imports)]
use super::*;

#[derive(Clone)]
pub(crate) struct ProfileDockMenuCtx {
    pub(crate) profiles: ProfileSignals,
    pub(crate) draft: QuickRuleDraftSignals,
    pub(crate) adopted: Signal<bool>,
    pub(crate) dropdown_open: Signal<bool>,
    pub(crate) history_open: Signal<bool>,
    pub(crate) controls: Arc<Controls>,
}

fn shown_profile_name(profiles: ProfileSignals) -> Option<String> {
    let typed = profiles.name.get();
    let typed = typed.trim();
    if !typed.is_empty() {
        return Some(typed.to_string());
    }
    let filter = profiles.filter.get();
    let bypass = profiles.bypass.get();
    let rules = profiles.rules.get();
    let upstream = profiles.upstream.get();
    profiles
        .saved
        .with(|items| matched_saved_profile(items, &filter, &bypass, &rules, &upstream))
}

fn menu_count_badge(count: impl Fn() -> usize + 'static) -> impl IntoElement {
    view()
        .h(px(16.0))
        .px_1()
        .rounded(px(RADIUS_SM))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .bg(bg_card())
        .child(
            mono(move || count().to_string())
                .tabular_nums()
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_muted()),
        )
}

fn saved_profile_key(item: &config::SavedProfile) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}",
        item.name, item.filter, item.bypass_hosts, item.rules, item.upstream_proxy
    )
}

fn history_entry_key(item: &config::SavedProfile) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        item.name,
        item.filter,
        item.bypass_hosts,
        item.rules,
        item.upstream_proxy,
        item.rules_enabled,
        item.breaks_enabled,
        item.show_tunnel_failures
    )
}

/// Saved setups dropdown menu inside the Preset popover dock.
pub(crate) fn profile_saved_menu(ctx: ProfileDockMenuCtx) -> impl IntoElement {
    let saved = ctx.profiles.saved;
    let dropdown_open = ctx.dropdown_open;
    let history_open = ctx.history_open;
    let confirm_delete = signal(None::<String>);
    bind_escape_while(
        move || dropdown_open.get() && !saved.with(Vec::is_empty),
        nsh_core::keymap::When::Always,
        move || dropdown_open.set(false),
    );
    let trigger_ctx = ctx.clone();
    menu()
        .open(move || Some(dropdown_open.get() && !saved.with(Vec::is_empty)))
        .on_open_change(move |event: &nsh_base::MenuEvent| {
            if let nsh_base::MenuEvent::OpenChange(next, _) = event {
                dropdown_open.set(*next && !saved.with(Vec::is_empty));
                if *next {
                    history_open.set(false);
                } else {
                    confirm_delete.set(None);
                }
            }
        })
        .placement(Placement::Bottom)
        .align(Align::Start)
        .offset(px(6.0))
        .margin(px(8.0))
        .popup_style(ui_popup_style)
        .popup_starting_style(ui_popup_starting_style)
        .popup_ending_style(ui_popup_ending_style)
        .trigger(profile_saved_trigger(trigger_ctx))
        .child(profile_saved_popup(ctx, confirm_delete))
}

fn profile_saved_trigger(ctx: ProfileDockMenuCtx) -> impl IntoElement {
    let profiles = ctx.profiles;
    let saved = profiles.saved;
    let dropdown_open = ctx.dropdown_open;
    let langs = ui_langs();
    view()
        .role(AccessibilityRole::Button)
        .min_w(px(132.0))
        .max_w(px(176.0))
        .h(px(CONTROL_H))
        .px_2p5()
        .flex_shrink_0()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(move || {
            if dropdown_open.get() || shown_profile_name(profiles).is_some() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if dropdown_open.get() || shown_profile_name(profiles).is_some() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if saved.with(Vec::is_empty) {
                    bg_control()
                } else if dropdown_open.get() || shown_profile_name(profiles).is_some() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| {
            style.bg(move || {
                if saved.with(Vec::is_empty) {
                    bg_control()
                } else {
                    bg_control_active()
                }
            })
        })
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .flex_row()
        .items_center()
        .gap_1p5()
        .disabled(move || saved.with(Vec::is_empty))
        .disabled_style(|style| style.opacity(0.6))
        .tooltip(
            tooltip()
                .disabled(move || dropdown_open.get())
                .child(ui_tooltip_surface(
                    text(move || -> SharedString {
                        let lang = ui_lang(langs);
                        if saved.with(Vec::is_empty) {
                            t(
                                lang,
                                "暂无已保存预设，可在下方填写名称后点保存",
                                "No saved presets yet — enter a name below and click Save",
                            )
                        } else {
                            t(lang, "选择或切换已保存预设", "Switch or clear saved preset")
                        }
                        .into()
                    })
                    .text_size(px(TEXT_SM))
                    .text_color(fg_primary()),
                )),
        )
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .flex_shrink_0()
                .bg(move || {
                    if shown_profile_name(profiles).is_some() {
                        accent_blue()
                    } else {
                        fg_muted()
                    }
                }),
        )
        .child(
            text(move || -> SharedString {
                let lang = ui_lang(langs);
                if let Some(name) = shown_profile_name(profiles) {
                    name.into()
                } else if saved.with(Vec::is_empty) {
                    t(lang, "暂无已保存预设", "No saved presets").into()
                } else {
                    t(lang, "选择已保存预设", "Saved presets").into()
                }
            })
            .flex_auto()
            .min_w(px(0.0))
            .number_of_lines(1)
            .text_overflow(EllipsizeMode::Tail)
            .text_size(px(TEXT_SM))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || {
                if shown_profile_name(profiles).is_some() {
                    accent_blue_text()
                } else if saved.with(Vec::is_empty) {
                    fg_muted()
                } else {
                    fg_secondary()
                }
            }),
        )
        .child(when(
            move || !saved.with(Vec::is_empty),
            move || menu_count_badge(move || saved.with(Vec::len)),
        ))
        .child(when(
            move || !saved.with(Vec::is_empty),
            move || glyph_px(12.0, || Glyph::Chevron, || color(fg_muted())),
        ))
}

fn profile_saved_popup(
    ctx: ProfileDockMenuCtx,
    confirm_delete: Signal<Option<String>>,
) -> impl IntoElement {
    let profiles = ctx.profiles;
    let saved = profiles.saved;
    let deselect_ctx = ctx.clone();
    view()
        .w(px(360.0))
        .max_w(move || px((window().width.0 - 24.0).clamp(220.0, 360.0)))
        .max_h(move || px((window().height.0 - 96.0).clamp(140.0, 220.0)))
        .overflow_hidden()
        .child(
            scroll()
                .w_full()
                .max_h(move || px((window().height.0 - 98.0).clamp(138.0, 218.0)))
                .child(
                    view()
                        .w_full()
                        .p_1()
                        .gap_0p5()
                        .child(when(
                            move || shown_profile_name(profiles).is_some(),
                            move || deselect_profile_section(deselect_ctx.clone(), confirm_delete),
                        ))
                        .child(each(saved, saved_profile_key, move |item, _| {
                            saved_profile_menu_row(item, ctx.clone(), confirm_delete)
                        })),
                ),
        )
}

fn deselect_profile_section(
    ctx: ProfileDockMenuCtx,
    confirm_delete: Signal<Option<String>>,
) -> impl IntoElement {
    let profiles = ctx.profiles;
    let adopted = ctx.adopted;
    let dropdown_open = ctx.dropdown_open;
    let clear_controls = ctx.controls;
    view()
        .w_full()
        .gap_1()
        .child(
            menu_item()
                .w_full()
                .h(px(CONTROL_H))
                .px_2()
                .rounded(px(RADIUS_MD))
                .flex_row()
                .items_center()
                .gap_2()
                .hover(|style| style.bg(bg_control_hover()))
                .active(|style| style.bg(bg_control_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .on_select(move || {
                    confirm_delete.set(None);
                    adopted.set(true);
                    deselect_active_profile(profiles, &clear_controls);
                    dropdown_open.set(false);
                })
                .child(glyph_px(12.0, || Glyph::Close, || color(fg_muted())))
                .child(
                    text(tr("取消选择当前预设", "Deselect current preset"))
                        .flex_1()
                        .min_w(px(0.0))
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg_secondary()),
                ),
        )
        .child(view().w_full().h(px(1.0)).my_0p5().bg(border_subtle()))
}

fn saved_profile_menu_row(
    item: ReadSignal<config::SavedProfile>,
    ctx: ProfileDockMenuCtx,
    confirm_delete: Signal<Option<String>>,
) -> impl IntoElement {
    let profiles = ctx.profiles;
    let draft = ctx.draft;
    let adopted = ctx.adopted;
    let dropdown_open = ctx.dropdown_open;
    let preset_controls = ctx.controls.clone();
    let remove_controls = ctx.controls;
    let is_active = move || is_saved_profile_active(profiles, item);
    let is_confirming = move || {
        item.with(|preset| {
            confirm_delete.with(|pending| {
                pending
                    .as_deref()
                    .is_some_and(|p| p.eq_ignore_ascii_case(&preset.name))
            })
        })
    };
    view()
        .h(px(30.0))
        .gap_1()
        .flex_row()
        .items_center()
        .child(
            menu_item()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .px_2()
                .rounded(px(RADIUS_MD))
                .overflow_hidden()
                .flex_row()
                .items_center()
                .gap_2()
                .border_1()
                .border_color(move || {
                    if is_active() {
                        accent_blue_border()
                    } else {
                        AdaptiveColor::TRANSPARENT
                    }
                })
                .bg(move || {
                    if is_active() {
                        accent_blue_soft()
                    } else {
                        AdaptiveColor::TRANSPARENT
                    }
                })
                .hover(|style| {
                    style.bg(move || {
                        if is_active() {
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
                .on_select(move || {
                    confirm_delete.set(None);
                    select_saved_profile_row(
                        item,
                        profiles,
                        draft,
                        adopted,
                        dropdown_open,
                        &preset_controls,
                    );
                })
                .child(
                    view()
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded_full()
                        .flex_shrink_0()
                        .bg(move || {
                            if is_active() {
                                accent_blue()
                            } else {
                                fg_muted()
                            }
                        }),
                )
                .child(profile_name_chip(
                    move || item.with(|preset| preset.name.clone()),
                    124.0,
                ))
                .child(
                    control_text(move || item.with(profile_preview))
                        .flex_1()
                        .min_w(px(0.0))
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .text_color(fg_secondary()),
                )
                .child(when(is_active, || {
                    text(tr("点击取消", "Click to clear"))
                        .flex_shrink_0()
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(accent_blue_text())
                })),
        )
        .child(
            when(is_confirming, move || {
                let remove_controls = remove_controls.clone();
                view()
                    .flex_shrink_0()
                    .tooltip(label_tip(
                        "再次点击确认删除预设",
                        "Click again to delete preset",
                    ))
                    .child(
                        button()
                            .h(px(ControlSize::Xs.height()))
                            .px_2()
                            .flex_shrink_0()
                            .flex_row()
                            .items_center()
                            .justify_center()
                            .rounded(px(RADIUS_SM))
                            .bg(danger_button_bg())
                            .hover(|style| style.bg(danger_button_hover()))
                            .active(|style| style.bg(danger_button_active()))
                            .transition_colors()
                            .duration(MOTION_INSTANT_MS)
                            .ease_out()
                            .focus_visible(apply_focus_ring)
                            .accessibility_label(tr_opt(
                                "再次点击确认删除预设",
                                "Click again to delete preset",
                            ))
                            .on_press(move |_: &()| {
                                let target = item.with(Clone::clone);
                                let deleted_name = target.name.clone();
                                confirm_delete.set(None);
                                adopted.set(true);
                                drop_saved_profile(
                                    &target,
                                    profiles.name,
                                    profiles.saved,
                                    profiles.history,
                                    &remove_controls,
                                );
                                set_status(
                                    &remove_controls,
                                    StatusMsg::PresetDeleted(deleted_name),
                                );
                                if profiles.saved.with(Vec::is_empty) {
                                    dropdown_open.set(false);
                                }
                            })
                            .child(
                                compact_control_text(tr("确认删除？", "Delete?"), TEXT_XS)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(fg_on_accent()),
                            ),
                    )
            })
            .otherwise(move || {
                view()
                    .flex_shrink_0()
                    .tooltip(label_tip(
                        "删除此预设（点击两次确认）",
                        "Delete preset (click twice to confirm)",
                    ))
                    .child(
                        button()
                            .w(px(26.0))
                            .h(px(26.0))
                            .flex_shrink_0()
                            .flex_row()
                            .items_center()
                            .justify_center()
                            .rounded(px(RADIUS_MD))
                            .hover(|style| style.bg(bg_control_hover()))
                            .active(|style| style.bg(bg_control_active()))
                            .transition_colors()
                            .duration(MOTION_INSTANT_MS)
                            .ease_out()
                            .focus_visible(apply_focus_ring)
                            .accessibility_label(tr_opt("删除预设", "Delete preset"))
                            .on_press(move |_: &()| {
                                let name = item.with_untracked(|preset| preset.name.clone());
                                confirm_delete.set(Some(name));
                            })
                            .child(glyph_px(12.0, || Glyph::Trash, || color(fg_muted()))),
                    )
            }),
        )
}

fn is_saved_profile_active(
    profiles: ProfileSignals,
    item: ReadSignal<config::SavedProfile>,
) -> bool {
    let typed = profiles.name.get();
    let typed = typed.trim();
    if !typed.is_empty() {
        return item.with(|preset| preset.name.eq_ignore_ascii_case(typed));
    }
    let f = profiles.filter.get();
    let b = profiles.bypass.get();
    let r = profiles.rules.get();
    let u = profiles.upstream.get();
    if f.trim().is_empty() && b.trim().is_empty() && r.trim().is_empty() && u.trim().is_empty() {
        return false;
    }
    item.with(|preset| {
        preset.filter.trim() == f.trim()
            && preset.bypass_hosts.trim() == b.trim()
            && preset.rules.trim() == r.trim()
            && preset.upstream_proxy.trim() == u.trim()
    })
}

fn select_saved_profile_row(
    item: ReadSignal<config::SavedProfile>,
    profiles: ProfileSignals,
    draft: QuickRuleDraftSignals,
    adopted: Signal<bool>,
    dropdown_open: Signal<bool>,
    controls: &Arc<Controls>,
) {
    if is_saved_profile_active(profiles, item) {
        adopted.set(true);
        deselect_active_profile(profiles, controls);
        dropdown_open.set(false);
        return;
    }
    let preset = item.with(Clone::clone);
    record_profile_history(
        &profiles.name.get(),
        &profiles.filter.get(),
        &profiles.bypass.get(),
        &profiles.rules.get(),
        &profiles.upstream.get(),
        profiles.rules_on.get(),
        profiles.breaks_on.get(),
        profiles.show_tunnels.get(),
        profiles.history,
    );
    profiles.name.set(preset.name.clone());
    clear_quick_draft(draft);
    apply_saved_profile(
        &preset,
        profiles.filter,
        profiles.bypass,
        profiles.rules,
        profiles.upstream,
        profiles.rules_on,
        profiles.breaks_on,
        profiles.show_tunnels,
        controls,
    );
    record_profile_history(
        &preset.name,
        &preset.filter,
        &preset.bypass_hosts,
        &preset.rules,
        &preset.upstream_proxy,
        preset.rules_enabled,
        preset.breaks_enabled,
        preset.show_tunnel_failures,
        profiles.history,
    );
    dropdown_open.set(false);
    set_status(controls, StatusMsg::PresetSwitched(preset.name));
}

fn profile_name_chip(name: impl Fn() -> String + 'static, max_w: f32) -> impl IntoElement {
    view()
        .h(px(18.0))
        .max_w(px(max_w))
        .px_1p5()
        .rounded(px(RADIUS_SM))
        .flex_shrink_0()
        .overflow_hidden()
        .flex_row()
        .items_center()
        .bg(accent_blue_soft())
        .child(
            text(name)
                .max_w(px(max_w - 12.0))
                .min_w(px(0.0))
                .number_of_lines(1)
                .text_overflow(EllipsizeMode::Tail)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::MEDIUM)
                .text_color(accent_blue_text()),
        )
}

/// Recent setup history dropdown menu inside the Preset popover dock.
pub(crate) fn profile_history_menu(ctx: ProfileDockMenuCtx) -> impl IntoElement {
    let history = ctx.profiles.history;
    let history_open = ctx.history_open;
    let dropdown_open = ctx.dropdown_open;
    bind_escape_while(
        move || history_open.get() && !history.with(Vec::is_empty),
        nsh_core::keymap::When::Always,
        move || history_open.set(false),
    );
    let trigger_ctx = ctx.clone();
    menu()
        .open(move || Some(history_open.get() && !history.with(Vec::is_empty)))
        .on_open_change(move |event: &nsh_base::MenuEvent| {
            if let nsh_base::MenuEvent::OpenChange(next, _) = event {
                history_open.set(*next && !history.with(Vec::is_empty));
                if *next {
                    dropdown_open.set(false);
                }
            }
        })
        .placement(Placement::Bottom)
        .align(Align::Start)
        .offset(px(6.0))
        .margin(px(8.0))
        .popup_style(ui_popup_style)
        .popup_starting_style(ui_popup_starting_style)
        .popup_ending_style(ui_popup_ending_style)
        .trigger(profile_history_trigger(trigger_ctx))
        .child(profile_history_popup(ctx))
}

fn profile_history_trigger(ctx: ProfileDockMenuCtx) -> impl IntoElement {
    let langs = ui_langs();
    let history = ctx.profiles.history;
    let history_open = ctx.history_open;
    view()
        .role(AccessibilityRole::Button)
        .h(px(CONTROL_H))
        .px_2()
        .flex_shrink_0()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(move || {
            if history_open.get() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if history_open.get() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if history.with(Vec::is_empty) {
                    bg_control()
                } else if history_open.get() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| {
            style.bg(move || {
                if history.with(Vec::is_empty) {
                    bg_control()
                } else {
                    bg_control_active()
                }
            })
        })
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .flex_row()
        .items_center()
        .gap_1()
        .disabled(move || history.with(Vec::is_empty))
        .disabled_style(|style| style.opacity(0.6))
        .tooltip(
            tooltip()
                .disabled(move || history_open.get())
                .child(ui_tooltip_surface(
                    text(move || -> SharedString {
                        let lang = ui_lang(langs);
                        if history.with(Vec::is_empty) {
                            t(lang, "暂无历史版本", "No version history yet")
                        } else {
                            t(
                                lang,
                                "最近 10 个预设历史版本，点击可恢复",
                                "Last 10 preset versions, click to restore",
                            )
                        }
                        .into()
                    })
                    .text_size(px(TEXT_SM))
                    .text_color(fg_primary()),
                )),
        )
        .child(glyph_px(
            12.0,
            || Glyph::History,
            move || {
                color(if history_open.get() {
                    accent_blue_text()
                } else if history.with(Vec::is_empty) {
                    fg_muted()
                } else {
                    fg_secondary()
                })
            },
        ))
        .child(
            text(tr("历史", "History"))
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if history_open.get() {
                        accent_blue_text()
                    } else if history.with(Vec::is_empty) {
                        fg_muted()
                    } else {
                        fg_secondary()
                    }
                }),
        )
        .child(when(
            move || !history.with(Vec::is_empty),
            move || menu_count_badge(move || history.with(Vec::len)),
        ))
}

fn profile_history_popup(ctx: ProfileDockMenuCtx) -> impl IntoElement {
    let history = ctx.profiles.history;
    view()
        .w(px(380.0))
        .max_w(move || px((window().width.0 - 24.0).clamp(220.0, 380.0)))
        .max_h(move || px((window().height.0 - 96.0).clamp(140.0, 240.0)))
        .overflow_hidden()
        .child(
            scroll()
                .w_full()
                .max_h(move || px((window().height.0 - 98.0).clamp(138.0, 238.0)))
                .child(view().w_full().p_1().gap_0p5().child(each(
                    history,
                    history_entry_key,
                    move |item, index| history_profile_menu_row(item, index, ctx.clone()),
                ))),
        )
}

fn history_profile_menu_row(
    item: ReadSignal<config::SavedProfile>,
    index: ReadSignal<usize>,
    ctx: ProfileDockMenuCtx,
) -> impl IntoElement {
    let profiles = ctx.profiles;
    let draft = ctx.draft;
    let history_controls = ctx.controls;
    let is_current = move || {
        item.with(|preset| {
            preset.filter.trim() == profiles.filter.get().trim()
                && preset.bypass_hosts.trim() == profiles.bypass.get().trim()
                && preset.rules.trim() == profiles.rules.get().trim()
                && preset.upstream_proxy.trim() == profiles.upstream.get().trim()
                && preset.rules_enabled == profiles.rules_on.get()
                && preset.breaks_enabled == profiles.breaks_on.get()
                && preset.show_tunnel_failures == profiles.show_tunnels.get()
        })
    };
    menu_item()
        .w_full()
        .h(px(30.0))
        .px_2()
        .rounded(px(RADIUS_MD))
        .overflow_hidden()
        .flex_row()
        .items_center()
        .gap_1p5()
        .border_1()
        .border_color(move || {
            if is_current() {
                accent_blue_border()
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .bg(move || {
            if is_current() {
                accent_blue_soft()
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .hover(|style| {
            style.bg(move || {
                if is_current() {
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
        .on_select(move || {
            let preset = item.with(Clone::clone);
            record_profile_history(
                &profiles.name.get(),
                &profiles.filter.get(),
                &profiles.bypass.get(),
                &profiles.rules.get(),
                &profiles.upstream.get(),
                profiles.rules_on.get(),
                profiles.breaks_on.get(),
                profiles.show_tunnels.get(),
                profiles.history,
            );
            profiles.name.set(preset.name.clone());
            clear_quick_draft(draft);
            apply_saved_profile(
                &preset,
                profiles.filter,
                profiles.bypass,
                profiles.rules,
                profiles.upstream,
                profiles.rules_on,
                profiles.breaks_on,
                profiles.show_tunnels,
                &history_controls,
            );
            set_status(
                &history_controls,
                StatusMsg::ProfileRestored(preset.name.clone()),
            );
        })
        .child(
            mono(move || format!("#{}", index.get() + 1))
                .tabular_nums()
                .text_size(px(TEXT_XS))
                .text_color(fg_muted()),
        )
        .child(profile_name_chip(
            move || item.with(|preset| preset.name.clone()),
            120.0,
        ))
        .child(
            control_text(move || item.with(profile_preview))
                .flex_1()
                .min_w(px(0.0))
                .number_of_lines(1)
                .text_overflow(EllipsizeMode::Tail)
                .text_color(fg_secondary()),
        )
}
