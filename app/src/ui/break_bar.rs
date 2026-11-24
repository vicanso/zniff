#[allow(unused_imports)]
use super::*;

pub(crate) fn config_broken_bar(
    config_broken: Signal<Option<String>>,
    config_broken_dismissed: Signal<bool>,
    profiles: ProfileSignals,
    port_text: Signal<String>,
    listen_all: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let copy_controls = controls.clone();
    let reset_controls = controls.clone();
    let msg_controls = controls;
    let langs = ui_langs();
    view()
        .min_h(px(40.0))
        .pl(px(LIST_INSET))
        .pr_3()
        .py_1p5()
        .bg(status_warn_bg())
        .border_b_1()
        .border_b_color(status_warn_fg())
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_2()
        .child(
            view()
                .flex_1()
                .min_w(px(220.0))
                .flex_row()
                .items_center()
                .gap_2()
                .child(glyph_px(
                    14.0,
                    || Glyph::Alert,
                    || color(status_warn_fg()),
                ))
                .child(
                    text(move || -> SharedString {
                        let err = config_broken.get().unwrap_or_default();
                        let cfg_path = lock_string(&msg_controls.config_path);
                        let file_name = if cfg_path.is_empty() {
                            "config.toml"
                        } else {
                            cert_display_name(&cfg_path)
                        };
                        match ui_lang(langs) {
                            Lang::Zh => format!(
                                "{file_name} 格式错误，当前更改无法保存（原文件已保留）：{err}"
                            )
                            .into(),
                            Lang::En => format!(
                                "{file_name} is invalid; changes cannot be saved (original file kept): {err}"
                            )
                            .into(),
                        }
                    })
                    .flex_1()
                    .min_w(px(0.0))
                    .selectable(true)
                    .number_of_lines(2)
                    .text_overflow(EllipsizeMode::Tail)
                    .text_size(px(TEXT_SM))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(status_warn_fg()),
                ),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(detail_button(tr("复制路径", "Copy path"), move || {
                    let path = lock_string(&copy_controls.config_path);
                    let target = if path.is_empty() {
                        "config.toml".to_string()
                    } else {
                        path
                    };
                    set_clipboard(target.clone());
                    set_status(&copy_controls, StatusMsg::CopiedConfigPath(target));
                }))
                .child(
                    view()
                        .role(AccessibilityRole::Button)
                        .h(px(CONTROL_H))
                        .px_2p5()
                        .rounded(px(RADIUS_MD))
                        .bg(accent_blue())
                        .hover(|style| style.bg(accent_blue_hover()))
                        .active(|style| style.bg(accent_blue_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .flex_row()
                        .items_center()
                        .on_press(move || {
                            reset_broken_config_action(
                                reset_controls.clone(),
                                config_broken,
                                config_broken_dismissed,
                                profiles,
                                port_text,
                                listen_all,
                                langs,
                            );
                        })
                        .child(
                            text(tr("备份并重置", "Backup & reset"))
                                .number_of_lines(1)
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(fg_on_accent()),
                        ),
                )
                .child(
                    view()
                        .role(AccessibilityRole::Button)
                        .w(px(CONTROL_H))
                        .h(px(CONTROL_H))
                        .rounded(px(RADIUS_MD))
                        .hover(|style| style.bg(bg_control_hover()))
                        .active(|style| style.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .label(tr("隐藏横幅", "Dismiss banner"))
                        .tooltip(label_tip(
                            "暂时收起（可从底部状态栏重新打开）",
                            "Dismiss (reopen from status bar)",
                        ))
                        .on_press(move || config_broken_dismissed.set(true))
                        .child(glyph_px(
                            12.0,
                            || Glyph::Close,
                            || color(status_warn_fg()),
                        )),
                ),
        )
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn reset_broken_config_action(
    controls: Arc<Controls>,
    config_broken: Signal<Option<String>>,
    config_broken_dismissed: Signal<bool>,
    profiles: ProfileSignals,
    port_text: Signal<String>,
    listen_all: Signal<bool>,
    langs: Signal<Option<Lang>>,
) {
    let dev = config::is_dev_mode();
    let mut current = config::Config::default_for(dev);
    current.listen_port =
        present::parse_port(&port_text.get()).unwrap_or_else(|| lock_port(&controls.listen_port));
    current.listen_all = listen_all.get();
    current.use_system_proxy = lock_bool(&controls.use_system_proxy);
    current.language = config::language_name(langs.get());
    current.active_profile = profiles.name.get();
    current.filter = profiles.filter.get();
    current.bypass_hosts = profiles.bypass.get();
    current.rules = profiles.rules.get();
    current.upstream_proxy = profiles.upstream.get();
    current.rules_enabled = profiles.rules_on.get();
    current.breaks_enabled = profiles.breaks_on.get();
    current.show_tunnel_failures = profiles.show_tunnels.get();
    current.proxy_auth = profiles.proxy_auth.get();
    current.relay_url = profiles.relay_url.get();
    current.relay_token = lock_string(&controls.relay_token);
    current.relay_accept = profiles.relay_accept.get();
    current.list_width = profiles.list_width.get().round() as u32;
    current.saved_profiles = profiles.saved.get();
    current.profile_history = profiles.history.get();
    current.redact_export = lock_bool(&controls.redact_export);
    current.redact_strict = lock_bool(&controls.redact_strict);
    current.redact_fields = lock_string(&controls.redact_fields);
    current.capturing = lock_bool(&controls.running);
    let win_size = window();
    if win_size.width.0.is_finite()
        && win_size.height.0.is_finite()
        && win_size.width.0 > 0.0
        && win_size.height.0 > 0.0
    {
        current.window_width = config::clamp_window_width(win_size.width.0.round() as u32);
        current.window_height = config::clamp_window_height(win_size.height.0.round() as u32);
    }
    match config::backup_and_reset_mode(&ca::data_dir(), dev, &current) {
        Ok(backup) => {
            *controls
                .config_error
                .lock()
                .unwrap_or_else(|err| err.into_inner()) = None;
            config_broken.set(None);
            config_broken_dismissed.set(false);
            set_status(
                &controls,
                StatusMsg::ConfigReset {
                    backup: backup.display().to_string(),
                },
            );
        }
        Err(err) => {
            set_status(&controls, StatusMsg::SaveFailed(err.to_string()));
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn reset_broken_config_action(
    controls: Arc<Controls>,
    config_broken: Signal<Option<String>>,
    config_broken_dismissed: Signal<bool>,
    _profiles: ProfileSignals,
    _port_text: Signal<String>,
    _listen_all: Signal<bool>,
    _langs: Signal<Option<Lang>>,
) {
    spawn(async move {
        let body = b"{\"op\":\"reset_broken_config\"}".to_vec();
        let request = HttpRequest::post("api/hub", body).header("content-type", "application/json");
        match fetch(request).await {
            Ok(response) => {
                let value: serde_json::Value =
                    serde_json::from_str(&response.text()).unwrap_or(serde_json::json!({}));
                if response.ok() {
                    let backup = value
                        .get("backup")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("config.toml.broken")
                        .to_string();
                    *controls
                        .config_error
                        .lock()
                        .unwrap_or_else(|err| err.into_inner()) = None;
                    config_broken.set(None);
                    config_broken_dismissed.set(false);
                    set_status(&controls, StatusMsg::ConfigReset { backup });
                } else {
                    let err = value
                        .get("error")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("reset failed")
                        .to_string();
                    set_status(&controls, StatusMsg::SaveFailed(err));
                }
            }
            Err(err) => set_status(&controls, StatusMsg::SaveFailed(err)),
        }
    });
}

const BREAK_QUEUE_LIMIT: usize = 8;

fn paused_flow_summary(controls: &Controls, id: u64) -> String {
    controls.hub.with_session(|s| {
        let Some(flow) = s.get(id) else {
            return String::new();
        };
        let target = format!("{}{}", flow.host, flow.path);
        let short_target = truncate_tail(&target, 34);
        if flow.method.is_empty() {
            short_target
        } else if short_target.is_empty() {
            flow.method.clone()
        } else {
            format!("{} {short_target}", flow.method)
        }
    })
}

fn truncate_tail(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

pub(crate) fn break_bar(
    paused: Signal<Option<(u64, BreakSide)>>,
    paused_count: Signal<usize>,
    paused_queue: Signal<Vec<(u64, BreakSide)>>,
    compose: Compose,
    selected: Signal<Option<u64>>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let banner_controls = controls.clone();
    let ttl_controls = controls.clone();
    let ttl_secs = signal(banner_controls.hub.paused_remaining_secs());
    let ttl_job = stored(None::<Spawned<()>>);
    effect(move || {
        let _ = paused.get();
        ttl_secs.set(ttl_controls.hub.paused_remaining_secs());
        if ttl_job.with(|j| j.is_none()) {
            let tick_controls = ttl_controls.clone();
            ttl_job.replace(Some(set_interval(Duration::from_secs(1), move || {
                ttl_secs.set(tick_controls.hub.paused_remaining_secs());
            })));
        }
    });
    let release = controls.clone();
    let release_all = controls.clone();
    let chip_controls = controls.clone();
    let drop_controls = controls;
    view()
        .min_h(px(40.0))
        .pl(px(LIST_INSET))
        .pr_3()
        .py_1p5()
        .gap_1p5()
        .bg(status_warn_bg())
        .border_b_1()
        .border_b_color(status_warn_fg())
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(
                    view()
                        .flex_1()
                        .min_w(px(180.0))
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .child(
                            text({
                                let langs = ui_langs();
                                move || -> SharedString {
                                    let lang = ui_lang(langs);
                                    let count = paused_count.get();
                                    let queue_suffix = if count > 1 {
                                        match lang {
                                            Lang::Zh => format!(" (1 / {count}, 上限 {BREAK_QUEUE_LIMIT})"),
                                            Lang::En => format!(" (1 / {count}, max {BREAK_QUEUE_LIMIT})"),
                                        }
                                    } else {
                                        String::new()
                                    };
                                    let ttl_suffix = match ttl_secs.get() {
                                        Some(secs) if secs <= 15 => match lang {
                                            Lang::Zh => format!(" · 剩 {secs}s"),
                                            Lang::En => format!(" · {secs}s left"),
                                        },
                                        _ => String::new(),
                                    };
                                    match paused.get() {
                                        Some((id, side)) => {
                                            let which = match side {
                                                BreakSide::Request => {
                                                    t(lang, "停在请求断点", "paused on request")
                                                }
                                                BreakSide::Response => {
                                                    t(lang, "停在响应断点", "paused on response")
                                                }
                                            };
                                            let ctx = paused_flow_summary(&banner_controls, id);
                                            if ctx.is_empty() {
                                                format!(
                                                    "{} #{id} {which}{queue_suffix}{ttl_suffix}",
                                                    t(lang, "流量", "Flow")
                                                )
                                            } else {
                                                format!(
                                                    "#{id} {ctx} · {which}{queue_suffix}{ttl_suffix}"
                                                )
                                            }
                                            .into()
                                        }
                                        None => "".into(),
                                    }
                                }
                            })
                            .flex_1()
                            .min_w(px(0.0))
                            .number_of_lines(1)
                            .text_overflow(EllipsizeMode::Tail)
                            .text_size(px(TEXT_SM))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(status_warn_fg()),
                        )
                        .child(when(
                            move || paused.get().is_some_and(|(id, _)| selected.get() != Some(id)),
                            move || {
                                view()
                                    .role(AccessibilityRole::Button)
                                    .h(px(ControlSize::Xs.height()))
                                    .px_2()
                                    .flex_shrink_0()
                                    .rounded(px(RADIUS_SM))
                                    .border_1()
                                    .border_color(status_warn_fg())
                                    .bg(bg_card())
                                    .hover(|s| s.bg(status_warn_bg_hover()))
                                    .active(|s| s.bg(bg_control_active()))
                                    .transition_colors()
                                    .duration(MOTION_INSTANT_MS)
                                    .ease_out()
                                    .focus_visible(apply_focus_ring)
                                    .items_center()
                                    .justify_center()
                                    .tooltip(label_tip("在列表中选中并查看此暂停流量", "Select and inspect paused flow"))
                                    .on_press(move || {
                                        if let Some((id, _)) = paused.get() {
                                            selected.set(Some(id));
                                        }
                                    })
                                    .child(
                                        text(tr("查看", "View"))
                                            .text_size(px(TEXT_XS))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(status_warn_fg()),
                                    )
                            },
                        )),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .child(
                            view()
                                .role(AccessibilityRole::Button)
                                .h(px(CONTROL_H))
                                .px_2p5()
                                .rounded(px(RADIUS_MD))
                                .bg(accent_blue())
                                .hover(|style| style.bg(accent_blue_hover()))
                                .active(|style| style.bg(accent_blue_active()))
                                .transition_colors()
                                .duration(MOTION_INSTANT_MS)
                                .ease_out()
                                .focus_visible(apply_focus_ring)
                                .flex_row()
                                .items_center()
                                .tooltip(label_tip(
                                    "放行当前断点流量（若已编辑则会应用修改；60 秒未操作将自动放行）",
                                    "Continue paused flow with current edits (auto-continues after 60s)",
                                ))
                                .on_press(move || {
                                    continue_break(&release, compose, selected, paused);
                                })
                                .child(
                                    text(tr("放行", "Continue"))
                                        .number_of_lines(1)
                                        .text_size(px(TEXT_SM))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(fg_on_accent()),
                                ),
                        )
                        .child(when(
                            move || paused_count.get() > 1,
                            move || {
                                let controls = release_all.clone();
                                view()
                                    .tooltip(label_tip(
                                        "一次放行全部排队中的断点流量",
                                        "Continue all queued paused flows",
                                    ))
                                    .child(detail_button(tr("全部放行", "Continue all"), move || {
                                        if !break_all_allowed() {
                                            return;
                                        }
                                        if compose.composing.get()
                                            && compose.paused_id.get().is_some()
                                        {
                                            reset_compose(compose);
                                        }
                                        controls.hub.resolve_all_breaks();
                                    }))
                            },
                        ))
                        .child(
                            view()
                                .role(AccessibilityRole::Button)
                                .ml_2()
                                .h(px(CONTROL_H))
                                .px_2p5()
                                .rounded(px(RADIUS_MD))
                                .bg(danger_button_bg())
                                .hover(|style| style.bg(danger_button_hover()))
                                .active(|style| style.bg(danger_button_active()))
                                .transition_colors()
                                .duration(MOTION_INSTANT_MS)
                                .ease_out()
                                .focus_visible(apply_focus_ring)
                                .flex_row()
                                .items_center()
                                .tooltip(label_tip(
                                    "中断并丢弃当前断点连接",
                                    "Drop and abort the paused connection",
                                ))
                                .on_press(move || {
                                    let paused_now =
                                        drop_controls.hub.paused().or_else(|| paused.get());
                                    if !break_action_allowed(paused_now.map(|(id, _)| id)) {
                                        return;
                                    }
                                    if compose.composing.get()
                                        && compose
                                            .paused_id
                                            .get()
                                            .is_some_and(|pid| paused_now.is_some_and(|(id, _)| id == pid))
                                    {
                                        reset_compose(compose);
                                    }
                                    let _ = drop_controls.hub.resolve_break(BreakDecision {
                                        drop: true,
                                        headers: None,
                                        body: None,
                                        status: None,
                                    });
                                })
                                .child(
                                    text(tr("丢弃", "Drop"))
                                        .number_of_lines(1)
                                        .text_size(px(TEXT_SM))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(fg_on_accent()),
                                ),
                        ),
                ),
        )
        .child(when(
            move || paused_queue.with(|queue| queue.len() > 1),
            move || {
                let controls = chip_controls.clone();
                view().flex_row().flex_wrap().gap_1().child(each(
                    move || paused_queue.get(),
                    |item| (item.0, matches!(item.1, BreakSide::Response)),
                    move |item, index| {
                        let controls = controls.clone();
                        let label_controls = controls.clone();
                        let is_head = move || index.get() == 0;
                        view()
                            .role(AccessibilityRole::Button)
                            .h(px(ControlSize::Xs.height()))
                            .max_w(px(260.0))
                            .overflow_hidden()
                            .px_2()
                            .rounded(px(RADIUS_SM))
                            .border_1()
                            .border_color(move || {
                                if is_head() {
                                    accent_blue_border()
                                } else {
                                    border_control()
                                }
                            })
                            .bg(move || {
                                if is_head() {
                                    accent_blue_soft()
                                } else {
                                    bg_control()
                                }
                            })
                            .hover(|style| {
                                style.bg(move || {
                                    if is_head() {
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
                            .tooltip(label_tip(
                                "点击切换处理此排队断点（最多排队 8 条）",
                                "Focus queued breakpoint (up to 8 queued)",
                            ))
                            .on_press(move || {
                                let id = item.with(|pair| pair.0);
                                focus_queued_break(&controls, id, selected, compose);
                            })
                            .child(
                                text({
                                    let langs = ui_langs();
                                    move || -> SharedString {
                                        let (id, side) = item.with(|pair| *pair);
                                        let which = match side {
                                            BreakSide::Request => {
                                                t(ui_lang(langs), "请求", "Req")
                                            }
                                            BreakSide::Response => {
                                                t(ui_lang(langs), "响应", "Res")
                                            }
                                        };
                                        let ctx = paused_flow_summary(&label_controls, id);
                                        if ctx.is_empty() {
                                            format!("#{id} {which}").into()
                                        } else {
                                            format!("#{id} {ctx} · {which}").into()
                                        }
                                    }
                                })
                                .min_w(px(0.0))
                                .flex_shrink(1.0)
                                .tabular_nums()
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Tail)
                                .text_size(px(TEXT_XS))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || {
                                    if is_head() {
                                        accent_blue_text()
                                    } else {
                                        fg_secondary()
                                    }
                                }),
                            )
                    },
                ))
            },
        ))
}

pub(crate) fn focus_queued_break(
    controls: &Controls,
    id: u64,
    selected: Signal<Option<u64>>,
    compose: Compose,
) {
    let already =
        selected.get() == Some(id) && controls.hub.paused().is_some_and(|(cur, _)| cur == id);
    if !controls.hub.promote_break(id) {
        return;
    }
    if !already && compose.composing.get() && compose.paused_id.get().is_some() {
        reset_compose(compose);
    }
    selected.set(Some(id));
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) static LAST_BREAK_RESOLVE: Mutex<Option<(u64, Instant)>> = Mutex::new(None);
#[cfg(not(target_arch = "wasm32"))]
pub(crate) static LAST_BREAK_ALL: Mutex<Option<Instant>> = Mutex::new(None);
#[cfg(target_arch = "wasm32")]
pub(crate) static LAST_BREAK_RESOLVE: Mutex<Option<(u64, SystemTime)>> = Mutex::new(None);
#[cfg(target_arch = "wasm32")]
pub(crate) static LAST_BREAK_ALL: Mutex<Option<SystemTime>> = Mutex::new(None);

pub(crate) fn break_action_allowed(flow_id: Option<u64>) -> bool {
    // `Instant::now` panics on wasm32. The debounce there uses the wall clock.
    #[cfg(not(target_arch = "wasm32"))]
    let now = Instant::now();
    #[cfg(target_arch = "wasm32")]
    let now = clock::now();
    let mut lock = LAST_BREAK_RESOLVE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(id) = flow_id {
        if let Some((last_id, last_time)) = *lock
            && last_id == id
            && debounce_elapsed(now, last_time) < Duration::from_millis(300)
        {
            return false;
        }
        *lock = Some((id, now));
    }
    true
}

pub(crate) fn break_all_allowed() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    let now = Instant::now();
    #[cfg(target_arch = "wasm32")]
    let now = clock::now();
    let mut lock = LAST_BREAK_ALL.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(last_time) = *lock
        && debounce_elapsed(now, last_time) < Duration::from_millis(300)
    {
        return false;
    }
    *lock = Some(now);
    true
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn debounce_elapsed(now: Instant, earlier: Instant) -> Duration {
    now.duration_since(earlier)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn debounce_elapsed(now: SystemTime, earlier: SystemTime) -> Duration {
    now.duration_since(earlier).unwrap_or_default()
}

pub(crate) fn continue_break(
    controls: &Controls,
    compose: Compose,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
) {
    let current_paused = controls.hub.paused().or_else(|| paused.get());
    if !break_action_allowed(current_paused.map(|(id, _)| id)) {
        return;
    }
    let edited = compose.composing.get()
        && current_paused.is_some_and(|(id, side)| {
            (compose.paused_id.get() == Some(id) || selected.get() == Some(id))
                && compose.response.get() == matches!(side, BreakSide::Response)
        });
    let decision = if edited {
        BreakDecision {
            drop: false,
            headers: Some(parse_header_text(&compose.headers.get())),
            body: Some(compose.body.get().into_bytes()),
            status: edited_status(compose),
        }
    } else {
        BreakDecision {
            drop: false,
            headers: None,
            body: None,
            status: None,
        }
    };
    if compose.composing.get()
        && compose
            .paused_id
            .get()
            .is_some_and(|pid| current_paused.is_some_and(|(id, _)| id == pid))
    {
        reset_compose(compose);
    }
    if !controls.hub.resolve_break(decision) {
        set_status(controls, StatusMsg::BreakDone);
    }
}
