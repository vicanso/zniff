#[allow(unused_imports)]
use super::*;

pub(crate) fn detail_actions(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
    compose: Compose,
    filter: Signal<String>,
    rules_text: Signal<String>,
    baseline: Signal<Option<u64>>,
) -> impl IntoElement {
    let copy_url = controls.clone();
    let copy_curl = controls.clone();
    let copy_fetch = controls.clone();
    let copy_python = controls.clone();
    let copy_rust = controls.clone();
    let copy_go = controls.clone();
    let copy_types = controls.clone();
    let copy_rust_struct = controls.clone();
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
    let pretty_controls = controls;
    let pinned = shown.pinned;
    let pretty = shown.pretty;
    let baseline_mark = baseline;
    let selected_mark = selected;
    let can_local = move || shown.relay.get().is_empty();
    let is_req_break = move || {
        !compose.response.get()
            && paused
                .get()
                .is_some_and(|(id, side)| selected.get() == Some(id) && side == BreakSide::Request)
    };
    let open = signal(false);
    let items = view()
        .w_full()
        .p_1()
        .gap_0p5()
        .child(action_item(tr("复制 URL", "Copy URL"), move || {
            copy_part(&copy_url, selected.get(), CopyPart::Url);
        }))
        .child(action_item("cURL", move || {
            copy_part(&copy_curl, selected.get(), CopyPart::Curl);
        }))
        .child(action_item("fetch", move || {
            copy_snippet(&copy_fetch, selected.get(), Snippet::Fetch);
        }))
        .child(action_item("Python", move || {
            copy_snippet(&copy_python, selected.get(), Snippet::Python);
        }))
        .child(action_item("Rust (reqwest)", move || {
            copy_snippet(&copy_rust, selected.get(), Snippet::RustReqwest);
        }))
        .child(action_item("Go (net/http)", move || {
            copy_snippet(&copy_go, selected.get(), Snippet::GoHttp);
        }))
        .child(action_item(tr("TS 类型", "TS types"), move || {
            copy_snippet(&copy_types, selected.get(), Snippet::Types);
        }))
        .child(action_item(
            tr("Rust 结构体", "Rust struct"),
            move || {
                copy_snippet(&copy_rust_struct, selected.get(), Snippet::RustStruct);
            },
        ))
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
                    publish_detail(
                        &shown,
                        &pretty_controls.hub,
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
            can_local,
            move || {
                replay_flow(&replay_controls, selected.get(), ReplayOverride::default());
            },
        ))
        .child(action_item_enabled(
            tr("批量重放…", "Repeat…"),
            can_local,
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
            can_local,
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
        .trigger(detail_more_trigger(open))
        .child(
            view()
                .w(px(208.0))
                .max_w(move || px((window().width.0 - 16.0).max(160.0)))
                .max_h(move || px((window().height.0 - 96.0).max(160.0)))
                .rounded(px(10.0))
                .border_1()
                .border_color(border_strong())
                .bg(bg_card())
                .shadow_md()
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
        .h(px(26.0))
        .px_2()
        .gap_1()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .rounded(px(6.0))
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
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.75))
        .child(
            text(tr("更多", "More"))
                .number_of_lines(1)
                .text_size(px(12.0))
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
        .disabled_style(|style| style.opacity(0.45).cursor(Cursor::Default))
        .on_select(on_select)
        .h(px(30.0))
        .px_2p5()
        .rounded(px(6.0))
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
        .active(|style| style.opacity(move || if active_check() { 0.75_f32 } else { 1.0_f32 }))
        .cursor(Cursor::Pointer)
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
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.opacity(0.75))
        .cursor(Cursor::Pointer)
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
        .h(px(26.0))
        .px_2()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(border_control())
        .bg(bg_control())
        .hover(|style| style.bg(bg_control_hover()))
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.75))
        .on_press(on_press)
        .child(
            text(label)
                .number_of_lines(1)
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg_secondary()),
        )
}

pub(crate) fn section_title(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg_secondary())
}

pub(crate) fn muted_line(label: impl IntoReactive<SharedString>) -> Text {
    text(label).text_size(px(12.0)).text_color(fg_muted())
}

pub(crate) fn cert_display_name(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|seg| !seg.is_empty())
        .unwrap_or(path)
}
