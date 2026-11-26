#[allow(unused_imports)]
use super::*;

/// The root view: owns process-lifetime runtime, tray lifecycle, and dynamically mounts/unmounts
/// the desktop window workspace.
#[component]
pub fn App() -> impl IntoElement {
    #[cfg(not(target_arch = "wasm32"))]
    crate::ensure_desktop_runtime_init();
    crate::startup_metrics::mark_app_mount();
    let initial_cfg = saved_config();
    let rt = init_app_runtime(&initial_cfg);
    let show_tray = rt.show_tray;
    let minimize_to_tray = rt.minimize_to_tray;
    #[cfg(not(target_arch = "wasm32"))]
    let win_mount_seq = Rc::new(Cell::new(0_u64));
    #[cfg(not(target_arch = "wasm32"))]
    {
        use nsh_core::desktop::QuitMode;
        use nsh_core::element::desktop::{on_reopen, set_quit_mode};

        let show_mode = rt.show_tray;
        let min_mode = rt.minimize_to_tray;
        effect(move || {
            let mode = if show_mode.get() && min_mode.get() {
                QuitMode::Explicit
            } else {
                QuitMode::LastWindowClosed
            };
            set_quit_mode(mode);
        });
        let reopen_win = rt.main_win;
        let reopen_open = rt.main_open;
        on_reopen(move || reveal_main_window(reopen_win, reopen_open));

        let tray_controls = rt.controls.clone();
        let tray_rt = rt.clone();
        let tray_clear_rev = rt.tray_clear_rev;
        let tray_win = rt.main_win;
        let tray_open = rt.main_open;
        install_system_tray(TrayProps {
            controls: rt.controls.clone(),
            langs: rt.langs,
            show_tray: rt.show_tray,
            main_win: rt.main_win,
            main_open: rt.main_open,
            running: rt.running,
            listen: rt.listen,
            use_system_proxy: rt.use_system_proxy,
            on_clear: Rc::new(move || {
                if clear_captured_session(&tray_controls) {
                    clear_runtime_selection(&tray_rt);
                    tray_clear_rev.set(tray_clear_rev.get_untracked().wrapping_add(1));
                }
                reveal_main_window(tray_win, tray_open);
            }),
        });
        #[cfg(target_os = "macos")]
        install_macos_application_menu(rt.about_open, rt.settings_open, rt.main_win, rt.main_open);
        install_background_maintenance(&rt);
    }
    let mut title_bar = nsh_core::desktop::TitleBarOptions::default();
    title_bar.style = nsh_core::desktop::TitleBarStyle::Hidden;
    title_bar.height = Some(px(TOOLBAR_H));
    let mut base_attrs = nsh_core::desktop::WindowAttributes::default();
    base_attrs.title_bar = Some(title_bar);
    base_attrs.min_size = Some(Some(Size {
        width: px(960.0),
        height: px(640.0),
    }));
    let rt_change = rt.clone();
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut main_window_node = nsh_core::element::desktop::window()
        .id("main")
        .title("zniff")
        .default_size(Size {
            width: px(initial_cfg.window_width as f32),
            height: px(initial_cfg.window_height as f32),
        })
        .attributes(base_attrs)
        .attributes(move || {
            let mut attrs = nsh_core::desktop::WindowAttributes::default();
            attrs.close_on_press = Some(!(show_tray.get() && minimize_to_tray.get()));
            attrs
        })
        .open(rt.main_open)
        .on_open_change(move |ev| {
            #[cfg(not(target_arch = "wasm32"))]
            on_main_window_open_change(&rt_change, ev.open);
            #[cfg(target_arch = "wasm32")]
            rt_change.main_open.set(ev.open);
        });
    #[cfg(not(target_arch = "wasm32"))]
    {
        use nsh_core::desktop::{DisplayId, WindowMode, WindowPosition};

        if let Some(id) = initial_cfg
            .window_display_id
            .as_deref()
            .and_then(config::parse_window_display_id)
        {
            main_window_node = main_window_node.display(DisplayId(id));
        }
        if let (Some(x), Some(y)) = (initial_cfg.window_x, initial_cfg.window_y) {
            main_window_node = main_window_node.default_position(WindowPosition::At(Point {
                x: px(x as f32),
                y: px(y as f32),
            }));
        }
        if let Some(mode) = match initial_cfg.window_mode.as_str() {
            "maximized" => Some(WindowMode::Maximized),
            "fullscreen" => Some(WindowMode::FullScreen),
            _ => None,
        } {
            main_window_node = main_window_node.default_mode(mode);
        }
        let rt_close = rt.clone();
        main_window_node = main_window_node.on_close_press(move |_| {
            if rt_close.show_tray.get_untracked() && rt_close.minimize_to_tray.get_untracked() {
                close_main_window_to_tray(&rt_close);
            } else {
                let win = rt_close
                    .main_win
                    .get_untracked()
                    .unwrap_or_else(nsh_core::element::desktop::current_window);
                win.close();
            }
        });
    }
    main_window_node.content(move || {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let seq = win_mount_seq.get().wrapping_add(1);
            win_mount_seq.set(seq);
            let win = nsh_core::element::desktop::current_window();
            rt.main_win.set(Some(win.clone()));
            if rt.has_mounted_once.replace(true) {
                if let Some(placement) = rt.last_placement.get() {
                    win.set_placement(placement);
                }
                win.focus();
            }
            let seq_ref = Rc::clone(&win_mount_seq);
            let win_sig = rt.main_win;
            cleanup(move || {
                if seq_ref.get() == seq {
                    win_sig.try_set(None);
                }
            });
        }
        app_workspace(rt.clone())
    })
}

fn app_workspace(rt: AppRuntime) -> impl IntoElement {
    let controls = rt.controls.clone();
    let langs = rt.langs;
    let show_tray = rt.show_tray;
    let minimize_to_tray = rt.minimize_to_tray;
    let status = signal(StatusMsg::Idle);
    let running = rt.running;
    let ca_status = signal(ca::CaStatus::NotInstalled);
    let active_external_ca = signal(None::<ca::ExternalCaSummary>);
    let detected_external_cas = signal(Vec::<ca::ExternalCaSummary>::new());
    let ca_ui = CaSignals {
        status: ca_status,
        active_external: active_external_ca,
        detected_external: detected_external_cas,
    };
    let use_system_proxy = rt.use_system_proxy;
    let saved = saved_config();
    let filter = rt.filter;
    let filter_enabled = rt.filter_enabled;
    let list_filter = rt.list_filter;
    let initial_filter = filter.get_untracked();
    let initial_filter_enabled = filter_enabled.get_untracked();
    let initial_list_filter = list_filter.get_untracked();
    let initial_effective_filter = session::combine_filters(
        if initial_filter_enabled {
            &initial_filter
        } else {
            ""
        },
        &initial_list_filter,
    );
    let effective_filter = signal(initial_effective_filter.clone());
    effect(move || {
        let primary = if filter_enabled.get() {
            filter.get()
        } else {
            String::new()
        };
        effective_filter.set(session::combine_filters(&primary, &list_filter.get()));
    });
    let saved_profiles = signal(saved.saved_profiles.clone());
    let profile_history = signal(saved.profile_history.clone());
    let selected = rt.selected;
    let baseline = rt.baseline;
    let sort = rt.sort;
    let columns = signal(ListColumns::from_config(&saved));
    let initial_lang = ui_lang(langs);
    let row_cache_tick = Rc::new(RefCell::new(RowCache::new(initial_lang)));
    let initial_snapshot = {
        let mut cache = row_cache_tick.borrow_mut();
        compute_workspace_initial_snapshot(&rt, columns.get_untracked(), initial_lang, &mut cache)
    };
    let memory = signal(initial_snapshot.memory);
    let dropped = signal(initial_snapshot.dropped);
    let rows = signal(initial_snapshot.rows);
    let filtered_stats = signal(initial_snapshot.filtered_stats);
    let empty_diagnosis = signal(initial_snapshot.empty_diagnosis);
    let facets = signal(initial_snapshot.facets);
    let filter_catalog = signal(initial_snapshot.filter_catalog);
    provide_context(filter_catalog);
    let popover_dismiss = provide_popover_dismiss();
    let facets_open = signal(false);
    let flow_total = signal(initial_snapshot.flow_total);
    let session_tabs_sig = signal(initial_snapshot.session_tabs);
    let active_tab_sig = signal(initial_snapshot.active_tab);
    let onboarding_traffic = signal(initial_snapshot.onboarding_traffic);
    let tls_failure_burst = signal(initial_snapshot.tls_failure_burst);
    let tls_banner_dismissed = signal(false);
    let listen = signal(String::new());
    let shown = Shown {
        phase: signal(Phase::Empty),
        method: signal(String::new()),
        url: signal(String::new()),
        protocol: signal(String::new()),
        rules: signal(String::new()),
        rule_hits: signal(Vec::new()),
        gql_badge: signal(String::new()),
        status: signal(None),
        error: signal(String::new()),
        request_headers: signal(Vec::new()),
        response_headers: signal(Vec::new()),
        request_body: signal(String::new()),
        response_body: signal(String::new()),
        request_body_bytes: signal(0),
        response_body_bytes: signal(0),
        request_hex: signal(Vec::new()),
        response_hex: signal(Vec::new()),
        pane_width: signal(0.0),
        request_language: signal(String::new()),
        response_language: signal(String::new()),
        request_doc: signal(None),
        response_doc: signal(None),
        req_doc_preview: signal(false),
        res_doc_preview: signal(false),
        request_json: signal(None),
        response_json: signal(None),
        req_json_tree: signal(saved.default_json_tree && saved.default_pretty),
        res_json_tree: signal(saved.default_json_tree && saved.default_pretty),
        request_form: signal(Vec::new()),
        request_note: signal(String::new()),
        response_note: signal(String::new()),
        metrics: signal(String::new()),
        timing_bar: signal(present::TimingBreakdown::default()),
        query: signal(Vec::new()),
        preview: signal(String::new()),
        preview_w: signal(0.0),
        preview_h: signal(0.0),
        image_caption: signal(String::new()),
        pretty: signal(saved.default_pretty),
        default_json_tree: signal(saved.default_json_tree),
        messages: signal(Vec::new()),
        llm_stream: signal(None),
        msg_llm_view: signal(true),
        diff: signal(String::new()),
        diff_old: signal(String::new()),
        diff_new: signal(String::new()),
        diff_baseline_label: signal(String::new()),
        diff_scope: signal(0_u8),
        pinned: signal(false),
        is_marker: signal(false),
        comment: signal(String::new()),
        editing_comment: signal(false),
        repeating: signal(false),
        repeat_count: signal(present::REPEAT_COUNT_DEFAULT.to_string()),
        repeat_interval: signal(present::REPEAT_INTERVAL_DEFAULT_MS.to_string()),
        repeat_concurrency: signal(present::REPEAT_CONCURRENCY_DEFAULT.to_string()),
        req_search: signal(String::new()),
        res_search: signal(String::new()),
        msg_search: signal(String::new()),
        msg_dir: signal(0),
        msg_kind: signal(0),
        relay: signal(String::new()),
        request_offset: signal(0),
        response_offset: signal(0),
        request_window: signal(TextWindow::default()),
        response_window: signal(TextWindow::default()),
        req_headers_loading: signal(false),
        res_headers_loading: signal(false),
        req_body_loading: signal(false),
        res_body_loading: signal(false),
        messages_loading: signal(false),
        diff_loading: signal(false),
    };
    let port_text = signal(String::new());
    let listen_all = signal(false);
    let web_disconnected = signal(None::<String>);
    let paused = signal(initial_snapshot.paused);
    let paused_count = signal(initial_snapshot.paused_count);
    let paused_queue = signal(initial_snapshot.paused_queue);
    let folded = signal(folded_bits_from_config(&saved.folded_cards));
    let compose = Compose {
        composing: signal(false),
        response: signal(false),
        paused_id: signal(None),
        method: signal(String::new()),
        url: signal(String::new()),
        headers: signal(String::new()),
        body: signal(String::new()),
        status: signal(String::new()),
        draft_saved: signal((String::new(), String::new(), String::new(), String::new())),
        recent: signal(
            saved
                .compose_history
                .iter()
                .map(|item| {
                    (
                        item.method.clone(),
                        item.url.clone(),
                        item.headers.clone(),
                        item.body.clone(),
                    )
                })
                .collect(),
        ),
        inline_mock: InlineMockSignals::new(),
    };
    effect(move || {
        crate::startup_metrics::mark_first_frame();
        if compose.composing.get() && !compose.response.get() && selected.get().is_none() {
            compose.draft_saved.set((
                compose.method.get(),
                compose.url.get(),
                compose.headers.get(),
                compose.body.get(),
            ));
        }
    });
    let seq_diagram = signal(present::SequenceDiagramData::default());
    let seq_viewport_w = signal(0.0_f32);
    let seq_builder = SeqBuilder {
        open: signal(false),
        items: signal(Vec::new()),
        resume_ids: signal(Vec::new()),
        resume_view_mode: signal(0),
        include_response: signal(true),
        include_query: signal(false),
        output: signal(String::new()),
        diagram: seq_diagram,
        view_mode: signal(0),
        source_flowchart: signal(false),
        hover_bar: signal(None),
        redact_open: signal(false),
        redact_export: signal(saved.redact_export),
        redact_strict: signal(saved.redact_strict),
        redact_fields: signal(saved.redact_fields.clone()),
        redact_flow_id: signal(None),
        viewport_w: seq_viewport_w,
        track_w: signal(0.0_f32),
        layout: computed(move || seq_diagram.with(|d| swimlane_layout(d, seq_viewport_w.get()))),
        flat_rows: computed(move || seq_diagram.with(build_dep_flat_rows)),
    };
    effect(move || {
        let has_selected = selected.get().is_some();
        let is_composing = compose.composing.get();
        if has_selected || is_composing {
            close_sequence_diagram(seq_builder);
        }
    });
    let saved_active_layout = saved.active_layout.clone();
    let saved_layout_pane_sizes = saved.layout_pane_sizes.clone();
    let list_width = signal(saved.list_width as f32);
    let initial_profile_name = if !saved.active_profile.trim().is_empty() {
        saved.active_profile.clone()
    } else {
        matched_saved_profile(
            &saved_profiles.get(),
            &saved.filter,
            &saved.bypass_hosts,
            &saved.rules,
            &saved.upstream_proxy,
        )
        .unwrap_or_default()
    };
    let bypass_text = signal(saved.bypass_hosts);
    #[cfg(not(target_arch = "wasm32"))]
    let rules_text = signal(controls.hub.rules_text());
    #[cfg(target_arch = "wasm32")]
    let rules_text = signal(saved.rules);
    #[cfg(not(target_arch = "wasm32"))]
    let rule_groups_sig = signal(controls.hub.rule_groups());
    #[cfg(target_arch = "wasm32")]
    let rule_groups_sig = signal(saved.rule_groups);
    let editing_group_sig = signal(None::<String>);
    let upstream_text = signal(saved.upstream_proxy);
    let proxy_auth_sig = signal(config::runtime_proxy_auth(&saved.proxy_auth));
    let allow_runtime = config::runtime_allow_hosts(&saved.allow_hosts);
    let allow_hosts_sig = signal(allow_runtime.text);
    let allow_hosts_strict_sig = signal(allow_runtime.strict);
    let allow_hosts_locked_sig = signal(allow_runtime.locked);
    let allow_denied_sig = signal(initial_snapshot.allow_denied);
    let relay_url_sig = signal(saved.relay_url);
    let relay_accept_sig = signal(saved.relay_accept);
    let profile_signals = ProfileSignals {
        saved: saved_profiles,
        history: profile_history,
        name: signal(initial_profile_name),
        filter,
        filter_enabled,
        bypass: bypass_text,
        rules: rules_text,
        rule_groups: rule_groups_sig,
        editing_group: editing_group_sig,
        focused_rule_line: signal(None),
        upstream: upstream_text,
        rules_on: signal(saved.rules_enabled),
        breaks_on: signal(saved.breaks_enabled),
        show_tunnels: signal(saved.show_tunnel_failures),
        proxy_auth: proxy_auth_sig,
        allow_hosts: allow_hosts_sig,
        allow_hosts_strict: allow_hosts_strict_sig,
        allow_hosts_locked: allow_hosts_locked_sig,
        allow_denied: allow_denied_sig,
        relay_url: relay_url_sig,
        relay_accept: relay_accept_sig,
        list_width,
        list_height: signal(saved.list_height as f32),
        sidebar_width: signal(config::clamp_sidebar_width(saved.sidebar_width) as f32),
        sidebar_collapsed: signal(saved.sidebar_collapsed),
        layout_mode: signal(saved.layout_mode),
        panel_open: signal(false),
        rules_panel_tab: signal(0),
        filter_panel_open: signal(false),
        preset_panel_open: signal(false),
        import_open: signal(false),
        access_open: signal(false),
        quick_rule_prefill: signal(None),
        machine_settings_open: signal(false),
        settings_open: rt.settings_open,
        settings_tab: signal(SettingsTab::General),
        shortcuts_open: signal(false),
        marker_modal_open: signal(false),
        watch_open: signal(false),
        about_open: rt.about_open,
        default_json_tree: shown.default_json_tree,
        show_tray,
        minimize_to_tray,
        onboarding_strip_dismissed: signal(saved.onboarding_strip_dismissed),
        deselect_rev: signal(0),
    };
    let lan_url = signal(String::new());
    let lan_choice = signal(String::new());
    let lan_addrs = signal(Vec::<LanAddr>::new());
    let qr_path = signal(String::new());
    let qr_open = signal(false);
    let pending_qr = signal(false);
    let adb_count = signal({
        #[cfg(not(target_arch = "wasm32"))]
        {
            launcher::configured_adb_count()
        }
        #[cfg(target_arch = "wasm32")]
        {
            0usize
        }
    });
    let tp_active = signal(false);
    #[cfg(not(target_arch = "wasm32"))]
    let tp_status = signal(crate::transparent_proxy::cached_status());
    #[cfg(not(target_arch = "wasm32"))]
    let tp_apps = signal(crate::transparent_proxy::cached_running_apps());
    #[cfg(not(target_arch = "wasm32"))]
    refresh_transparent_proxy_state_async();
    #[cfg(not(target_arch = "wasm32"))]
    let tp_probe_at = Cell::new(clock::now());
    let config_broken = signal(lock_config_error(&controls));
    let default_json_tree_effect = shown.default_json_tree;
    let shown_tree_effect = shown;
    effect(move || {
        let on = default_json_tree_effect.get();
        let pretty_on = untracked(|| shown_tree_effect.pretty.get());
        let active = on && pretty_on;
        shown_tree_effect.req_json_tree.set(active);
        shown_tree_effect.res_json_tree.set(active);
    });
    let config_broken_dismissed = signal(false);
    let update_state = signal(lock_update_state(&controls));
    status.set(lock_status(&controls));
    running.set(lock_bool(&controls.running));
    listen.set(lock_string(&controls.listen));
    ca_status.set(lock_ca_status(&controls));
    active_external_ca.set(lock_active_external_ca(&controls));
    detected_external_cas.set(lock_external_cas(&controls));
    use_system_proxy.set(lock_bool(&controls.use_system_proxy));
    port_text.set(lock_port(&controls.listen_port).to_string());
    listen_all.set(lock_bool(&controls.listen_all));
    #[cfg(target_arch = "wasm32")]
    {
        let web_bg = signal(lifecycle() == Lifecycle::Background);
        effect(move || {
            let bg = lifecycle() == Lifecycle::Background;
            if web_bg.get_untracked() != bg {
                web_bg.set(bg);
                if !bg {
                    proxy::notify_poll_wake();
                }
            }
        });
        start_web_poll(
            controls.clone(),
            port_text,
            listen_all,
            profile_signals,
            effective_filter,
            seq_builder,
            selected,
            baseline,
            web_bg,
            web_disconnected,
        );
        start_web_detail_watch(controls.clone(), selected, baseline);
    }

    let seen_tray_clear = stored(rt.tray_clear_rev.get_untracked());
    let tray_clear_rev = rt.tray_clear_rev;
    effect(move || {
        let rev = tray_clear_rev.get();
        if seen_tray_clear.replace(rev) != rev {
            selected.set(None);
            baseline.set(None);
            reset_compose(compose);
            close_sequence_diagram(seq_builder);
            clear_shown(shown);
        }
    });

    let rt_tick = rt.clone();
    let controls_tick = controls.clone();
    let filter_tick = filter;
    let list_filter_tick = list_filter;
    let effective_filter_tick = effective_filter;
    let empty_diagnosis_tick = empty_diagnosis;
    let profile_tick = profile_signals;
    let filter_pending = stored(filter.get());
    let filter_since = stored(None::<SystemTime>);
    let history_pending = stored((
        filter.get(),
        bypass_text.get(),
        rules_text.get(),
        upstream_text.get(),
        saved.rules_enabled,
        saved.breaks_enabled,
        saved.show_tunnel_failures,
    ));
    let history_since = stored(Some(clock::now()));
    let win_pending = stored(WindowPlacementSnapshot {
        width: saved.window_width,
        height: saved.window_height,
        x: saved.window_x,
        y: saved.window_y,
        display_id: saved.window_display_id.clone(),
        mode: saved.window_mode.clone(),
    });
    let win_since = stored(None::<SystemTime>);
    #[cfg(not(target_arch = "wasm32"))]
    let saved_display_id = stored(
        saved
            .window_display_id
            .as_deref()
            .and_then(config::parse_window_display_id),
    );
    #[cfg(not(target_arch = "wasm32"))]
    let display_checked = stored(false);
    let selected_tick = selected;
    let rows_tick = rows;
    let stats_tick = filtered_stats;
    let facets_tick = facets;
    let filter_catalog_tick = filter_catalog;
    let facets_open_tick = facets_open;
    let total_tick = flow_total;
    let onboarding_traffic_tick = onboarding_traffic;
    let tls_burst_tick = tls_failure_burst;
    let tls_dismissed_tick = tls_banner_dismissed;
    let shown_tick = shown;
    if initial_snapshot.selected.is_some() {
        publish_detail_for_controls(
            &shown_tick,
            &controls_tick,
            initial_snapshot.selected,
            initial_lang,
            initial_snapshot.baseline,
        );
    }
    let (initial_detail_stamp, _) = compute_detail_stamp(
        &controls_tick,
        initial_snapshot.active_tab,
        initial_snapshot.selected,
        initial_snapshot.baseline,
        shown_tick.pretty.get_untracked(),
        initial_lang,
        shown_tick.diff_scope.get_untracked(),
    );
    let seen_gen = stored(initial_snapshot.generation);
    let seen_filter = stored(initial_effective_filter);
    let seen_setup_filter = stored(initial_filter);
    let seen_selected = stored(initial_snapshot.selected);
    // One follow-up after the selection frame. Layout can put the previous
    // offset back once; the same flow updating does not scroll.
    let detail_scroll = element_ref();
    let detail_scroll_tick = detail_scroll;
    let workspace_ref = element_ref();
    provide_context(SidebarLayoutRef(workspace_ref));
    let detail_top_left = stored(false);
    let seen_detail_stamp = stored(initial_detail_stamp);
    let seen_tabs_ver = stored(initial_snapshot.tabs_ver);
    let seen_active_tab = stored(initial_snapshot.active_tab);
    let seen_live_count = stored(initial_snapshot.live_count);
    let session_tabs_tick = session_tabs_sig;
    let active_tab_tick = active_tab_sig;
    let seen_listen = stored(String::new());
    #[cfg(target_arch = "wasm32")]
    let seen_lan_rev = stored(u64::MAX);
    let seen_lang = stored(initial_lang);
    let baseline_tick = baseline;
    let sort_tick = sort;
    let seen_sort = stored(sort.get_untracked());
    let cols_tick = columns;
    let layout_mode_tick = profile_signals.layout_mode;
    let seen_path_display = stored(effective_path_display(
        columns.get(),
        &effective_filter.get_untracked(),
    ));
    let seen_paused = stored(None::<(u64, BreakSide)>);
    let compose_tick = compose;
    let share_id = signal(String::new());
    let share_id_tick = share_id;
    let seen_share = stored(String::new());
    let update_tick = update_state;
    let seen_update = stored(update_state.get());
    let config_broken_tick = config_broken;
    let config_dismissed_tick = config_broken_dismissed;
    let seen_config_broken = stored(config_broken.get());
    let seen_facets_open = stored(false);
    let detail_hold = stored(SystemTime::UNIX_EPOCH);
    let refresh_throttle = Rc::new(RefCell::new(RefreshThrottle::default()));
    let toast_manager = stored(ToastManager::<u64, StatusMsg>::new(ToastMotion::sonner()));
    let toast_shown = signal(Vec::<(u64, StatusMsg, ToastTransitionStatus)>::new());
    let toast_state = ToastStackState::new();
    let toast_overlay_gen = signal(0_u64);
    let clear_toast_id = stored(None::<u64>);
    let zoom_toast_id = stored(None::<u64>);
    let toast_manager_tick = toast_manager;
    let toast_shown_tick = toast_shown;
    let watch_rules = rt.watch_rules;
    let watch_eval = rt.watch_eval;
    let toast_state_tick = toast_state;
    let toast_gen_tick = toast_overlay_gen;
    let clear_toast_tick = clear_toast_id;
    let zoom_toast_tick = zoom_toast_id;
    // The hex text's own box is only as wide as the current line, so measuring
    // it always says 16 bytes fit. The pane viewport is the width the card
    // actually has. Publishing it from this tick rebuilds the text; a signal
    // write inside `on_layout` can miss that text.
    let pane_px = Rc::new(Cell::new(0.0_f32));
    let pane_tick = pane_px.clone();
    let seen_rules_rev = stored(controls_tick.hub.rules_revision());
    let seen_denied_gen = stored(initial_snapshot.generation);
    set_interval(Duration::from_millis(16), move || {
        controls_tick.hub.expire_clear_undo();
        let pending_toasts = {
            let mut q = controls_tick
                .toast_queue
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            std::mem::take(&mut *q)
        };
        if !pending_toasts.is_empty() {
            if toast_shown_tick.with(Vec::is_empty)
                && let Some((last_id, _)) = pending_toasts.last()
            {
                toast_gen_tick.set(*last_id);
            }
            let now = nsh_core::time::now();
            let next_toasts = toast_manager_tick.with(|m| {
                for (tid, msg) in pending_toasts {
                    if matches!(msg, StatusMsg::ClearedFlows { .. }) {
                        if let Some(prev_id) = clear_toast_tick.get() {
                            m.dismiss(&prev_id, now);
                        }
                        clear_toast_tick.replace(Some(tid));
                    }
                    if matches!(msg, StatusMsg::UiZoomChanged(_)) {
                        if let Some(prev_id) = zoom_toast_tick.get() {
                            m.dismiss(&prev_id, now);
                        }
                        zoom_toast_tick.replace(Some(tid));
                    }
                    let timeout_ms = match &msg {
                        StatusMsg::ClearedFlows { .. } => 5000,
                        StatusMsg::UiZoomChanged(_) => 1600,
                        StatusMsg::UpdateUpToDate(_)
                        | StatusMsg::UpdateAvailable(_)
                        | StatusMsg::UpdateCheckFailed(_) => 4000,
                        _ => 2800,
                    };
                    m.push(
                        tid,
                        msg,
                        ToastOptions::timeout(Some(Duration::from_millis(timeout_ms))),
                        now,
                    );
                }
                m.visible(3)
                    .map(|(id, data, status)| (*id, data.clone(), status))
                    .collect::<Vec<_>>()
            });
            toast_shown_tick.set(next_toasts);
        }
        let now_instant = nsh_core::time::now();
        let next_after_advance = toast_manager_tick.with(|m| {
            if m.advance(now_instant, toast_state_tick.is_expanded())
                .changed
            {
                Some(
                    m.visible(3)
                        .map(|(id, data, status)| (*id, data.clone(), status))
                        .collect::<Vec<_>>(),
                )
            } else {
                None
            }
        });
        if let Some(next_toasts) = next_after_advance {
            toast_shown_tick.set(next_toasts);
        }
        let width = pane_tick.get();
        let published = shown_tick.pane_width.get_untracked();
        if width > 0.0 && (width - published).abs() > 1.0 {
            shown_tick.pane_width.set(width);
        }
        let share_now = controls_tick.hub.share_id().unwrap_or_default();
        if share_now != seen_share.get() {
            seen_share.replace(share_now.clone());
            share_id_tick.set(share_now);
        }
        let update_now = lock_update_state(&controls_tick);
        if update_now != seen_update.get() {
            seen_update.replace(update_now.clone());
            update_tick.set(update_now);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if clock::elapsed(rt_tick.update_probe_at.get()) >= Duration::from_secs(3600) {
            rt_tick.update_probe_at.replace(clock::now());
            if updater::auto_check_due(clock::now()) {
                auto_check_update(controls_tick.clone());
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(err) = proxy::take_save_error() {
                set_status(&controls_tick, StatusMsg::WebConfigUnsaved(err));
            }
            for err in proxy::drain_web_op_errors() {
                set_status(&controls_tick, StatusMsg::WebOpFailed(err));
            }
        }
        let cfg_err_now = current_config_error(&controls_tick);
        if cfg_err_now != seen_config_broken.get() {
            seen_config_broken.replace(cfg_err_now.clone());
            if cfg_err_now.is_some() {
                config_dismissed_tick.set(false);
            }
            config_broken_tick.set(cfg_err_now);
        }
        let status_now = lock_status(&controls_tick);
        if status.with(|cur| cur != &status_now) {
            status.set(status_now.clone());
        }
        let running_now = *controls_tick
            .running
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if running.get_untracked() != running_now {
            running.set(running_now);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let adb_now = launcher::configured_adb_count();
            if adb_count.get_untracked() != adb_now {
                adb_count.set(adb_now);
            }
            let tp_now = crate::transparent_proxy::cached_status();
            let tp_on = tp_now.is_on_or_starting();
            if tp_active.get_untracked() != tp_on {
                tp_active.set(tp_on);
            }
            if tp_status.with_untracked(|cur| cur != &tp_now) {
                tp_status.set(tp_now.clone());
            }
            let tp_apps_now = crate::transparent_proxy::cached_running_apps();
            if tp_apps.with_untracked(|cur| cur != &tp_apps_now) {
                tp_apps.set(tp_apps_now);
            }
            if tp_now.embedded
                && matches!(
                    tp_now.state,
                    crate::transparent_proxy::TpState::NeedsApproval
                        | crate::transparent_proxy::TpState::Starting
                        | crate::transparent_proxy::TpState::Stopping
                )
            {
                poll_transparent_proxy_if_pending(&tp_probe_at);
            }
        }
        if pending_qr.get_untracked() {
            if running_now {
                pending_qr.set(false);
                qr_open.set(true);
            } else if matches!(
                status_now,
                StatusMsg::BindFailed { .. } | StatusMsg::CaUnavailable(_)
            ) {
                pending_qr.set(false);
            }
        } else if !running_now && qr_open.get_untracked() {
            qr_open.set(false);
        }
        let listen_now = lock_string(&controls_tick.listen);
        if listen.with(|cur| cur != &listen_now) {
            listen.set(listen_now.clone());
        }
        let ca_now = lock_ca_status(&controls_tick);
        if ca_status.get_untracked() != ca_now {
            ca_status.set(ca_now);
        }
        let active_ext_now = lock_active_external_ca(&controls_tick);
        if active_external_ca.with_untracked(|cur| cur != &active_ext_now) {
            active_external_ca.set(active_ext_now);
        }
        let detected_ext_now = lock_external_cas(&controls_tick);
        if detected_external_cas.with_untracked(|cur| cur != &detected_ext_now) {
            detected_external_cas.set(detected_ext_now);
        }
        let sys_proxy_now = lock_bool(&controls_tick.use_system_proxy);
        if use_system_proxy.get_untracked() != sys_proxy_now {
            use_system_proxy.set(sys_proxy_now);
        }
        let rules_rev_now = controls_tick.hub.rules_revision();
        if rules_rev_now != seen_rules_rev.get() {
            seen_rules_rev.replace(rules_rev_now);
            let hub_rules = controls_tick.hub.rules_text();
            if hub_rules != profile_tick.rules.get_untracked() {
                profile_tick.rules.set(hub_rules);
            }
            let hub_groups = controls_tick.hub.rule_groups();
            if profile_tick
                .rule_groups
                .with_untracked(|cur| cur != &hub_groups)
            {
                if let Some(editing) = profile_tick.editing_group.get_untracked()
                    && !hub_groups
                        .iter()
                        .any(|g| g.name.eq_ignore_ascii_case(&editing))
                {
                    profile_tick.editing_group.set(None);
                }
                profile_tick.rule_groups.set(hub_groups);
            }
        }
        let paused_now = controls_tick.hub.paused();
        let paused_len = controls_tick.hub.paused_queue_len();
        let paused_queue_now = controls_tick.hub.paused_queue();
        if paused.get_untracked() != paused_now {
            paused.set(paused_now);
        }
        if paused_count.get_untracked() != paused_len {
            paused_count.set(paused_len);
        }
        if paused_queue.with(|cur| cur != &paused_queue_now) {
            paused_queue.set(paused_queue_now);
        }
        if paused_now != seen_paused.get() {
            let prev_paused = seen_paused.replace(paused_now);
            if let Some((paused_id, side)) = paused_now {
                if selected_tick.get().is_none() {
                    selected_tick.set(Some(paused_id));
                }
                if compose_tick.composing.get()
                    && selected_tick.get() == Some(paused_id)
                    && compose_tick.response.get() != matches!(side, BreakSide::Response)
                {
                    begin_compose(&controls_tick, Some(paused_id), compose_tick);
                }
            } else if let Some((prev_id, prev_side)) = prev_paused
                && compose_tick.composing.get()
                && (compose_tick.response.get()
                    || (prev_side == BreakSide::Request && selected_tick.get() == Some(prev_id)))
            {
                reset_compose(compose_tick);
            }
        }
        #[cfg(target_arch = "wasm32")]
        let lan_changed = {
            let rev = proxy::lan_revision();
            let changed = rev != seen_lan_rev.get();
            if changed {
                seen_lan_rev.replace(rev);
            }
            changed && is_listening_lan(&listen_now)
        };
        #[cfg(not(target_arch = "wasm32"))]
        let lan_changed = false;
        if listen_now != seen_listen.get() || lan_changed {
            seen_listen.replace(listen_now.clone());
            publish_lan(
                &listen_now,
                &lan_choice.get_untracked(),
                lan_addrs,
                lan_choice,
                lan_url,
                qr_path,
            );
        }
        let generation = controls_tick.hub.generation();
        if generation != seen_denied_gen.get() {
            seen_denied_gen.replace(generation);
            let denied_now = controls_tick.hub.allow_denied_summary();
            if profile_tick
                .allow_denied
                .with_untracked(|cur| cur != &denied_now)
            {
                profile_tick.allow_denied.set(denied_now);
            }
        }
        let filter_now = filter_tick.get();
        let effective_filter_now = effective_filter_tick.get();
        if filter_now != filter_pending.get() {
            filter_pending.replace(filter_now.clone());
            filter_since.replace(Some(clock::now()));
        } else if filter_since
            .get()
            .is_some_and(|at| clock::elapsed(at) >= Duration::from_millis(500))
        {
            remember_filter(&controls_tick, &filter_now);
            filter_since.replace(None);
        }
        let history_changed = history_pending.with(|pending| {
            pending.0 != filter_now
                || profile_tick.bypass.with_untracked(|s| s != &pending.1)
                || profile_tick.rules.with_untracked(|s| s != &pending.2)
                || profile_tick.upstream.with_untracked(|s| s != &pending.3)
                || profile_tick.rules_on.get_untracked() != pending.4
                || profile_tick.breaks_on.get_untracked() != pending.5
                || profile_tick.show_tunnels.get_untracked() != pending.6
        });
        if history_changed {
            history_pending.replace((
                filter_now.clone(),
                profile_tick.bypass.get_untracked(),
                profile_tick.rules.get_untracked(),
                profile_tick.upstream.get_untracked(),
                profile_tick.rules_on.get_untracked(),
                profile_tick.breaks_on.get_untracked(),
                profile_tick.show_tunnels.get_untracked(),
            ));
            history_since.replace(Some(clock::now()));
        } else if history_since
            .get()
            .is_some_and(|at| clock::elapsed(at) >= Duration::from_millis(1200))
        {
            history_pending.with(|pending| {
                record_profile_history(
                    &profile_tick.name.get_untracked(),
                    &pending.0,
                    &pending.1,
                    &pending.2,
                    &pending.3,
                    pending.4,
                    pending.5,
                    pending.6,
                    profile_tick.history,
                );
            });
            history_since.replace(None);
        }
        let win_now = {
            #[cfg(not(target_arch = "wasm32"))]
            let desktop_snap = {
                let state_opt = nsh_core::element::desktop::window_state();
                if !display_checked.get() && state_opt.is_some() {
                    let all_displays = nsh_core::element::desktop::displays();
                    if !all_displays.is_empty() {
                        display_checked.replace(true);
                        if let Some(expected_id) = saved_display_id.get()
                            && !all_displays.iter().any(|d| d.id.0 == expected_id)
                        {
                            nsh_core::element::desktop::current_window().center(None);
                        }
                    }
                }
                capture_placement_snapshot(state_opt).map(|(placement, snapshot)| {
                    rt_tick.last_placement.replace(Some(placement));
                    snapshot
                })
            };
            #[cfg(target_arch = "wasm32")]
            let desktop_snap: Option<WindowPlacementSnapshot> = None;

            desktop_snap.or_else(|| {
                let win_size = window();
                if win_size.width.0.is_finite()
                    && win_size.height.0.is_finite()
                    && win_size.width.0 > 0.0
                    && win_size.height.0 > 0.0
                {
                    let prev = win_pending.get();
                    Some(WindowPlacementSnapshot {
                        width: config::clamp_window_width(win_size.width.0.round() as u32),
                        height: config::clamp_window_height(win_size.height.0.round() as u32),
                        x: prev.x,
                        y: prev.y,
                        display_id: prev.display_id,
                        mode: prev.mode,
                    })
                } else {
                    None
                }
            })
        };
        if let Some(win_now) = win_now {
            if win_now != win_pending.get() {
                win_pending.replace(win_now);
                win_since.replace(Some(clock::now()));
            } else if win_since
                .get()
                .is_some_and(|at| clock::elapsed(at) >= Duration::from_millis(500))
            {
                remember_window_placement(&controls_tick, &win_now);
                win_since.replace(None);
            }
        }
        release_sequence_data(seq_builder);
        let spill_expired = if clock::elapsed(rt_tick.spill_gc_at.get()) >= Duration::from_secs(15)
        {
            let now = clock::now();
            rt_tick.spill_gc_at.replace(now);
            controls_tick
                .hub
                .with_session(|session| session.gc_spill(now))
        } else {
            false
        };
        let tabs_ver_now = session_tabs_version(&controls_tick);
        let active_tab_now = active_tab_id(&controls_tick);
        let tab_switched = active_tab_now != seen_active_tab.get();
        if tab_switched {
            let was_synced = active_tab_tick.get_untracked() == active_tab_now;
            seen_active_tab.replace(active_tab_now);
            active_tab_tick.set(active_tab_now);
            row_cache_tick.borrow_mut().rows.clear();
            if !was_synced && selected_tick.get_untracked().is_some() {
                selected_tick.set(None);
            }
            baseline_tick.set(baseline_id_for_active_tab(&controls_tick));
        }
        let live_count_now = controls_tick.hub.with_session(|s| s.len());
        if tabs_ver_now != seen_tabs_ver.get() || live_count_now != seen_live_count.get() {
            seen_tabs_ver.replace(tabs_ver_now);
            seen_live_count.replace(live_count_now);
            let next_summaries = session_tab_summaries(&controls_tick, live_count_now);
            if session_tabs_tick.with_untracked(|cur| cur != &next_summaries) {
                session_tabs_tick.set(next_summaries);
            }
        }
        if let Some((source_id, replayed_id)) = take_pending_replay_diff(&controls_tick) {
            if apply_replay_auto_diff(
                &controls_tick.hub,
                source_id,
                replayed_id,
                selected_tick,
                baseline_tick,
                folded,
                compose_tick,
            ) {
                if let Some(src_flow) = controls_tick.hub.with_session(|s| s.hydrated(source_id)) {
                    controls_tick
                        .session_tabs
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .set_baseline_ref(LIVE_TAB_ID, src_flow);
                }
            } else {
                queue_pending_replay_diff(&controls_tick, source_id, replayed_id);
            }
        }
        let selected_now = selected_tick.get();
        let lang_now = ui_lang(langs);
        let baseline_now = baseline_tick.get();
        let pretty_now = shown_tick.pretty.get();
        let sort_now = sort_tick.get();
        let path_display_now = effective_path_display(cols_tick.get(), &effective_filter_now);
        let facets_open_now = facets_open_tick.get();
        let layout_mode_now = layout_mode_tick.get();
        let facets_needed = true;
        let facets_opened = facets_open_now && !seen_facets_open.get();
        seen_facets_open.replace(facets_open_now);
        let interactive_changed = effective_filter_now != seen_filter.get()
            || filter_now != seen_setup_filter.get()
            || lang_now != seen_lang.get()
            || sort_now != seen_sort.get()
            || path_display_now != seen_path_display.get()
            || spill_expired
            || facets_opened
            || tab_switched;
        let gen_changed = active_tab_now == LIVE_TAB_ID && generation != seen_gen.get();
        let refresh = refresh_throttle.borrow_mut().decide(
            clock::now(),
            total_tick.get_untracked(),
            interactive_changed,
            gen_changed,
        );
        let list_changed = refresh.run_list;
        let run_secondary = refresh.run_secondary;
        let want_stats = selected_now.is_none();
        let stats_only = !list_changed && want_stats && seen_selected.get().is_some();
        run_watch_evaluation(&rt_tick, generation);
        if list_changed {
            seen_gen.replace(generation);
            seen_filter.replace(effective_filter_now.clone());
            seen_setup_filter.replace(filter_now.clone());
            seen_lang.replace(lang_now);
            seen_sort.replace(sort_now);
            seen_path_display.replace(path_display_now);
            let matched = active_matching_ids(&controls_tick, &effective_filter_now);
            let filter_enabled_now = profile_tick.filter_enabled.get();
            let active_primary = if filter_enabled_now {
                filter_now.as_str()
            } else {
                ""
            };
            let facet_ids = if run_secondary
                && (facets_open_now || layout_mode_now == config::LayoutMode::Split)
                && active_primary != effective_filter_now
            {
                Some(active_matching_ids(&controls_tick, active_primary))
            } else {
                None
            };
            let list_filter_now = list_filter_tick.get();
            let (
                lines,
                stats,
                next_facets,
                next_catalog,
                next_total,
                next_mem,
                next_dropped,
                diag_plan,
            ) = {
                let mut cache = row_cache_tick.borrow_mut();
                with_active_session(&controls_tick, |session| {
                    let lines = project_rows(
                        session,
                        &matched,
                        lang_now,
                        sort_now,
                        path_display_now,
                        &mut cache,
                    );
                    let stats = (want_stats && run_secondary).then(|| {
                        session::FilteredStats::from_flows(
                            matched.iter().filter_map(|(id, _)| session.get(*id)),
                        )
                    });
                    let ids = facet_ids.as_ref().unwrap_or(&matched);
                    let next_facets = (facets_needed && run_secondary).then(|| {
                        session::SessionFacets::from_flows(
                            ids.iter().filter_map(|(id, _)| session.get(*id)),
                        )
                    });
                    let next_catalog = next_facets.as_ref().map(|f| {
                        FilterSuggestionsCatalog::from_flows(
                            ids.iter().filter_map(|(id, _)| session.get(*id)),
                            f,
                        )
                    });
                    let diag_plan = (matched.is_empty()
                        && session.len() > 0
                        && !effective_filter_now.trim().is_empty())
                    .then(|| {
                        plan_empty_filter_diagnosis(
                            session,
                            &filter_now,
                            filter_enabled_now,
                            &list_filter_now,
                        )
                    });
                    (
                        lines,
                        stats,
                        next_facets,
                        next_catalog,
                        session.len(),
                        format_bytes(session.memory_bytes()),
                        session.dropped(),
                        diag_plan,
                    )
                })
            };
            let (next_onboarding_traffic, next_tls_burst) = {
                let burst_now = clock::now();
                controls_tick.hub.with_session(|live| {
                    (
                        OnboardingTraffic::from_flows(live.iter()),
                        TlsFailureBurst::from_flows(live.iter(), burst_now),
                    )
                })
            };
            if let Some(next_catalog) = next_catalog
                && filter_catalog_tick.with(|cur| cur != &next_catalog)
            {
                filter_catalog_tick.set(next_catalog);
            }
            let next_diagnosis = diag_plan
                .map(FilterDiagnosisPlan::finish)
                .unwrap_or_default();
            if empty_diagnosis_tick.with(|cur| cur != &next_diagnosis) {
                empty_diagnosis_tick.set(next_diagnosis);
            }
            if total_tick.get_untracked() != next_total {
                total_tick.set(next_total);
            }
            if onboarding_traffic_tick.get_untracked() != next_onboarding_traffic {
                onboarding_traffic_tick.set(next_onboarding_traffic);
            }
            if next_total == 0 && tls_dismissed_tick.get_untracked() {
                tls_dismissed_tick.set(false);
            }
            if tls_burst_tick.get_untracked() != next_tls_burst {
                tls_burst_tick.set(next_tls_burst);
            }
            if let Some(next_facets) = next_facets
                && facets_tick.with(|cur| cur != &next_facets)
            {
                facets_tick.set(next_facets);
            }
            let had_rows = rows_tick.with(|cur| !cur.is_empty());
            if !rows_tick.with(|cur| same_row_arcs(cur, &lines)) {
                rows_tick.set(lines);
            }
            if had_rows && next_total == 0 {
                // The session was just emptied: the list copies are freed only now,
                // after the clear's own trim, so hand those pages back as well.
                crate::session::trim_memory();
            }
            if let Some(stats) = stats
                && stats_tick.with(|cur| cur != &stats)
            {
                stats_tick.set(stats);
            }
            if memory.with(|cur| cur != &next_mem) {
                memory.set(next_mem);
            }
            if dropped.get_untracked() != next_dropped {
                dropped.set(next_dropped);
            }
        } else if stats_only {
            let matched = active_matching_ids(&controls_tick, &effective_filter_now);
            let stats = with_active_session(&controls_tick, |session| {
                session::FilteredStats::from_flows(
                    matched.iter().filter_map(|(id, _)| session.get(*id)),
                )
            });
            if stats_tick.with(|cur| cur != &stats) {
                stats_tick.set(stats);
            }
        }
        if selected_now != seen_selected.get() {
            seen_selected.replace(selected_now);
            shown_tick.req_doc_preview.set(false);
            shown_tick.res_doc_preview.set(false);
            let tree_default = shown_tick.default_json_tree.get() && shown_tick.pretty.get();
            shown_tick.req_json_tree.set(tree_default);
            shown_tick.res_json_tree.set(tree_default);
            shown_tick.editing_comment.set(false);
            shown_tick.repeating.set(false);
            shown_tick.req_search.set(String::new());
            shown_tick.res_search.set(String::new());
            shown_tick.msg_search.set(String::new());
            shown_tick.msg_dir.set(0);
            shown_tick.msg_kind.set(0);
            detail_scroll_tick.scroll_to_offset(px(0.0), false);
            detail_top_left.replace(true);
        } else if detail_top_left.get() {
            detail_scroll_tick.scroll_to_offset(px(0.0), false);
            detail_top_left.replace(false);
        }
        let (detail_stamp, selected_done) = compute_detail_stamp(
            &controls_tick,
            active_tab_now,
            selected_now,
            baseline_now,
            pretty_now,
            lang_now,
            shown_tick.diff_scope.get(),
        );
        let baseline_key_now = detail_stamp.4;
        if detail_stamp != seen_detail_stamp.get() || spill_expired {
            let prev = seen_detail_stamp.get();
            let view_changed = prev.1 != active_tab_now
                || prev.2 != selected_now
                || prev.4 != baseline_key_now
                || prev.6 != pretty_now
                || prev.7 != lang_now
                || prev.8 != detail_stamp.8
                || prev.9 != detail_stamp.9;
            let streaming = selected_now.is_some() && !selected_done;
            let due = clock::elapsed(detail_hold.get()) >= Duration::from_millis(200);
            if !streaming || view_changed || spill_expired || due {
                seen_detail_stamp.replace(detail_stamp);
                detail_hold.replace(clock::now());
                if prev.1 != active_tab_now || prev.2 != selected_now || prev.6 != pretty_now {
                    reset_body_offsets(shown_tick);
                }
                publish_detail_for_controls(
                    &shown_tick,
                    &controls_tick,
                    selected_now,
                    lang_now,
                    baseline_now,
                );
            }
        }
    });

    let controls_start = controls.clone();
    let controls_ca = controls.clone();
    let controls_remove = controls.clone();
    let controls_proxy = controls.clone();
    let controls_clear = controls.clone();
    let controls_clear_unpinned = controls.clone();
    let controls_clear_key = controls_clear.clone();
    let controls_replay_key = controls.clone();
    let controls_export = controls.clone();
    let controls_env = controls.clone();
    let selected_clear = selected;
    let baseline_clear = baseline;
    let compose_clear = compose;
    let selected_clear_unpinned = selected;
    let baseline_clear_unpinned = baseline;
    let compose_clear_unpinned = compose;
    let controls_detail = controls.clone();
    let controls_break = controls.clone();
    let controls_cfg_bar = controls.clone();
    let controls_list = controls.clone();
    let controls_split = controls.clone();
    let controls_import = controls.clone();
    let controls_drop = controls.clone();
    let drop_langs = langs;
    let filter_focus = element_ref();
    let rows_step = rows;
    let selected_step = selected;
    let compose_step = compose;
    on_key_with(
        "up",
        KeyOptions {
            repeat: true,
            ..KeyOptions::default()
        },
        move |_| step_selected(rows_step, selected_step, compose_step, -1),
    );
    on_key_with(
        "down",
        KeyOptions {
            repeat: true,
            ..KeyOptions::default()
        },
        move |_| step_selected(rows_step, selected_step, compose_step, 1),
    );
    let selected_clear_key = selected;
    let baseline_clear_key = baseline;
    let compose_clear_key = compose;
    let seq_clear_key = seq_builder;
    let shown_clear_key = shown;
    on_key("mod+k", move |_| {
        clear_captured(
            &controls_clear_key,
            selected_clear_key,
            baseline_clear_key,
            compose_clear_key,
            seq_clear_key,
            shown_clear_key,
        );
    });
    on_key("mod+f", move |_| filter_focus.focus());
    let selected_replay = selected;
    let controls_submit_key = controls_replay_key.clone();
    on_key("mod+r", move |_| {
        replay_flow(
            &controls_replay_key,
            selected_replay.get(),
            ReplayOverride::default(),
        );
    });
    let marker_open_key = profile_signals.marker_modal_open;
    let controls_marker_key = controls.clone();
    on_key("mod+shift+m", move |_| {
        if guard_read_only_tab(&controls_marker_key) {
            return;
        }
        marker_open_key.set(!marker_open_key.get());
    });
    let compose_submit_key = compose;
    let selected_submit_key = selected;
    let paused_submit_key = paused;
    let profiles_submit_key = profile_signals;
    on_key_with(
        "mod+enter",
        KeyOptions {
            when: nsh_core::keymap::When::Always,
            ..KeyOptions::default()
        },
        move |_| {
            if compose_submit_key.composing.get() {
                submit_compose(
                    &controls_submit_key,
                    compose_submit_key,
                    selected_submit_key,
                    paused_submit_key,
                );
            } else if compose_submit_key.inline_mock.open.get() {
                save_inline_mock(
                    &controls_submit_key,
                    compose_submit_key,
                    profiles_submit_key,
                );
            }
        },
    );
    let selected_esc = selected;
    let compose_esc = compose;
    let seq_esc = seq_builder;
    let shown_esc = shown;
    let palette_open = signal(false);
    let shortcuts_open = profile_signals.shortcuts_open;
    on_key_with(
        "mod+,",
        KeyOptions {
            when: nsh_core::keymap::When::Always,
            ..KeyOptions::default()
        },
        move |_| {
            let next = !profile_signals.settings_open.get();
            profile_signals.settings_open.set(next);
        },
    );
    on_key_with(
        "mod+shift+p",
        KeyOptions {
            when: nsh_core::keymap::When::Always,
            ..KeyOptions::default()
        },
        move |_| {
            palette_open.set(!palette_open.get());
        },
    );
    on_key("shift+/", move |_| {
        shortcuts_open.set(!shortcuts_open.get());
    });
    on_key_with(
        "mod+/",
        KeyOptions {
            when: nsh_core::keymap::When::Always,
            ..KeyOptions::default()
        },
        move |_| {
            shortcuts_open.set(!shortcuts_open.get());
        },
    );
    let controls_zoom_in = controls.clone();
    on_key_with(
        "mod+=",
        KeyOptions {
            when: nsh_core::keymap::When::Always,
            ..KeyOptions::default()
        },
        move |_| step_ui_zoom(&controls_zoom_in, config::UI_ZOOM_STEP as i16),
    );
    let controls_zoom_plus = controls.clone();
    on_key_with(
        "mod+shift+=",
        KeyOptions {
            when: nsh_core::keymap::When::Always,
            ..KeyOptions::default()
        },
        move |_| step_ui_zoom(&controls_zoom_plus, config::UI_ZOOM_STEP as i16),
    );
    let controls_zoom_out = controls.clone();
    on_key_with(
        "mod+-",
        KeyOptions {
            when: nsh_core::keymap::When::Always,
            ..KeyOptions::default()
        },
        move |_| step_ui_zoom(&controls_zoom_out, -(config::UI_ZOOM_STEP as i16)),
    );
    let controls_zoom_reset = controls.clone();
    on_key_with(
        "mod+0",
        KeyOptions {
            when: nsh_core::keymap::When::Always,
            ..KeyOptions::default()
        },
        move |_| reset_ui_zoom(&controls_zoom_reset),
    );
    on_key("escape", move |_| {
        if compose_esc.composing.get() || compose_esc.inline_mock.open.get() {
            reset_compose(compose_esc);
        } else if seq_esc.open.get() {
            close_sequence_diagram(seq_esc);
        } else if seq_esc.redact_open.get() {
            seq_esc.redact_open.set(false);
        } else if shown_esc.repeating.get() {
            shown_esc.repeating.set(false);
        } else if shown_esc.editing_comment.get() {
            shown_esc.editing_comment.set(false);
        } else if selected_esc.get().is_some() {
            selected_esc.set(None);
        }
    });

    let layout_ui = init_layout_ui_state(
        &saved_active_layout,
        &saved_layout_pane_sizes,
        profile_signals.layout_mode.get_untracked(),
    );
    provide_context(profile_signals);
    provide_context(layout_ui);

    view()
        .flex_1()
        .bg(bg_canvas())
        .text_color(fg_primary())
        .on_file_drop(move |event: &FileDropEvent| {
            import_dropped(&controls_drop, event, ui_lang(drop_langs), profile_signals);
        })
        .child(toolbar(
            status,
            running,
            ca_ui,
            use_system_proxy,
            move || install_ca(controls_ca.clone()),
            move || remove_ca(controls_remove.clone()),
            move || toggle_system_proxy(controls_proxy.clone()),
            move || {
                clear_unpinned_captured(
                    &controls_clear_unpinned,
                    selected_clear_unpinned,
                    baseline_clear_unpinned,
                    compose_clear_unpinned,
                    seq_builder,
                    shown,
                );
            },
            move || export_har_file(controls_export.clone(), &effective_filter.get()),
            move || copy_proxy_env(&controls_env),
            move || toggle_capture(controls_start.clone()),
            controls_import,
            rows,
            flow_total,
            effective_filter,
            shown.pretty,
            shown.phase,
            share_id,
            profile_signals,
            compose,
            seq_builder,
            selected,
            update_state,
        ))
        .child(when(
            move || config_broken.get().is_some() && !config_broken_dismissed.get(),
            move || {
                config_broken_bar(
                    config_broken,
                    config_broken_dismissed,
                    profile_signals,
                    port_text,
                    listen_all,
                    controls_cfg_bar.clone(),
                )
            },
        ))
        .child(when(
            move || paused.get().is_some(),
            move || {
                break_bar(
                    paused,
                    paused_count,
                    paused_queue,
                    compose,
                    selected,
                    controls_break.clone(),
                )
            },
        ))
        .child({
            let install_ca_cb: Rc<dyn Fn()> = {
                let c = controls.clone();
                Rc::new(move || install_ca(c.clone()))
            };
            let active_layout = layout_ui.active_spec;
            let standalone_summary = signal(active_layout.get_untracked().has_standalone_summary());
            let seen_layout_key = stored((
                String::new(),
                profile_signals.layout_mode.get_untracked(),
                0_i32,
                0_i32,
                u64::MAX,
            ));
            effect(move || {
                let bounds = window();
                let w_bucket = (bounds.width.0 / 20.0).round() as i32;
                let h_bucket = (bounds.height.0 / 20.0).round() as i32;
                let id = layout_ui.active_id.get();
                let mode = profile_signals.layout_mode.get();
                let rev = layout_ui.reload_rev.get();
                let key = (id, mode, w_bucket, h_bucket, rev);
                if seen_layout_key.with(|cur| *cur != key) {
                    seen_layout_key.replace(key);
                    evaluate_active_layout(
                        layout_ui,
                        mode,
                        crate::layout_spec::LayoutContext {
                            width: bounds.width.0.max(320.0),
                            height: bounds.height.0.max(240.0),
                            target: if cfg!(target_arch = "wasm32") {
                                crate::layout_spec::LayoutTarget::Web
                            } else {
                                crate::layout_spec::LayoutTarget::Desktop
                            },
                        },
                    );
                }
            });
            effect(move || {
                let has_summary = active_layout.with(|s| s.has_standalone_summary());
                if standalone_summary.get_untracked() != has_summary {
                    standalone_summary.set(has_summary);
                }
            });
            provide_context(StandaloneSummaryContext(standalone_summary));
            provide_context(TlsFailureBannerSignals {
                burst: tls_failure_burst,
                dismissed: tls_banner_dismissed,
                ca_status,
                controls: controls.clone(),
                selected,
                install_ca: Rc::clone(&install_ca_cb),
            });
            provide_context(SessionTabBarProps {
                tabs: session_tabs_sig,
                active_tab: active_tab_sig,
                controls: controls.clone(),
                selected,
                baseline,
                compose,
                seq_builder,
            });
            provide_context(seq_builder);
            provide_context(DiagramResumeRows(rows));
            let toggle_sys_cb: Rc<dyn Fn()> = {
                let c = controls.clone();
                Rc::new(move || toggle_system_proxy(c.clone()))
            };
            let onboarding_base = OnboardingProps {
                controls: controls.clone(),
                running,
                listen,
                ca_ui,
                use_system_proxy,
                adb_count,
                flow_total,
                onboarding_traffic,
                compose,
                selected,
                profiles: profile_signals,
                install_ca: install_ca_cb,
                toggle_system_proxy: toggle_sys_cb,
                qr_open,
                pending_qr,
            };
            let onboarding_strip_props = onboarding_base.clone();
            workspace_ref.on_focus_in(move || popover_dismiss.dismiss());
            let on_clear: Rc<dyn Fn()> = Rc::new(move || {
                clear_captured(
                    &controls_clear,
                    selected_clear,
                    baseline_clear,
                    compose_clear,
                    seq_builder,
                    shown,
                );
            });
            let controls_summary = controls.clone();
            let workspace_host = WorkspaceLayoutHost {
                workspace_ref,
                profiles: profile_signals,
                list_width,
                controls: controls_split.clone(),
                pane_overrides: layout_ui.active_pane_overrides,
                on_save_pane_size: Rc::new(move |path, size| {
                    save_pane_size_override(layout_ui, path, size, &controls_split);
                }),
                modules: WorkspaceModules {
                    sidebar: Rc::new(move || {
                        facet_sidebar(
                            facets,
                            list_filter,
                            profile_signals.sidebar_width,
                            profile_signals.sidebar_collapsed,
                        )
                        .into_element()
                    }),
                    flow_list: Rc::new(move || {
                        let clear_controls = controls_list.clone();
                        let empty_state = EmptyFilterState {
                            effective_filter,
                            main_filter: filter,
                            filter_enabled,
                            list_filter,
                            diagnosis: empty_diagnosis,
                            running,
                            on_clear_primary: Some(Rc::new(move || {
                                save_config_or_notify(&clear_controls, PrefName::Filter, |cfg| {
                                    cfg.filter.clear();
                                });
                            })),
                        };
                        flow_list(
                            rows,
                            facets,
                            facets_open,
                            selected,
                            paused,
                            compose,
                            empty_state,
                            profile_signals,
                            columns,
                            sort,
                            controls_list.clone(),
                            shown,
                            baseline,
                            filter_focus,
                            Rc::clone(&on_clear),
                        )
                        .into_element()
                    }),
                    detail: Rc::new(move || {
                        detail_pane(
                            shown,
                            controls_detail.clone(),
                            selected,
                            paused,
                            compose,
                            seq_builder,
                            rows,
                            list_filter,
                            effective_filter,
                            baseline,
                            folded,
                            Rc::clone(&pane_px),
                            filtered_stats,
                            detail_scroll,
                            profile_signals.layout_mode,
                            onboarding_base.clone(),
                        )
                        .into_element()
                    }),
                    summary: Rc::new(move || {
                        summary_pane(
                            filtered_stats,
                            selected,
                            controls_summary.clone(),
                            rows,
                            seq_builder,
                            compose,
                            shown.phase,
                        )
                        .into_element()
                    }),
                },
            };
            let build_footer: Rc<dyn Fn() -> AnyElement> = {
                let controls = controls.clone();
                Rc::new(move || {
                    footer(
                        listen,
                        running,
                        memory,
                        dropped,
                        port_text,
                        lan_url,
                        lan_choice,
                        lan_addrs,
                        qr_path,
                        listen_all,
                        AccessControlProps {
                            open: profile_signals.access_open,
                            proxy_auth: profile_signals.proxy_auth,
                            allow_hosts: profile_signals.allow_hosts,
                            allow_hosts_strict: profile_signals.allow_hosts_strict,
                            allow_hosts_locked: profile_signals.allow_hosts_locked,
                            allow_denied: profile_signals.allow_denied,
                            controls: controls.clone(),
                        },
                        config_broken,
                        config_broken_dismissed,
                        ConnectHubProps {
                            controls: controls.clone(),
                            running,
                            listen,
                            ca_ui,
                            use_system_proxy,
                            adb_count,
                            tp_active,
                            #[cfg(not(target_arch = "wasm32"))]
                            tp_status,
                            #[cfg(not(target_arch = "wasm32"))]
                            tp_apps,
                            selected,
                            install_ca: {
                                let c = controls.clone();
                                Rc::new(move || install_ca(c.clone()))
                            },
                            toggle_system_proxy: {
                                let c = controls.clone();
                                Rc::new(move || toggle_system_proxy(c.clone()))
                            },
                            qr_open,
                            pending_qr,
                        },
                        WatchPopoverProps {
                            open: profile_signals.watch_open,
                            rules: watch_rules,
                            eval: watch_eval,
                            effective_filter,
                            list_filter,
                            selected,
                            active_tab: active_tab_sig,
                            baseline,
                            primary_filter: filter,
                            filter_enabled: profile_signals.filter_enabled,
                            controls: controls.clone(),
                        },
                        controls.clone(),
                    )
                    .into_element()
                })
            };
            let build_footer_top = Rc::clone(&build_footer);
            let build_footer_bottom = build_footer;
            view()
                .flex_1()
                .min_h(px(0.0))
                .flex_col()
                .child(when(
                    move || active_layout.with(|s| s.footer) == crate::layout_spec::FooterDock::Top,
                    move || (build_footer_top)(),
                ))
                .child(web_disconnected_banner(web_disconnected))
                .child(layout_error_banner(
                    layout_ui,
                    profile_signals,
                    controls.clone(),
                ))
                .child(onboarding_progress_strip(
                    onboarding_strip_props,
                    shown.phase,
                ))
                .child(render_workspace_layout(active_layout, workspace_host))
                .child(when(
                    move || {
                        active_layout.with(|s| s.footer) == crate::layout_spec::FooterDock::Bottom
                    },
                    move || (build_footer_bottom)(),
                ))
        })
        .child(machine_settings_modal(profile_signals, controls.clone()))
        .child(settings_window(SettingsWindowProps {
            profiles: profile_signals,
            controls: controls.clone(),
            ca_ui,
            running,
            listen,
            proxy_on: use_system_proxy,
            port_text,
            listen_all,
            pretty: shown.pretty,
            seq_builder,
            rows,
            compose,
            selected,
            shown_phase: shown.phase,
            update_state,
            install_ca: {
                let c = controls.clone();
                Rc::new(move || {
                    #[cfg(not(target_arch = "wasm32"))]
                    install_ca(c.clone());
                    #[cfg(target_arch = "wasm32")]
                    let _ = &c;
                })
            },
            remove_ca: {
                let c = controls.clone();
                Rc::new(move || {
                    #[cfg(not(target_arch = "wasm32"))]
                    remove_ca(c.clone());
                    #[cfg(target_arch = "wasm32")]
                    let _ = &c;
                })
            },
            toggle_proxy: {
                let c = controls.clone();
                Rc::new(move || {
                    #[cfg(not(target_arch = "wasm32"))]
                    toggle_system_proxy(c.clone());
                    #[cfg(target_arch = "wasm32")]
                    let _ = &c;
                })
            },
        }))
        .child(about_modal(profile_signals, update_state, controls.clone()))
        .child(command_palette_modal(
            palette_open,
            shortcuts_open,
            filter_focus,
            controls.clone(),
            running,
            use_system_proxy,
            rows,
            effective_filter,
            shown.pretty,
            profile_signals,
            compose,
            seq_builder,
            selected,
            baseline,
            shown,
        ))
        .child(shortcuts_modal(shortcuts_open))
        .child(marker_modal(
            profile_signals.marker_modal_open,
            controls.clone(),
        ))
        .child(toast_overlay(
            toast_shown,
            toast_overlay_gen,
            toast_state,
            toast_manager,
            controls.clone(),
        ))
}
