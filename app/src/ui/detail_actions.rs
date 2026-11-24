#[allow(unused_imports)]
use super::*;

pub(crate) fn summary_primary_actions(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
) -> impl IntoElement {
    let langs = ui_langs();
    let pin_controls = controls.clone();
    let replay_controls = controls.clone();
    let pin_guard = controls.clone();
    let replay_guard = controls;
    let can_pin = move || {
        let _ = selected.get();
        shown.relay.get().is_empty() && !is_active_tab_read_only(&pin_guard)
    };
    let can_replay = move || {
        let _ = selected.get();
        shown.relay.get().is_empty() && !is_active_tab_read_only(&replay_guard)
    };
    view()
        .flex_row()
        .items_center()
        .gap_1p5()
        .child(
            view()
                .tooltip(label_tip(
                    "星标此流量（清空未星标时保留）",
                    "Star flow (kept when clearing unstarred)",
                ))
                .child(
                    ui_toggle_button(ControlSize::Sm, move || shown.pinned.get())
                        .accessibility_label(move || {
                            let lang = ui_lang(langs);
                            Some(
                                if shown.pinned.get() {
                                    t(lang, "取消星标", "Unstar")
                                } else {
                                    t(lang, "星标", "Star")
                                }
                                .into(),
                            )
                        })
                        .disabled(move || !can_pin())
                        .border_color(move || {
                            if shown.pinned.get() {
                                accent_blue_border()
                            } else {
                                border_control()
                            }
                        })
                        .bg(move || {
                            if shown.pinned.get() {
                                accent_blue_soft()
                            } else {
                                bg_control()
                            }
                        })
                        .on_press(move || toggle_pin(&pin_controls, selected.get()))
                        .child(glyph(
                            move || {
                                if shown.pinned.get() {
                                    Glyph::StarFilled
                                } else {
                                    Glyph::Star
                                }
                            },
                            move || {
                                color(if shown.pinned.get() {
                                    accent_blue_text()
                                } else {
                                    fg_secondary()
                                })
                            },
                        ))
                        .child(
                            text(move || -> SharedString {
                                let lang = ui_lang(langs);
                                if shown.pinned.get() {
                                    t(lang, "已星标", "Starred").into()
                                } else {
                                    t(lang, "星标", "Star").into()
                                }
                            })
                            .number_of_lines(1)
                            .text_size(px(TEXT_SM))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || {
                                if shown.pinned.get() {
                                    accent_blue_text()
                                } else {
                                    fg_secondary()
                                }
                            }),
                        ),
                ),
        )
        .child(
            view()
                .tooltip(label_tip(
                    "重放请求 (Cmd/Ctrl+R)",
                    "Replay request (Cmd/Ctrl+R)",
                ))
                .child(
                    ui_button(ButtonVariant::Outline, ControlSize::Sm)
                        .accessibility_label(move || {
                            Some(t(ui_lang(langs), "重放", "Replay").into())
                        })
                        .disabled(move || !can_replay())
                        .on_press(move || {
                            replay_flow(
                                &replay_controls,
                                selected.get(),
                                ReplayOverride::default(),
                            );
                        })
                        .child(glyph(|| Glyph::Replay, || color(fg_secondary())))
                        .child(control_text(tr("重放", "Replay")).text_color(fg_secondary())),
                ),
        )
}

fn code_snippets_section(
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    code_open: Signal<bool>,
) -> impl IntoElement {
    let langs = ui_langs();
    let entries: [(Option<(&'static str, &'static str)>, &'static str, Snippet); 11] = [
        (None, "fetch", Snippet::Fetch),
        (None, "Python", Snippet::Python),
        (None, "Rust (reqwest)", Snippet::RustReqwest),
        (None, "Go (net/http)", Snippet::GoHttp),
        (None, "HTTPie", Snippet::Httpie),
        (None, "PowerShell", Snippet::PowerShell),
        (None, "JS (axios)", Snippet::Axios),
        (None, "Kotlin (OkHttp)", Snippet::KotlinOkHttp),
        (None, "Swift (URLSession)", Snippet::SwiftUrlSession),
        (Some(("TS 类型", "TS types")), "TS types", Snippet::Types),
        (
            Some(("Rust 结构体", "Rust struct")),
            "Rust struct",
            Snippet::RustStruct,
        ),
    ];
    view()
        .w_full()
        .gap_0p5()
        .child(
            menu_item()
                .close_on_select(false)
                .on_select(move || code_open.update(|v| *v = !*v))
                .h(px(30.0))
                .px_2p5()
                .gap_1p5()
                .rounded(px(RADIUS_MD))
                .flex_row()
                .items_center()
                .justify_between()
                .hover(|style| style.bg(bg_control_hover()))
                .active(|style| style.bg(bg_control_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .child(
                    control_text(move || -> SharedString {
                        t(ui_lang(langs), "复制为代码 (11)", "Copy as code (11)").into()
                    })
                    .text_color(fg_secondary()),
                )
                .child(glyph_px(
                    11.0,
                    move || {
                        if code_open.get() {
                            Glyph::Chevron
                        } else {
                            Glyph::ChevronRight
                        }
                    },
                    || color(fg_muted()),
                )),
        )
        .child(when(
            move || code_open.get(),
            move || {
                let mut group = view().w_full().pl_2().gap_0p5();
                for (localized, raw, snippet) in entries {
                    let cb = controls.clone();
                    group = group.child(match localized {
                        Some((zh, en)) => action_item(tr(zh, en), move || {
                            copy_snippet(&cb, selected.get(), snippet);
                        })
                        .into_element(),
                        None => action_item(raw, move || {
                            copy_snippet(&cb, selected.get(), snippet);
                        })
                        .into_element(),
                    });
                }
                group
            },
        ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn detail_actions(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
    compose: Compose,
    filter: Signal<String>,
    profiles: ProfileSignals,
    baseline: Signal<Option<u64>>,
) -> impl IntoElement {
    let rules_text = profiles.rules;
    let copy_url = controls.clone();
    let copy_curl = controls.clone();
    let copy_ai = controls.clone();
    let code_controls = controls.clone();
    let copy_request = controls.clone();
    let copy_response = controls.clone();
    let save = controls.clone();
    let focus_controls = controls.clone();
    let mute_controls = controls.clone();
    let pin_controls = controls.clone();
    let base_controls = controls.clone();
    let replay_controls = controls.clone();
    let repeat_controls = controls.clone();
    let edit_controls = controls.clone();
    let send_controls = controls.clone();
    let break_controls = controls.clone();
    let map_local_controls = controls.clone();
    let quick_rule_controls = controls.clone();
    let mutate_guard = controls.clone();
    let pretty_controls = controls;
    let pinned = shown.pinned;
    let pretty = shown.pretty;
    let baseline_mark = baseline;
    let selected_mark = selected;
    let can_local = move || shown.relay.get().is_empty();
    let can_mutate = move || {
        let _ = selected.get();
        shown.relay.get().is_empty() && !is_active_tab_read_only(&mutate_guard)
    };
    let is_req_break = move || {
        !compose.response.get()
            && paused
                .get()
                .is_some_and(|(id, side)| selected.get() == Some(id) && side == BreakSide::Request)
    };
    let open = signal(false);
    let code_open = signal(false);
    let items = view()
        .w_full()
        .p_1()
        .gap_0p5()
        .child(action_item(tr("复制 URL", "Copy URL"), move || {
            copy_part(&copy_url, selected.get(), CopyPart::Url);
        }))
        .child(action_item(tr("复制 cURL", "Copy cURL"), move || {
            copy_part(&copy_curl, selected.get(), CopyPart::Curl);
        }))
        .child(action_item(tr("复制给 AI", "Copy for AI"), move || {
            copy_snippet(&copy_ai, selected.get(), Snippet::AiContext);
        }))
        .child(code_snippets_section(code_controls, selected, code_open))
        .child(action_item(tr("复制请求", "Copy request"), move || {
            copy_part(&copy_request, selected.get(), CopyPart::Request);
        }))
        .child(action_item(
            tr("复制响应", "Copy response"),
            move || {
                copy_part(&copy_response, selected.get(), CopyPart::Response);
            },
        ));
    #[cfg(not(target_arch = "wasm32"))]
    let items = items.child(action_item(
        tr("另存响应…", "Save response…"),
        move || {
            export_response(save.clone(), selected.get());
        },
    ));
    #[cfg(target_arch = "wasm32")]
    let items = {
        let save_web = save.clone();
        let has_res_body = move || {
            !shown.response_body.get().is_empty()
                || !shown.response_hex.with(Vec::is_empty)
                || !shown.preview.get().is_empty()
                || selected
                    .get()
                    .and_then(|id| selected_flow(&save, Some(id)))
                    .is_some_and(|flow| !flow.response_body.is_empty())
        };
        items.child(action_item_enabled(
            tr("另存响应…", "Save response…"),
            has_res_body,
            move || {
                export_response_web(save_web.clone(), selected.get());
            },
        ))
    };
    let items = items
        .child(menu_rule())
        .child(action_item(tr("只看", "Focus"), move || {
            focus_host(&focus_controls, selected.get(), filter);
        }))
        .child(action_item(tr("屏蔽", "Mute"), move || {
            mute_host(&mute_controls, selected.get(), filter);
        }))
        .child(menu_rule())
        .child(marked_action(
            move || pinned.get(),
            {
                let langs = ui_langs();
                move || -> SharedString {
                    if pinned.get() {
                        t(ui_lang(langs), "取消星标", "Unstar").into()
                    } else {
                        t(ui_lang(langs), "星标", "Star").into()
                    }
                }
            },
            move || toggle_pin(&pin_controls, selected.get()),
        ))
        .child(marked_action(
            move || {
                baseline_mark
                    .get()
                    .is_some_and(|id| selected_mark.get() == Some(id))
            },
            {
                let langs = ui_langs();
                let selected_label = selected;
                let baseline_label = baseline;
                move || -> SharedString {
                    if baseline_label
                        .get()
                        .is_some_and(|id| selected_label.get() == Some(id))
                    {
                        t(ui_lang(langs), "取消基准", "Clear baseline").into()
                    } else {
                        t(ui_lang(langs), "基准", "Baseline").into()
                    }
                }
            },
            {
                let langs = ui_langs();
                move || {
                    set_baseline(
                        &base_controls,
                        selected.get(),
                        baseline,
                        &shown,
                        ui_lang(langs),
                    );
                }
            },
        ))
        .child(action_item(tr("备注…", "Note…"), move || {
            shown.repeating.set(false);
            shown.editing_comment.set(true);
        }))
        .child(marked_action(
            move || pretty.get(),
            tr("美化", "Pretty"),
            {
                let langs = ui_langs();
                move || {
                    shown.req_doc_preview.set(false);
                    shown.res_doc_preview.set(false);
                    shown.req_json_tree.set(false);
                    shown.res_json_tree.set(false);
                    shown.pretty.set(!shown.pretty.get());
                    reset_body_offsets(shown);
                    publish_detail_for_controls(
                        &shown,
                        &pretty_controls,
                        selected.get(),
                        ui_lang(langs),
                        baseline.get(),
                    );
                }
            },
        ))
        .child(menu_rule())
        .child(action_item_enabled(
            tr("重放", "Replay"),
            can_mutate.clone(),
            move || {
                replay_flow(&replay_controls, selected.get(), ReplayOverride::default());
            },
        ))
        .child(action_item_enabled(
            tr("批量重放…", "Repeat…"),
            can_mutate.clone(),
            move || {
                if refuse_relay(&repeat_controls, selected.get()) {
                    return;
                }
                shown.editing_comment.set(false);
                shown.repeating.set(true);
            },
        ))
        .child(action_item_enabled(
            {
                let langs = ui_langs();
                move || -> SharedString {
                    edit_action_label(ui_lang(langs), selected.get(), paused.get()).into()
                }
            },
            can_mutate,
            move || {
                begin_compose(&edit_controls, selected.get(), compose);
            },
        ))
        .child(action_item_enabled(
            tr("填入断点", "Add break"),
            can_local,
            move || {
                fill_break_rule(&break_controls, selected.get(), rules_text);
            },
        ))
        .child(action_item_enabled(
            {
                let langs = ui_langs();
                move || -> SharedString {
                    #[cfg(not(target_arch = "wasm32"))]
                    let saves_file = !shown.response_body.get().is_empty()
                        || !shown.response_hex.with(Vec::is_empty);
                    #[cfg(target_arch = "wasm32")]
                    let saves_file = false;
                    if saves_file {
                        t(ui_lang(langs), "本地映射…", "Map local…").into()
                    } else {
                        t(ui_lang(langs), "本地映射", "Map local").into()
                    }
                }
            },
            can_local,
            move || {
                map_local_from_flow(map_local_controls.clone(), selected.get(), rules_text);
            },
        ))
        .child(flow_quick_rule_section_opts(
            false,
            can_local,
            move |action| {
                open_quick_rule_for_flow(&quick_rule_controls, profiles, selected.get(), action);
            },
        ))
        .child(when(
            move || compose.composing.get() && !compose.response.get(),
            move || {
                let controls = send_controls.clone();
                let langs = ui_langs();
                action_item_enabled(
                    move || -> SharedString {
                        if is_req_break() {
                            t(ui_lang(langs), "放行", "Continue").into()
                        } else {
                            t(ui_lang(langs), "发送", "Send").into()
                        }
                    },
                    can_local,
                    move || {
                        if is_req_break()
                            || controls.hub.paused().is_some_and(|(id, side)| {
                                selected.get() == Some(id) && side == BreakSide::Request
                            })
                        {
                            continue_break(&controls, compose, selected, paused);
                            return;
                        }
                        replay_flow(
                            &controls,
                            selected.get(),
                            ReplayOverride {
                                method: Some(compose.method.get()),
                                url: Some(compose.url.get()),
                                headers: Some(parse_header_text(&compose.headers.get())),
                                body: Some(compose.body.get().into_bytes()),
                            },
                        );
                    },
                )
            },
        ));
    menu()
        .on_open_change(move |event: &nsh_base::MenuEvent| {
            if let nsh_base::MenuEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(8.0))
        .popup_style(ui_popup_style)
        .popup_starting_style(ui_popup_starting_style)
        .trigger(detail_more_trigger(open))
        .child(
            view()
                .w(px(208.0))
                .max_w(move || px((window().width.0 - 16.0).max(160.0)))
                .max_h(move || px((window().height.0 - 96.0).max(160.0)))
                .overflow_hidden()
                .child(
                    scroll()
                        .w_full()
                        .max_h(move || px((window().height.0 - 98.0).max(158.0)))
                        .child(items),
                ),
        )
}

pub(crate) fn detail_more_trigger(open: Signal<bool>) -> impl IntoElement {
    view()
        .role(AccessibilityRole::Button)
        .h(px(CONTROL_H))
        .px_2()
        .gap_1()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(move || {
            if open.get() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if open.get() {
                accent_blue_soft()
            } else {
                bg_control()
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
        .child(
            text(tr("更多", "More"))
                .number_of_lines(1)
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if open.get() {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
        )
        .child(glyph(
            || Glyph::Chevron,
            move || {
                color(if open.get() {
                    accent_blue_text()
                } else {
                    fg_secondary()
                })
            },
        ))
}

pub(crate) fn action_item(
    label: impl IntoReactive<SharedString>,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    action_item_enabled(label, || true, on_select)
}

pub(crate) fn action_item_enabled(
    label: impl IntoReactive<SharedString>,
    enabled: impl Fn() -> bool + 'static + Clone,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    let disabled_check = enabled.clone();
    let hover_check = enabled.clone();
    let active_check = enabled.clone();
    let fg_check = enabled;
    menu_item()
        .disabled(move || !disabled_check())
        .disabled_style(|style| style.opacity(0.45))
        .on_select(on_select)
        .h(px(30.0))
        .px_2p5()
        .rounded(px(RADIUS_MD))
        .flex_row()
        .items_center()
        .hover(|style| {
            style.bg(move || {
                if hover_check() {
                    bg_control_hover()
                } else {
                    AdaptiveColor::TRANSPARENT
                }
            })
        })
        .active(|style| {
            style.bg(move || {
                if active_check() {
                    bg_control_active()
                } else {
                    AdaptiveColor::TRANSPARENT
                }
            })
        })
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .child(control_text(label).text_color(move || {
            if fg_check() {
                fg_secondary()
            } else {
                fg_muted()
            }
        }))
}

pub(crate) fn marked_action(
    marked: impl Fn() -> bool + 'static + Clone,
    label: impl IntoReactive<SharedString>,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    marked_action_inner(marked, label, true, on_select)
}

pub(crate) fn marked_radio_stay(
    marked: impl Fn() -> bool + 'static + Clone,
    label: impl IntoReactive<SharedString>,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    marked_action_inner(marked, label, false, on_select)
}

pub(crate) fn marked_action_inner(
    marked: impl Fn() -> bool + 'static + Clone,
    label: impl IntoReactive<SharedString>,
    close_on_select: bool,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    let dot = marked.clone();
    let dot_fill = marked.clone();
    let label_on = marked;
    menu_item()
        .close_on_select(close_on_select)
        .on_select(on_select)
        .h(px(30.0))
        .px_2p5()
        .gap_2()
        .rounded(px(RADIUS_MD))
        .flex_row()
        .items_center()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .child(
            view()
                .w(px(8.0))
                .h(px(8.0))
                .rounded_full()
                .border_1()
                .border_color(move || if dot() { accent_blue() } else { fg_muted() })
                .bg(move || if dot_fill() { accent_blue() } else { bg_card() }),
        )
        .child(control_text(label).text_color(move || {
            if label_on() {
                accent_blue_text()
            } else {
                fg_secondary()
            }
        }))
}

pub(crate) fn detail_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .role(AccessibilityRole::Button)
        .h(px(CONTROL_H))
        .px_2()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border_control())
        .bg(bg_control())
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .on_press(on_press)
        .child(
            text(label)
                .number_of_lines(1)
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg_secondary()),
        )
}

pub(crate) fn section_title(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(TEXT_SM))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg_secondary())
}

pub(crate) fn muted_line(label: impl IntoReactive<SharedString>) -> Text {
    text(label).text_size(px(TEXT_SM)).text_color(fg_muted())
}

pub(crate) fn cert_display_name(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|seg| !seg.is_empty())
        .unwrap_or(path)
}
