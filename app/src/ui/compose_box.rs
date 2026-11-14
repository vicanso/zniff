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
    replay_flow(
        controls,
        selected.get(),
        ReplayOverride {
            method: Some(compose.method.get()),
            url: Some(compose.url.get()),
            headers: Some(parse_header_text(&compose.headers.get())),
            body: Some(compose.body.get().into_bytes()),
        },
    );
}

pub(crate) fn compose_box(
    compose: Compose,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
) -> impl IntoElement {
    let controls_continue = controls.clone();
    let controls_touch_status = controls.clone();
    let controls_touch_method = controls.clone();
    let controls_touch_url = controls.clone();
    let controls_submit_url = controls.clone();
    let controls_touch_headers = controls.clone();
    let controls_touch_body = controls.clone();
    let controls_submit_status = controls.clone();
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
        .rounded(px(10.0))
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
                        .child(
                            view()
                                .h(px(26.0))
                                .px_2p5()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(border_control())
                                .bg(bg_control())
                                .hover(|style| style.bg(bg_control_hover()))
                                .active(|style| style.opacity(0.75))
                                .cursor(Cursor::Pointer)
                                .flex_row()
                                .items_center()
                                .on_press(move || reset_compose(compose))
                                .child(
                                    text(tr("取消", "Cancel"))
                                        .number_of_lines(1)
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(fg_secondary()),
                                ),
                        )
                        .child(when(
                            move || compose.response.get(),
                            move || {
                                let controls = controls_continue.clone();
                                view()
                                    .h(px(26.0))
                                    .px_2p5()
                                    .rounded(px(6.0))
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
                                        style.opacity(move || {
                                            if status_invalid() { 1.0_f32 } else { 0.85_f32 }
                                        })
                                    })
                                    .cursor(Cursor::Pointer)
                                    .disabled(status_invalid)
                                    .disabled_style(|style| style.cursor(Cursor::Default))
                                    .flex_row()
                                    .items_center()
                                    .tooltip(label_tip("Cmd/Ctrl+Enter 放行", "Cmd/Ctrl+Enter to continue"))
                                    .on_press(move || {
                                        submit_compose(&controls, compose, selected, paused);
                                    })
                                    .child(
                                        text(tr("放行", "Continue"))
                                            .number_of_lines(1)
                                            .text_size(px(12.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || {
                                                if status_invalid() {
                                                    fg_muted()
                                                } else {
                                                    AdaptiveColor::rgb(0xffffff, 0xffffff)
                                                }
                                            }),
                                    )
                            },
                        ))
                        .child(when(
                            move || !compose.response.get(),
                            move || {
                                let controls = controls.clone();
                                let langs = ui_langs();
                                view()
                                    .h(px(26.0))
                                    .px_2p5()
                                    .rounded(px(6.0))
                                    .bg(accent_blue())
                                    .hover(|style| style.bg(accent_blue_hover()))
                                    .active(|style| style.opacity(0.85))
                                    .cursor(Cursor::Pointer)
                                    .flex_row()
                                    .items_center()
                                    .tooltip(label_tip("Cmd/Ctrl+Enter 发送", "Cmd/Ctrl+Enter to send"))
                                    .on_press(move || {
                                        submit_compose(&controls, compose, selected, paused);
                                    })
                                    .child(
                                        text(move || -> SharedString {
                                            if is_req_break() {
                                                t(ui_lang(langs), "放行", "Continue").into()
                                            } else {
                                                t(ui_lang(langs), "发送", "Send").into()
                                            }
                                        })
                                        .number_of_lines(1)
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(white()),
                                    )
                            },
                        )),
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
                                            .text_size(px(12.0))
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
                                            .rounded(px(7.0))
                                            .border_1()
                                            .border_color(border_subtle())
                                            .bg(bg_code())
                                            .child(
                                                view()
                                                    .px_2()
                                                    .py_0p5()
                                                    .flex_shrink_0()
                                                    .rounded(px(5.0))
                                                    .bg(move || method_bg(&compose.method.get()))
                                                    .child(
                                                        mono(move || compose.method.get())
                                                            .text_size(px(11.5))
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
                                                    .text_size(px(12.0))
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
                                                        .trigger(
                                                            view()
                                                                .w(px(26.0))
                                                                .h(px(30.0))
                                                                .rounded(px(7.0))
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
                                                                .active(|style| style.opacity(0.8))
                                                                .cursor(Cursor::Pointer)
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
                                                                .rounded(px(9.0))
                                                                .border_1()
                                                                .border_color(border_strong())
                                                                .bg(bg_card())
                                                                .shadow_md()
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
                .child(compose_editor(
                    compose.headers,
                    tr("一行一个 Name: value", "One Name: value per line"),
                    120.0,
                    "plaintext",
                    controls_touch_headers,
                ))
                .child(compose_body_editor(
                    compose.body,
                    compose.headers,
                    tr("body", "body"),
                    160.0,
                    controls_touch_body,
                )),
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
        .rounded(px(7.0))
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
                .font_size(Points(12.0))
                .line_height(Points(18.0))
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
        .rounded(px(7.0))
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
                .font_size(Points(12.0))
                .line_height(Points(18.0))
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
        .rounded(px(7.0))
        .bg(bg_code())
        .border_1()
        .border_color(border_control())
        .focus(|style| style.border_color(accent_blue()))
        .text_size(px(12.0))
        .text_color(fg_primary())
        .multiline(multiline)
        .on_change(on_change)
}

pub(crate) fn apply_url_edit(compose: Compose, text: &str) {
    if looks_like_curl(text)
        && let Some(parsed) = present::parse_curl(text)
    {
        compose.method.set(parsed.method);
        compose.url.set(parsed.url);
        compose.headers.set(header_text(&parsed.headers));
        compose.body.set(parsed.body);
        return;
    }
    compose.url.set(text.to_string());
}

pub(crate) fn looks_like_curl(text: &str) -> bool {
    let text = text.trim_start();
    text == "curl"
        || text.starts_with("curl ")
        || text.starts_with("curl\t")
        || text.starts_with("curl\n")
        || text.starts_with("curl\r")
}

pub(crate) fn parse_status_code(text: &str) -> Option<u16> {
    let code = text.split_whitespace().next()?.parse::<u16>().ok()?;
    (100..600).contains(&code).then_some(code)
}

pub(crate) fn edited_status(compose: Compose) -> Option<u16> {
    if !compose.response.get() {
        return None;
    }
    parse_status_code(&compose.status.get())
}

pub(crate) fn lan_toggle(on: Signal<bool>, controls: Arc<Controls>) -> impl IntoElement {
    view()
        .h(px(22.0))
        .px_2()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_1()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || {
            if on.get() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(bg_control())
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .on_press(move || {
            let next = !on.get();
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
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .bg(move || if on.get() { status_ok_fg() } else { fg_muted() }),
        )
        .child(
            text({
                let langs = ui_langs();
                move || -> SharedString {
                    if on.get() {
                        t(ui_lang(langs), "局域网 开", "LAN on").into()
                    } else {
                        t(ui_lang(langs), "局域网 关", "LAN off").into()
                    }
                }
            })
            .text_size(px(11.5))
            .text_color(fg_secondary()),
        )
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn prompt_import_har(controls: Arc<Controls>) {
    spawn_detached(async move {
        let dialog = open_dialog_with_default_dir()
            .multiple(true)
            .filter(nsh_core::file_dialog::FileFilter::new("HAR", ["har"]));
        match pick_paths(dialog).await {
            Ok(Some(paths)) => {
                for path in paths {
                    import_har_file(&controls, &path.to_string_lossy());
                }
            }
            Ok(None) => {}
            Err(err) => set_status(&controls, StatusMsg::ImportFailed(err)),
        }
    });
}

pub(crate) fn begin_new_request(compose: Compose, selected: Signal<Option<u64>>) {
    selected.set(None);
    compose.response.set(false);
    compose.method.set("GET".to_string());
    compose.url.set(String::new());
    compose.headers.set(String::new());
    compose.body.set(String::new());
    compose.status.set(String::new());
    compose.composing.set(true);
}

pub(crate) fn begin_compose(controls: &Controls, id: Option<u64>, compose: Compose) {
    if refuse_relay(controls, id) {
        return;
    }
    if id.is_none() {
        compose.response.set(false);
        compose.method.set("GET".to_string());
        compose.url.set(String::new());
        compose.headers.set(String::new());
        compose.body.set(String::new());
        compose.status.set(String::new());
        compose.composing.set(true);
        return;
    }
    let Some(flow) = selected_flow(controls, id) else {
        set_status(controls, StatusMsg::NeedSelection);
        return;
    };
    controls.hub.touch_break();
    let response = controls
        .hub
        .paused()
        .is_some_and(|(paused_id, side)| id == Some(paused_id) && side == BreakSide::Response);
    compose.response.set(response);
    compose.method.set(flow.method);
    compose.url.set(flow.url);
    if response {
        compose
            .status
            .set(flow.status.map(|code| code.to_string()).unwrap_or_default());
        compose.headers.set(header_text(&flow.response_headers));
        compose
            .body
            .set(String::from_utf8_lossy(&flow.response_body).into_owned());
    } else {
        compose.status.set(String::new());
        compose.headers.set(header_text(&flow.request_headers));
        compose
            .body
            .set(String::from_utf8_lossy(&flow.request_body).into_owned());
    }
    compose.composing.set(true);
}

pub(crate) fn header_text(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn reset_compose(compose: Compose) {
    compose.composing.set(false);
    compose.response.set(false);
    compose.method.set(String::new());
    compose.url.set(String::new());
    compose.headers.set(String::new());
    compose.body.set(String::new());
    compose.status.set(String::new());
}

pub(crate) fn clear_shown(shown: Shown) {
    shown.phase.set(Phase::Empty);
    shown.method.set(String::new());
    shown.url.set(String::new());
    shown.protocol.set(String::new());
    shown.rules.set(String::new());
    shown.status.set(None);
    shown.error.set(String::new());
    shown.request_headers.set(Vec::new());
    shown.response_headers.set(Vec::new());
    shown.request_body.set(String::new());
    shown.response_body.set(String::new());
    shown.request_hex.set(Vec::new());
    shown.response_hex.set(Vec::new());
    shown.request_language.set(String::new());
    shown.response_language.set(String::new());
    shown.request_doc.set(None);
    shown.response_doc.set(None);
    shown.req_doc_preview.set(false);
    shown.res_doc_preview.set(false);
    shown.request_json.set(None);
    shown.response_json.set(None);
    shown.req_json_tree.set(false);
    shown.res_json_tree.set(false);
    shown.request_form.set(Vec::new());
    shown.request_note.set(String::new());
    shown.response_note.set(String::new());
    reset_body_offsets(shown);
    shown.request_window.set(TextWindow::default());
    shown.response_window.set(TextWindow::default());
    shown.metrics.set(String::new());
    shown.timing_bar.set(present::TimingBreakdown::default());
    shown.query.set(Vec::new());
    shown.preview.set(String::new());
    shown.preview_w.set(0.0);
    shown.preview_h.set(0.0);
    shown.image_caption.set(String::new());
    shown.messages.set(Vec::new());
    shown.diff.set(String::new());
    shown.diff_old.set(String::new());
    shown.diff_new.set(String::new());
    shown.pinned.set(false);
    shown.relay.set(String::new());
    shown.comment.set(String::new());
    shown.editing_comment.set(false);
    shown.repeating.set(false);
}

/// Only hides the panels. The panel's views and memos are still mounted until
/// the next frame, so clearing their data here would make them read signals
/// that are being torn down. [`release_sequence_data`] frees the data later.
pub(crate) fn close_sequence_diagram(seq_builder: SeqBuilder) {
    seq_builder.open.set(false);
    seq_builder.redact_open.set(false);
}

/// Frees the sequence-diagram data once both panels are closed. Runs from the
/// periodic tick, after the closed panels have been unmounted.
pub(crate) fn release_sequence_data(seq_builder: SeqBuilder) {
    if seq_builder.open.get_untracked() || seq_builder.redact_open.get_untracked() {
        return;
    }
    let has_data = seq_builder.items.with_untracked(|items| !items.is_empty())
        || seq_builder
            .diagram
            .with_untracked(|d| !d.steps.is_empty() || !d.bars.is_empty());
    if !has_data {
        return;
    }
    seq_builder.hover_bar.set(None);
    seq_builder.source_flowchart.set(false);
    seq_builder.items.set(Vec::new());
    seq_builder.output.set(String::new());
    seq_builder
        .diagram
        .set(present::SequenceDiagramData::default());
    seq_builder.redact_flow_id.set(None);
    crate::session::trim_memory();
}
