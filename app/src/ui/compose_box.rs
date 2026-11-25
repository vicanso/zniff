#[allow(unused_imports)]
use super::*;

pub(crate) fn submit_compose(
    controls: &Arc<Controls>,
    compose: Compose,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
) {
    if compose.response.get() {
        let raw_status = compose.status.get();
        if !raw_status.trim().is_empty() && parse_status_code(&raw_status).is_none() {
            return;
        }
        continue_break(controls, compose, selected, paused);
        return;
    }
    let at_req_break = paused
        .get()
        .is_some_and(|(id, side)| selected.get() == Some(id) && side == BreakSide::Request)
        || controls
            .hub
            .paused()
            .is_some_and(|(id, side)| selected.get() == Some(id) && side == BreakSide::Request);
    if at_req_break {
        continue_break(controls, compose, selected, paused);
        return;
    }
    let method_now = compose.method.get();
    let url_now = compose.url.get();
    let headers_now = compose.headers.get();
    let body_now = compose.body.get();
    if selected.get().is_none() && url_now.trim().is_empty() {
        set_status(controls, StatusMsg::NeedUrl);
        return;
    }
    if !url_now.trim().is_empty() {
        let entry = (
            if method_now.trim().is_empty() {
                "GET".to_string()
            } else {
                method_now.clone()
            },
            url_now.clone(),
            headers_now.clone(),
            body_now.clone(),
        );
        compose.recent.update(|list| {
            list.retain(|item| item != &entry);
            list.insert(0, entry);
            list.truncate(config::COMPOSE_HISTORY_CAP);
        });
        let persisted = compose.recent.with_untracked(|list| {
            list.iter()
                .map(|(m, u, h, b)| config::ComposeHistoryItem {
                    method: m.clone(),
                    url: u.clone(),
                    headers: h.clone(),
                    body: b.clone(),
                })
                .collect()
        });
        let _ = save_config(|config| {
            config.compose_history = config::sanitize_compose_history(persisted);
        });
        if selected.get().is_none() {
            compose.draft_saved.set((
                "GET".to_string(),
                String::new(),
                String::new(),
                String::new(),
            ));
        }
    }
    replay_flow(
        controls,
        selected.get(),
        ReplayOverride {
            method: Some(method_now),
            url: Some(url_now),
            headers: Some(parse_header_text(&headers_now)),
            body: Some(body_now.into_bytes()),
        },
    );
}

pub(crate) fn compose_box(
    compose: Compose,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
) -> impl IntoElement {
    let controls_touch_status = controls.clone();
    let controls_touch_method = controls.clone();
    let controls_touch_url = controls.clone();
    let controls_submit_url = controls.clone();
    let controls_touch_headers = controls.clone();
    let controls_touch_body = controls.clone();
    let controls_submit_status = controls.clone();
    let controls_recent = controls.clone();
    let status_invalid = move || {
        compose.response.get()
            && !compose.status.get().trim().is_empty()
            && parse_status_code(&compose.status.get()).is_none()
    };
    let is_req_break = move || {
        !compose.response.get()
            && paused
                .get()
                .is_some_and(|(id, side)| selected.get() == Some(id) && side == BreakSide::Request)
    };
    view()
        .rounded(px(RADIUS_LG))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .overflow_hidden()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .px_3p5()
                .py_2()
                .border_b_1()
                .border_b_color(border_subtle())
                .child(section_title({
                    let langs = ui_langs();
                    move || -> SharedString {
                        if compose.response.get() {
                            t(ui_lang(langs), "编辑响应", "Edit response").into()
                        } else if is_req_break() {
                            t(ui_lang(langs), "编辑请求", "Edit request").into()
                        } else if selected.get().is_none() {
                            t(ui_lang(langs), "构造请求", "New request").into()
                        } else {
                            t(ui_lang(langs), "编辑重发", "Edit and resend").into()
                        }
                    }
                }))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .child(when(
                            move || {
                                !compose.response.get()
                                    && !is_req_break()
                                    && !compose.recent.with(Vec::is_empty)
                            },
                            move || compose_recent_menu(compose, controls_recent.clone()),
                        ))
                        .child(when(
                            move || {
                                !compose.response.get()
                                    && !is_req_break()
                                    && (!compose.url.get().is_empty()
                                        || !compose.headers.get().is_empty()
                                        || !compose.body.get().is_empty())
                            },
                            move || compose_clear_button(compose),
                        ))
                        .child(
                            button()
                                .h(px(26.0))
                                .px_2p5()
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
                                .flex_row()
                                .items_center()
                                .on_press(move |_: &()| reset_compose(compose))
                                .child(
                                    text(tr("取消", "Cancel"))
                                        .number_of_lines(1)
                                        .text_size(px(TEXT_SM))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(fg_secondary()),
                                ),
                        )
                        .child({
                            let controls = controls.clone();
                            let langs = ui_langs();
                            let is_continue =
                                move || compose.response.get() || is_req_break();
                            view()
                                .tooltip(tooltip().child(ui_tooltip_surface(
                                    text(move || -> SharedString {
                                        if is_continue() {
                                            t(
                                                ui_lang(langs),
                                                "Cmd/Ctrl+Enter 放行",
                                                "Cmd/Ctrl+Enter to continue",
                                            )
                                        } else {
                                            t(
                                                ui_lang(langs),
                                                "Cmd/Ctrl+Enter 发送",
                                                "Cmd/Ctrl+Enter to send",
                                            )
                                        }
                                        .into()
                                    })
                                    .text_size(px(TEXT_SM))
                                    .text_color(fg_primary()),
                                )))
                                .child(
                                    button()
                                        .h(px(26.0))
                                        .px_2p5()
                                        .rounded(px(RADIUS_MD))
                                        .bg(move || {
                                            if status_invalid() {
                                                bg_control()
                                            } else {
                                                accent_blue()
                                            }
                                        })
                                        .hover(|style| {
                                            style.bg(move || {
                                                if status_invalid() {
                                                    bg_control()
                                                } else {
                                                    accent_blue_hover()
                                                }
                                            })
                                        })
                                        .active(|style| {
                                            style.bg(move || {
                                                if status_invalid() {
                                                    bg_control()
                                                } else {
                                                    accent_blue_active()
                                                }
                                            })
                                        })
                                        .transition_colors()
                                        .duration(MOTION_INSTANT_MS)
                                        .ease_out()
                                        .focus_visible(apply_focus_ring)
                                        .disabled(status_invalid)
                                        .disabled_style(|style| style.opacity(0.55))
                                        .flex_row()
                                        .items_center()
                                        .on_press(move |_: &()| {
                                            submit_compose(&controls, compose, selected, paused);
                                        })
                                        .child(
                                            text(move || -> SharedString {
                                                if is_continue() {
                                                    t(ui_lang(langs), "放行", "Continue")
                                                } else {
                                                    t(ui_lang(langs), "发送", "Send")
                                                }
                                                .into()
                                            })
                                            .number_of_lines(1)
                                            .text_size(px(TEXT_SM))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || {
                                                if status_invalid() {
                                                    fg_muted()
                                                } else {
                                                    fg_on_accent()
                                                }
                                            }),
                                        ),
                                )
                        }),
                ),
        )
        .child(
            view()
                .p_3p5()
                .gap_2()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .child(when(
                            move || compose.response.get(),
                            move || {
                                let controls_touch = controls_touch_status.clone();
                                let controls_submit = controls_submit_status.clone();
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        text(tr("状态码", "Status code"))
                                            .number_of_lines(1)
                                            .text_size(px(TEXT_SM))
                                            .text_color(fg_secondary()),
                                    )
                                    .child(
                                        view().w(px(88.0)).flex_shrink_0().child(
                                            compose_input_on(
                                                compose.status,
                                                "200",
                                                30.0,
                                                false,
                                                move |text| {
                                                    controls_touch.hub.touch_break();
                                                    compose.status.set(text.to_string());
                                                },
                                            )
                                            .border_color(move || {
                                                if status_invalid() {
                                                    status_err_fg()
                                                } else {
                                                    border_control()
                                                }
                                            })
                                            .focus(move |style| {
                                                style.border_color(move || {
                                                    if status_invalid() {
                                                        status_err_fg()
                                                    } else {
                                                        accent_blue()
                                                    }
                                                })
                                            })
                                            .on_submit(move |text: &str| {
                                                compose.status.set(text.to_string());
                                                if status_invalid() {
                                                    return;
                                                }
                                                continue_break(
                                                    &controls_submit,
                                                    compose,
                                                    selected,
                                                    paused,
                                                );
                                            }),
                                        ),
                                    )
                            },
                        ))
                        .child(when(
                            move || !compose.response.get(),
                            move || {
                                let controls_method = controls_touch_method.clone();
                                let controls_method_pick = controls_touch_method.clone();
                                let controls_url = controls_touch_url.clone();
                                let controls_url_submit = controls_submit_url.clone();
                                when(
                                    is_req_break,
                                    move || {
                                        view()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .h(px(30.0))
                                            .flex_row()
                                            .items_center()
                                            .gap_2()
                                            .px_2p5()
                                            .rounded(px(RADIUS_MD))
                                            .border_1()
                                            .border_color(border_subtle())
                                            .bg(bg_code())
                                            .child(
                                                view()
                                                    .px_2()
                                                    .py_0p5()
                                                    .flex_shrink_0()
                                                    .rounded(px(RADIUS_SM))
                                                    .bg(move || method_bg(&compose.method.get()))
                                                    .child(
                                                        mono(move || compose.method.get())
                                                            .text_size(px(TEXT_XS))
                                                            .font_weight(FontWeight::BOLD)
                                                            .text_color(move || {
                                                                method_fg(&compose.method.get())
                                                            }),
                                                    ),
                                            )
                                            .child(
                                                mono(move || compose.url.get())
                                                    .flex_1()
                                                    .min_w(px(0.0))
                                                    .number_of_lines(1)
                                                    .text_overflow(EllipsizeMode::Middle)
                                                    .text_size(px(TEXT_SM))
                                                    .text_color(fg_secondary()),
                                            )
                                    },
                                )
                                .otherwise(move || {
                                    let controls_method = controls_method.clone();
                                    let controls_method_pick = controls_method_pick.clone();
                                    let controls_url = controls_url.clone();
                                    let controls_url_submit = controls_url_submit.clone();
                                    let method_open = signal(false);
                                    bind_escape_while(
                                        move || method_open.get(),
                                        nsh_core::keymap::When::Always,
                                        move || method_open.set(false),
                                    );
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .flex_row()
                                        .gap_1p5()
                                        .child(
                                            view()
                                                .flex_row()
                                                .items_center()
                                                .gap_1()
                                                .flex_shrink_0()
                                                .child(view().w(px(76.0)).child(compose_input_on(
                                                    compose.method,
                                                    "POST",
                                                    30.0,
                                                    false,
                                                    move |text| {
                                                        controls_method.hub.touch_break();
                                                        compose.method.set(text.to_string());
                                                    },
                                                )))
                                                .child(
                                                    menu()
                                                        .open(move || Some(method_open.get()))
                                                        .on_open_change(
                                                            move |event: &nsh_base::MenuEvent| {
                                                                if let nsh_base::MenuEvent::OpenChange(
                                                                    next,
                                                                    _,
                                                                ) = event
                                                                {
                                                                    method_open.set(*next);
                                                                }
                                                            },
                                                        )
                                                        .placement(Placement::Bottom)
                                                        .align(Align::Start)
                                                        .offset(px(4.0))
                                                        .margin(px(8.0))
                                                        .popup_style(ui_popup_style)
                                                        .popup_starting_style(
                                                            ui_popup_starting_style,
                                                        )
                                                        .popup_ending_style(
                                                            ui_popup_ending_style,
                                                        )
                                                        .trigger(
                                                            view()
                                                                .role(AccessibilityRole::Button)
                                                                .w(px(26.0))
                                                                .h(px(30.0))
                                                                .rounded(px(RADIUS_MD))
                                                                .border_1()
                                                                .border_color(move || {
                                                                    if method_open.get() {
                                                                        accent_blue_border()
                                                                    } else {
                                                                        border_control()
                                                                    }
                                                                })
                                                                .bg(move || {
                                                                    if method_open.get() {
                                                                        accent_blue_soft()
                                                                    } else {
                                                                        bg_control()
                                                                    }
                                                                })
                                                                .hover(|style| {
                                                                    style.bg(move || {
                                                                        if method_open.get() {
                                                                            accent_blue_soft_hover()
                                                                        } else {
                                                                            bg_control_hover()
                                                                        }
                                                                    })
                                                                })
                                                                .active(|style| {
                                                                    style.bg(bg_control_active())
                                                                })
                                                                .transition_colors()
                                                                .duration(MOTION_INSTANT_MS)
                                                                .ease_out()
                                                                .focus_visible(apply_focus_ring)
                                                                .items_center()
                                                                .justify_center()
                                                                .label(tr(
                                                                    "选择 HTTP 方法",
                                                                    "Pick HTTP method",
                                                                ))
                                                                .tooltip(label_tip(
                                                                    "选择常用 HTTP 方法",
                                                                    "Pick HTTP method",
                                                                ))
                                                                .child(glyph(
                                                                    || Glyph::Chevron,
                                                                    move || {
                                                                        color(if method_open.get() {
                                                                            accent_blue_text()
                                                                        } else {
                                                                            fg_secondary()
                                                                        })
                                                                    },
                                                                )),
                                                        )
                                                        .child(
                                                            view()
                                                                .w(px(116.0))
                                                                .max_w(move || {
                                                                    px((window().width.0 - 16.0)
                                                                        .max(96.0))
                                                                })
                                                                .max_h(move || {
                                                                    px((window().height.0 - 24.0)
                                                                        .max(120.0))
                                                                })
                                                                .overflow_hidden()
                                                                .child(
                                                                    scroll()
                                                                        .w_full()
                                                                        .max_h(move || {
                                                                            px((window().height.0
                                                                                - 26.0)
                                                                                .max(118.0))
                                                                        })
                                                                        .child(
                                                                            view()
                                                                                .w_full()
                                                                                .flex_col()
                                                                                .p_1()
                                                                                .gap_0p5()
                                                                                .children(
                                                                                    [
                                                                                        "GET",
                                                                                        "POST",
                                                                                        "PUT",
                                                                                        "PATCH",
                                                                                        "DELETE",
                                                                                        "HEAD",
                                                                                        "OPTIONS",
                                                                                    ]
                                                                                    .into_iter()
                                                                                    .map(move |m| {
                                                                                        let controls_pick =
                                                                                            controls_method_pick
                                                                                                .clone();
                                                                                        marked_action(
                                                                                            move || {
                                                                                                compose
                                                                                                    .method
                                                                                                    .get()
                                                                                                    .trim()
                                                                                                    .eq_ignore_ascii_case(
                                                                                                        m,
                                                                                                    )
                                                                                            },
                                                                                            m,
                                                                                            move || {
                                                                                                controls_pick
                                                                                                    .hub
                                                                                                    .touch_break();
                                                                                                compose
                                                                                                    .method
                                                                                                    .set(m.to_string());
                                                                                            },
                                                                                        )
                                                                                    }),
                                                                                ),
                                                                        ),
                                                                ),
                                                        ),
                                                ),
                                        )
                                        .child(view().flex_1().min_w(px(0.0)).child(
                                            compose_input_on(
                                                compose.url,
                                                tr(
                                                    "https://example.com/path，或粘贴 curl",
                                                    "https://example.com/path, or paste curl",
                                                ),
                                                30.0,
                                                false,
                                                move |text| {
                                                    controls_url.hub.touch_break();
                                                    apply_url_edit(compose, text);
                                                },
                                            )
                                            .on_submit(move |text: &str| {
                                                controls_url_submit.hub.touch_break();
                                                if looks_like_curl(text) {
                                                    apply_url_edit(compose, text);
                                                    return;
                                                }
                                                apply_url_edit(compose, text);
                                                submit_compose(
                                                    &controls_url_submit,
                                                    compose,
                                                    selected,
                                                    paused,
                                                );
                                            }),
                                        ))
                                })
                            },
                        )),
                )
                .child(
                    view()
                        .w_full()
                        .gap_1()
                        .flex_col()
                        .child(
                            text({
                                let langs = ui_langs();
                                move || -> SharedString {
                                    if compose.response.get() {
                                        t(
                                            ui_lang(langs),
                                            "响应头 (Name: Value)",
                                            "Response headers (Name: Value)",
                                        )
                                        .into()
                                    } else {
                                        t(
                                            ui_lang(langs),
                                            "请求头 (Name: Value)",
                                            "Request headers (Name: Value)",
                                        )
                                        .into()
                                    }
                                }
                            })
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg_secondary()),
                        )
                        .child(compose_editor(
                            compose.headers,
                            tr("一行一个 Name: value", "One Name: value per line"),
                            120.0,
                            "plaintext",
                            controls_touch_headers,
                        )),
                )
                .child(
                    view()
                        .w_full()
                        .gap_1()
                        .flex_col()
                        .child(
                            text({
                                let langs = ui_langs();
                                move || -> SharedString {
                                    if compose.response.get() {
                                        t(ui_lang(langs), "响应正文", "Response body").into()
                                    } else {
                                        t(ui_lang(langs), "请求正文", "Request body").into()
                                    }
                                }
                            })
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg_secondary()),
                        )
                        .child(compose_body_editor(
                            compose.body,
                            compose.headers,
                            tr("body", "body"),
                            160.0,
                            controls_touch_body,
                        )),
                ),
        )
}

pub(crate) fn compose_body_editor(
    value: Signal<String>,
    headers: Signal<String>,
    placeholder: impl Fn() -> String + 'static,
    height: f32,
    controls: Arc<Controls>,
) -> impl IntoElement {
    guard_culled_pointer_events(view())
        .w_full()
        .h(px(height))
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border_control())
        .bg(bg_code())
        .overflow_hidden()
        .child(
            editor()
                .value(move || SharedString::from(value.get()))
                .language(move || {
                    let parsed = parse_header_text(&headers.get());
                    let ct = present::content_type(&parsed);
                    SharedString::from(
                        present::editor_language(ct, &value.get()).unwrap_or("plaintext"),
                    )
                })
                .placeholder(move || SharedString::from(placeholder()))
                .line_numbers(true)
                .folding(true)
                .soft_wrap(true)
                .font_size(move || scaled_px(12.0))
                .line_height(move || scaled_px(18.0))
                .scroll_beyond_last_line(false)
                .on_change(move |text: &SharedString| {
                    controls.hub.touch_break();
                    value.set(text.to_string());
                })
                .size_full(),
        )
}

pub(crate) fn compose_editor(
    value: Signal<String>,
    placeholder: impl Fn() -> String + 'static,
    height: f32,
    language: &'static str,
    controls: Arc<Controls>,
) -> impl IntoElement {
    guard_culled_pointer_events(view())
        .w_full()
        .h(px(height))
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border_control())
        .bg(bg_code())
        .overflow_hidden()
        .child(
            editor()
                .value(move || SharedString::from(value.get()))
                .language(language)
                .placeholder(move || SharedString::from(placeholder()))
                .line_numbers(true)
                .folding(true)
                .soft_wrap(true)
                .font_size(move || scaled_px(12.0))
                .line_height(move || scaled_px(18.0))
                .scroll_beyond_last_line(false)
                .on_change(move |text: &SharedString| {
                    controls.hub.touch_break();
                    value.set(text.to_string());
                })
                .size_full(),
        )
}

pub(crate) fn compose_input_on(
    value: Signal<String>,
    placeholder: impl IntoReactive<String> + 'static,
    height: f32,
    multiline: bool,
    on_change: impl Fn(&str) + 'static,
) -> Input {
    input()
        .value(value)
        .placeholder(placeholder)
        .placeholder_color(move || color(fg_muted()))
        .w_full()
        .h(px(height))
        .px_2p5()
        .rounded(px(RADIUS_MD))
        .bg(bg_code())
        .border_1()
        .border_color(border_control())
        .focus(|style| style.border_color(accent_blue()))
        .text_size(px(TEXT_SM))
        .text_color(fg_primary())
        .multiline(multiline)
        .on_change(on_change)
}

pub(crate) fn lan_toggle(on: Signal<bool>, controls: Arc<Controls>) -> impl IntoElement {
    let langs = ui_langs();
    switch()
        .h(px(24.0))
        .px_1p5()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_1p5()
        .rounded(px(RADIUS_MD))
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(move || motion_ms(MOTION_INSTANT_MS))
        .ease_bezier(0.22, 1.0, 0.36, 1.0)
        .focus_visible(apply_focus_ring)
        .checked(move || on.get())
        .on_change(move |next: &bool| {
            let next = *next;
            on.set(next);
            set_bool(&controls.listen_all, next);
            if let Err(err) = store_listen_all(next) {
                set_status(
                    &controls,
                    StatusMsg::PrefUnsaved {
                        name: PrefName::Lan,
                        err: err.to_string(),
                    },
                );
            }
        })
        .track_style(|s| {
            s.w(px(26.0))
                .h(px(14.0))
                .p(px(2.0))
                .flex_row()
                .items_center()
                .rounded_full()
                .bg(switch_track_off())
                .checked(|c| c.bg(accent_blue()))
                .transition_colors()
                .duration(move || motion_ms(MOTION_INSTANT_MS))
                .ease_bezier(0.22, 1.0, 0.36, 1.0)
        })
        .thumb_style(|s| {
            s.w(px(10.0))
                .h(px(10.0))
                .rounded_full()
                .bg(switch_thumb())
                .checked(|c| c.translate_x(px(12.0)))
                .transition_transform()
                .duration(move || motion_ms(MOTION_INSTANT_MS))
                .ease_bezier(0.22, 1.0, 0.36, 1.0)
        })
        .child(
            text(move || -> SharedString {
                if on.get() {
                    t(ui_lang(langs), "局域网 开", "LAN on").into()
                } else {
                    t(ui_lang(langs), "局域网 关", "LAN off").into()
                }
            })
            .number_of_lines(1)
            .text_size(px(TEXT_XS))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || if on.get() { fg_primary() } else { fg_muted() }),
        )
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn prompt_import_har(controls: Arc<Controls>) {
    spawn_detached(async move {
        let dialog =
            open_files_dialog()
                .multiple(true)
                .filter(nsh_core::file_dialog::FileFilter::new(
                    "HAR / Charles (.chlsj)",
                    ["har", "chlsj"],
                ));
        match pick_paths_after_paint(dialog).await {
            DialogOutcome::Picked(paths) => {
                for path in paths {
                    let text = path.to_string_lossy();
                    if path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("chlsj"))
                    {
                        import_chlsj_file(&controls, &text);
                    } else {
                        import_har_file(&controls, &text);
                    }
                }
            }
            DialogOutcome::Cancelled | DialogOutcome::Busy => {}
            DialogOutcome::Failed(err) => set_status(&controls, StatusMsg::ImportFailed(err)),
        }
    });
}

fn compose_clear_button(compose: Compose) -> impl IntoElement {
    let confirm = signal(false);
    let reset_job = stored(None::<Spawned<()>>);
    let langs = ui_langs();
    button()
        .h(px(26.0))
        .px_2p5()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(move || {
            if confirm.get() {
                status_err_fg()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if confirm.get() {
                status_err_bg()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if confirm.get() {
                    status_err_bg_hover()
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
        .flex_row()
        .items_center()
        .on_press(move |_: &()| {
            if confirm.get_untracked() {
                reset_job.replace(None);
                confirm.set(false);
                compose.method.set("GET".to_string());
                compose.url.set(String::new());
                compose.headers.set(String::new());
                compose.body.set(String::new());
                compose.draft_saved.set((
                    "GET".to_string(),
                    String::new(),
                    String::new(),
                    String::new(),
                ));
            } else {
                confirm.set(true);
                reset_job.replace(Some(set_timeout(Duration::from_millis(3500), move || {
                    confirm.set(false);
                })));
            }
        })
        .child(
            text(move || -> SharedString {
                if confirm.get() {
                    t(ui_lang(langs), "确认清空？", "Confirm clear?").into()
                } else {
                    t(ui_lang(langs), "清空", "Clear").into()
                }
            })
            .number_of_lines(1)
            .text_size(px(TEXT_SM))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || {
                if confirm.get() {
                    status_err_fg()
                } else {
                    fg_secondary()
                }
            }),
        )
}

fn compose_recent_menu(compose: Compose, controls: Arc<Controls>) -> impl IntoElement {
    let recent_open = signal(false);
    bind_escape_while(
        move || recent_open.get(),
        nsh_core::keymap::When::Always,
        move || recent_open.set(false),
    );
    let confirm_clear = signal(false);
    popover()
        .open(move || Some(recent_open.get()))
        .on_open_change(move |event: &nsh_base::PopoverEvent| {
            if let nsh_base::PopoverEvent::OpenChange(next, _) = event {
                recent_open.set(*next);
                if !*next {
                    confirm_clear.set(false);
                }
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
                .px_2()
                .rounded(px(RADIUS_MD))
                .border_1()
                .border_color(move || {
                    if recent_open.get() {
                        accent_blue_border()
                    } else {
                        border_control()
                    }
                })
                .bg(move || {
                    if recent_open.get() {
                        accent_blue_soft()
                    } else {
                        bg_control()
                    }
                })
                .hover(|style| {
                    style.bg(move || {
                        if recent_open.get() {
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
                .flex_row()
                .items_center()
                .gap_1()
                .child(glyph_px(
                    12.0,
                    || Glyph::History,
                    move || {
                        color(if recent_open.get() {
                            accent_blue_text()
                        } else {
                            fg_secondary()
                        })
                    },
                ))
                .child(
                    text(tr("最近", "Recent"))
                        .number_of_lines(1)
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if recent_open.get() {
                                accent_blue_text()
                            } else {
                                fg_secondary()
                            }
                        }),
                ),
        )
        .child(
            view()
                .w(px(360.0))
                .max_h(px(312.0))
                .p_1()
                .child(
                    scroll()
                        .w_full()
                        .max_h(px(268.0))
                        .child(view().w_full().gap_0p5().child(each(
                            compose.recent,
                            |(m, u, h, b)| format!("{m}\t{u}\t{h}\t{b}"),
                            move |item, _idx| {
                                let pick = item;
                                let read_m = item;
                                let read_u = item;
                                button()
                                    .w_full()
                                    .px_2()
                                    .py_1p5()
                                    .rounded(px(RADIUS_MD))
                                    .hover(|style| style.bg(bg_control_hover()))
                                    .active(|style| style.bg(bg_control_active()))
                                    .transition_colors()
                                    .duration(MOTION_INSTANT_MS)
                                    .ease_out()
                                    .focus_visible(apply_focus_ring)
                                    .flex_row()
                                    .items_center()
                                    .gap_1p5()
                                    .on_press(move |_: &()| {
                                        let (m, u, h, b) = pick.get();
                                        compose.method.set(m);
                                        compose.url.set(u);
                                        compose.headers.set(h);
                                        compose.body.set(b);
                                        confirm_clear.set(false);
                                        recent_open.set(false);
                                    })
                                    .child(
                                        mono(move || {
                                            SharedString::from(read_m.with(|it| it.0.clone()))
                                        })
                                        .number_of_lines(1)
                                        .flex_shrink_0()
                                        .text_size(px(TEXT_XS))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || read_m.with(|it| method_fg(&it.0))),
                                    )
                                    .child(
                                        mono(move || {
                                            SharedString::from(read_u.with(|it| it.1.clone()))
                                        })
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .number_of_lines(1)
                                        .text_overflow(EllipsizeMode::Middle)
                                        .text_size(px(TEXT_SM))
                                        .text_color(fg_primary()),
                                    )
                            },
                        ))),
                )
                .child(when(
                    move || !compose.recent.with(Vec::is_empty),
                    move || {
                        let controls = controls.clone();
                        view()
                            .w_full()
                            .px_2()
                            .py_1()
                            .border_t_1()
                            .border_color(border_subtle())
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(
                                text(move || -> SharedString {
                                    let count = compose.recent.with(Vec::len);
                                    let lang = ui_lang(ui_langs());
                                    match lang {
                                        Lang::Zh => format!("最近 {count} 条"),
                                        Lang::En => format!("{count} recent"),
                                    }
                                    .into()
                                })
                                .text_size(px(TEXT_XS))
                                .text_color(fg_muted()),
                            )
                            .child(
                                button()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(px(RADIUS_SM))
                                    .bg(move || {
                                        if confirm_clear.get() {
                                            danger_button_bg()
                                        } else {
                                            AdaptiveColor::TRANSPARENT
                                        }
                                    })
                                    .hover(|s| {
                                        s.bg(move || {
                                            if confirm_clear.get() {
                                                danger_button_hover()
                                            } else {
                                                bg_control_hover()
                                            }
                                        })
                                    })
                                    .active(|s| {
                                        s.bg(move || {
                                            if confirm_clear.get() {
                                                danger_button_active()
                                            } else {
                                                bg_control_active()
                                            }
                                        })
                                    })
                                    .transition_colors()
                                    .duration(MOTION_INSTANT_MS)
                                    .ease_out()
                                    .focus_visible(apply_focus_ring)
                                    .accessibility_label(move || -> Option<SharedString> {
                                        let lang = ui_lang(ui_langs());
                                        Some(
                                            if confirm_clear.get() {
                                                t(
                                                    lang,
                                                    "再次点击确认清空构造历史",
                                                    "Click again to clear compose history",
                                                )
                                            } else {
                                                t(lang, "清空历史", "Clear history")
                                            }
                                            .into(),
                                        )
                                    })
                                    .on_press(move |_: &()| {
                                        if !confirm_clear.get_untracked() {
                                            confirm_clear.set(true);
                                            return;
                                        }
                                        confirm_clear.set(false);
                                        compose.recent.set(Vec::new());
                                        let _ = save_config(|config| {
                                            config.compose_history.clear();
                                        });
                                        recent_open.set(false);
                                        set_status(&controls, StatusMsg::ComposeHistoryCleared);
                                    })
                                    .child(
                                        text(move || -> SharedString {
                                            let lang = ui_lang(ui_langs());
                                            if confirm_clear.get() {
                                                t(lang, "确认清空？", "Confirm clear?").into()
                                            } else {
                                                t(lang, "清空历史", "Clear history").into()
                                            }
                                        })
                                        .text_size(px(TEXT_XS))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(
                                            move || {
                                                if confirm_clear.get() {
                                                    fg_on_accent()
                                                } else {
                                                    status_warn_fg()
                                                }
                                            },
                                        ),
                                    ),
                            )
                    },
                )),
        )
}
