#[allow(unused_imports)]
use super::*;

/// The root view: a flow list and the selected flow.
#[component]
pub fn App() -> impl IntoElement {
    #[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
    {
        let mut bar = nsh_core::desktop::TitleBarOptions::default();
        bar.style = nsh_core::desktop::TitleBarStyle::Hidden;
        bar.height = Some(px(42.0));
        let mut attributes = nsh_core::desktop::WindowAttributes::default();
        attributes.title_bar = Some(bar);
        nsh_core::element::desktop::main_window().set(attributes);
    }
    let langs = signal(load_lang_pref());
    provide_context(langs);
    provide_mono_font(saved_config().system_mono_font);
    let status = signal(StatusMsg::Idle);
    let running = signal(false);
    let ca_status = signal(ca::CaStatus::NotInstalled);
    let use_system_proxy = signal(false);
    let memory = signal(String::from("0 B"));
    let dropped = signal(0_u64);
    let saved_filter = saved_config();
    let filter = signal(saved_filter.filter.clone());
    let list_filter = signal(String::new());
    let effective_filter = signal(saved_filter.filter);
    effect(move || {
        effective_filter.set(session::combine_filters(&filter.get(), &list_filter.get()));
    });
    let saved_profiles = signal(saved_filter.saved_profiles);
    let profile_history = signal(saved_filter.profile_history);
    let selected = signal(None::<u64>);
    let rows = signal(Vec::<Arc<RowLine>>::new());
    let filtered_stats = signal(session::FilteredStats::default());
    let facets = signal(session::SessionFacets::default());
    let facets_open = signal(false);
    let flow_total = signal(0_usize);
    let listen = signal(String::new());
    let saved = saved_config();
    let shown = Shown {
        phase: signal(Phase::Empty),
        method: signal(String::new()),
        url: signal(String::new()),
        protocol: signal(String::new()),
        rules: signal(String::new()),
        status: signal(None),
        error: signal(String::new()),
        request_headers: signal(Vec::new()),
        response_headers: signal(Vec::new()),
        request_body: signal(String::new()),
        response_body: signal(String::new()),
        request_hex: signal(Vec::new()),
        response_hex: signal(Vec::new()),
        pane_width: signal(0.0),
        request_language: signal(String::new()),
        response_language: signal(String::new()),
        request_doc: signal(None),
        response_doc: signal(None),
        req_doc_preview: signal(false),
        res_doc_preview: signal(false),
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
        messages: signal(Vec::new()),
        diff: signal(String::new()),
        diff_old: signal(String::new()),
        diff_new: signal(String::new()),
        pinned: signal(false),
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
    };
    let port_text = signal(String::new());
    let listen_all = signal(false);
    let paused = signal(None::<(u64, BreakSide)>);
    let paused_count = signal(0_usize);
    let paused_queue = signal(Vec::<(u64, BreakSide)>::new());
    let baseline = signal(None::<u64>);
    let compose = Compose {
        composing: signal(false),
        response: signal(false),
        method: signal(String::new()),
        url: signal(String::new()),
        headers: signal(String::new()),
        body: signal(String::new()),
        status: signal(String::new()),
    };
    let seq_diagram = signal(present::SequenceDiagramData::default());
    let seq_viewport_w = signal(0.0_f32);
    let seq_builder = SeqBuilder {
        open: signal(false),
        items: signal(Vec::new()),
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
    let list_width = signal(saved.list_width as f32);
    let columns = signal(ListColumns::from_config(&saved));
    let sort = signal(None::<ListSort>);
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
    let rules_text = signal(saved.rules);
    let upstream_text = signal(saved.upstream_proxy);
    let proxy_auth_sig = signal(config::runtime_proxy_auth(&saved.proxy_auth));
    let relay_url_sig = signal(saved.relay_url);
    let relay_accept_sig = signal(saved.relay_accept);
    let profile_signals = ProfileSignals {
        saved: saved_profiles,
        history: profile_history,
        name: signal(initial_profile_name),
        filter,
        bypass: bypass_text,
        rules: rules_text,
        upstream: upstream_text,
        rules_on: signal(saved.rules_enabled),
        breaks_on: signal(saved.breaks_enabled),
        show_tunnels: signal(saved.show_tunnel_failures),
        proxy_auth: proxy_auth_sig,
        relay_url: relay_url_sig,
        relay_accept: relay_accept_sig,
        list_width,
        panel_open: signal(false),
        machine_settings_open: signal(false),
        deselect_rev: signal(0),
    };
    let lan_url = signal(String::new());
    let lan_choice = signal(String::new());
    let lan_addrs = signal(Vec::<LanAddr>::new());
    let qr_path = signal(String::new());
    let controls = open_controls();
    let config_broken = signal(lock_config_error(&controls));
    let config_broken_dismissed = signal(false);
    if let Some(err) = config_broken.get() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let cfg_path = lock_string(&controls.config_path);
            eprintln!("zniff: 配置文件 {cfg_path} 格式错误，已使用默认配置（原文件已保留）: {err}");
        }
        set_status(&controls, StatusMsg::ConfigBroken { err });
    }
    let update_state = signal(lock_update_state(&controls));
    status.set(lock_status(&controls));
    ca_status.set(lock_ca_status(&controls));
    use_system_proxy.set(lock_bool(&controls.use_system_proxy));
    port_text.set(lock_port(&controls.listen_port).to_string());
    listen_all.set(lock_bool(&controls.listen_all));
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::spawn(updater::sweep_installer_mounts);
        if updater::auto_check_due(clock::now()) {
            auto_check_update(controls.clone());
        }
        if saved_config().capturing {
            let controls_boot = controls.clone();
            std::thread::spawn(move || start_capture(&controls_boot));
        }
    }
    #[cfg(target_arch = "wasm32")]
    start_web_poll(
        controls.clone(),
        port_text,
        listen_all,
        profile_signals,
        effective_filter,
        seq_builder,
        selected,
        baseline,
    );

    let controls_tick = controls.clone();
    let filter_tick = filter;
    let effective_filter_tick = effective_filter;
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
    let win_pending = stored((saved.window_width, saved.window_height));
    let win_since = stored(None::<SystemTime>);
    let selected_tick = selected;
    let rows_tick = rows;
    let stats_tick = filtered_stats;
    let facets_tick = facets;
    let facets_open_tick = facets_open;
    let total_tick = flow_total;
    let shown_tick = shown;
    let seen_gen = stored(0_u64);
    let seen_filter = stored(String::new());
    let seen_setup_filter = stored(String::new());
    let seen_selected = stored(None::<u64>);
    // One follow-up after the selection frame. Layout can put the previous
    // offset back once; the same flow updating does not scroll.
    let detail_scroll = element_ref();
    let detail_scroll_tick = detail_scroll;
    let detail_top_left = stored(false);
    let seen_detail_stamp = stored((
        true,
        None::<u64>,
        None::<u64>,
        None::<u64>,
        None::<u64>,
        shown_tick.pretty.get(),
        ui_lang(langs),
    ));
    let seen_listen = stored(String::new());
    let seen_lang = stored(ui_lang(langs));
    let baseline_tick = baseline;
    let sort_tick = sort;
    let seen_sort = stored(None::<ListSort>);
    let cols_tick = columns;
    let seen_full_url = stored(columns.get().full_url);
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
    let update_probe_at = stored(clock::now());
    let spill_gc_at = stored(clock::now());
    let seen_facets_open = stored(false);
    let detail_hold = stored(SystemTime::UNIX_EPOCH);
    let row_cache_tick = Rc::new(RefCell::new(RowCache {
        lang: ui_lang(langs),
        rows: HashMap::new(),
    }));
    let toast_manager = stored(ToastManager::<u64, StatusMsg>::new(ToastMotion::sonner()));
    let toast_shown = signal(Vec::<(u64, StatusMsg, ToastTransitionStatus)>::new());
    let toast_state = ToastStackState::new();
    let toast_overlay_gen = signal(0_u64);
    let toast_manager_tick = toast_manager;
    let toast_shown_tick = toast_shown;
    let toast_state_tick = toast_state;
    let toast_gen_tick = toast_overlay_gen;
    // The hex text's own box is only as wide as the current line, so measuring
    // it always says 16 bytes fit. The pane viewport is the width the card
    // actually has. Publishing it from this tick rebuilds the text; a signal
    // write inside `on_layout` can miss that text.
    let pane_px = Rc::new(Cell::new(0.0_f32));
    let pane_tick = pane_px.clone();
    let seen_rules_rev = stored(controls_tick.hub.rules_revision());
    set_interval(Duration::from_millis(16), move || {
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
                    m.push(
                        tid,
                        msg,
                        ToastOptions::timeout(Some(Duration::from_millis(2800))),
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
        if clock::elapsed(update_probe_at.get()) >= Duration::from_secs(3600) {
            update_probe_at.replace(clock::now());
            if updater::auto_check_due(clock::now()) {
                auto_check_update(controls_tick.clone());
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = update_probe_at.get();
            if let Some(err) = proxy::take_save_error() {
                set_status(&controls_tick, StatusMsg::ProxyOptionUnsaved(err));
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
            status.set(status_now);
        }
        let running_now = *controls_tick
            .running
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if running.get_untracked() != running_now {
            running.set(running_now);
        }
        let listen_now = lock_string(&controls_tick.listen);
        if listen.with(|cur| cur != &listen_now) {
            listen.set(listen_now.clone());
        }
        let ca_now = lock_ca_status(&controls_tick);
        if ca_status.get_untracked() != ca_now {
            ca_status.set(ca_now);
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
        if listen_now != seen_listen.get() {
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
        let win_size = window();
        if win_size.width.0.is_finite()
            && win_size.height.0.is_finite()
            && win_size.width.0 > 0.0
            && win_size.height.0 > 0.0
        {
            let win_now = (
                config::clamp_window_width(win_size.width.0.round() as u32),
                config::clamp_window_height(win_size.height.0.round() as u32),
            );
            if win_now != win_pending.get() {
                win_pending.replace(win_now);
                win_since.replace(Some(clock::now()));
            } else if win_since
                .get()
                .is_some_and(|at| clock::elapsed(at) >= Duration::from_millis(500))
            {
                remember_window_size(&controls_tick, win_now.0, win_now.1);
                win_since.replace(None);
            }
        }
        release_sequence_data(seq_builder);
        let spill_expired = if clock::elapsed(spill_gc_at.get()) >= Duration::from_secs(15) {
            let now = clock::now();
            spill_gc_at.replace(now);
            controls_tick
                .hub
                .with_session(|session| session.gc_spill(now))
        } else {
            false
        };
        let selected_now = selected_tick.get();
        let lang_now = ui_lang(langs);
        let baseline_now = baseline_tick.get();
        let pretty_now = shown_tick.pretty.get();
        let sort_now = sort_tick.get();
        let full_url_now = cols_tick.get().full_url;
        let facets_open_now = facets_open_tick.get();
        let facets_opened = facets_open_now && !seen_facets_open.get();
        seen_facets_open.replace(facets_open_now);
        let list_changed = generation != seen_gen.get()
            || effective_filter_now != seen_filter.get()
            || filter_now != seen_setup_filter.get()
            || lang_now != seen_lang.get()
            || sort_now != seen_sort.get()
            || full_url_now != seen_full_url.get()
            || spill_expired
            || facets_opened;
        let want_stats = selected_now.is_none();
        let stats_only = !list_changed && want_stats && seen_selected.get().is_some();
        if list_changed {
            seen_gen.replace(generation);
            seen_filter.replace(effective_filter_now.clone());
            seen_setup_filter.replace(filter_now.clone());
            seen_lang.replace(lang_now);
            seen_sort.replace(sort_now);
            seen_full_url.replace(full_url_now);
            let (matched, _) = controls_tick.hub.matching_ids(&effective_filter_now);
            let facet_ids = if facets_open_now && filter_now != effective_filter_now {
                Some(controls_tick.hub.matching_ids(&filter_now).0)
            } else {
                None
            };
            let (lines, stats, next_facets, next_total, next_mem, next_dropped) = {
                let mut cache = row_cache_tick.borrow_mut();
                controls_tick.hub.with_session(|session| {
                    let lines = project_rows(
                        session,
                        &matched,
                        lang_now,
                        sort_now,
                        full_url_now,
                        &mut cache,
                    );
                    let stats = want_stats.then(|| {
                        session::FilteredStats::from_flows(
                            matched.iter().filter_map(|(id, _)| session.get(*id)),
                        )
                    });
                    let next_facets = facets_open_now.then(|| {
                        let ids = facet_ids.as_ref().unwrap_or(&matched);
                        session::SessionFacets::from_flows(
                            ids.iter().filter_map(|(id, _)| session.get(*id)),
                        )
                    });
                    (
                        lines,
                        stats,
                        next_facets,
                        session.len(),
                        format_bytes(session.memory_bytes()),
                        session.dropped(),
                    )
                })
            };
            if total_tick.get_untracked() != next_total {
                total_tick.set(next_total);
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
            let (matched, _) = controls_tick.hub.matching_ids(&effective_filter_now);
            let stats = controls_tick.hub.with_session(|session| {
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
            shown_tick.editing_comment.set(false);
            shown_tick.repeating.set(false);
            shown_tick.msg_search.set(String::new());
            shown_tick.msg_dir.set(0);
            shown_tick.msg_kind.set(0);
            detail_scroll_tick.scroll_to_offset(px(0.0), false);
            detail_top_left.replace(true);
        } else if detail_top_left.get() {
            detail_scroll_tick.scroll_to_offset(px(0.0), false);
            detail_top_left.replace(false);
        }
        let (selected_rev, selected_done, baseline_rev) =
            if selected_now.is_some() || baseline_now.is_some() {
                controls_tick.hub.with_session(|session| {
                    let selected_flow = selected_now.and_then(|id| session.get(id));
                    (
                        selected_flow.map(|flow| flow.rev),
                        selected_flow
                            .map(|flow| flow.finished_at.is_some())
                            .unwrap_or(true),
                        baseline_now.and_then(|id| session.get(id).map(|flow| flow.rev)),
                    )
                })
            } else {
                (None, true, None)
            };
        let detail_stamp = (
            false,
            selected_now,
            selected_rev,
            baseline_now,
            baseline_rev,
            pretty_now,
            lang_now,
        );
        if detail_stamp != seen_detail_stamp.get() || spill_expired {
            let prev = seen_detail_stamp.get();
            let view_changed = prev.1 != selected_now
                || prev.3 != baseline_now
                || prev.5 != pretty_now
                || prev.6 != lang_now;
            let streaming = selected_now.is_some() && !selected_done;
            let due = clock::elapsed(detail_hold.get()) >= Duration::from_millis(200);
            if !streaming || view_changed || spill_expired || due {
                seen_detail_stamp.replace(detail_stamp);
                detail_hold.replace(clock::now());
                if prev.1 != selected_now || prev.5 != pretty_now {
                    reset_body_offsets(shown_tick);
                }
                publish_detail(
                    &shown_tick,
                    &controls_tick.hub,
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
    let folded = signal(0_u8);
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
    let compose_submit_key = compose;
    let selected_submit_key = selected;
    let paused_submit_key = paused;
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
            }
        },
    );
    let selected_esc = selected;
    let compose_esc = compose;
    let seq_esc = seq_builder;
    let shown_esc = shown;
    on_key("escape", move |_| {
        if profile_signals.machine_settings_open.get() {
            profile_signals.machine_settings_open.set(false);
        } else if compose_esc.composing.get() {
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
            ca_status,
            use_system_proxy,
            move || install_ca(controls_ca.clone()),
            move || remove_ca(controls_remove.clone()),
            move || toggle_system_proxy(controls_proxy.clone()),
            move || {
                clear_captured(
                    &controls_clear,
                    selected_clear,
                    baseline_clear,
                    compose_clear,
                    seq_builder,
                    shown,
                );
            },
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
        .child(
            view()
                .flex_1()
                .flex_row()
                .child(flow_list(
                    rows,
                    facets,
                    facets_open,
                    selected,
                    paused,
                    compose,
                    filter,
                    list_filter,
                    effective_filter,
                    rules_text,
                    list_width,
                    columns,
                    sort,
                    controls_list,
                    shown,
                    baseline,
                    filter_focus,
                ))
                .child(column_splitter(list_width, controls_split))
                .child(detail_pane(
                    shown,
                    controls_detail,
                    selected,
                    paused,
                    compose,
                    seq_builder,
                    rows,
                    list_filter,
                    effective_filter,
                    rules_text,
                    baseline,
                    folded,
                    pane_px,
                    filtered_stats,
                    detail_scroll,
                )),
        )
        .child(footer(
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
            config_broken,
            config_broken_dismissed,
            controls.clone(),
        ))
        .child(machine_settings_modal(profile_signals, controls))
        .child(each(
            move || {
                if toast_shown.with(Vec::is_empty) {
                    Vec::new()
                } else {
                    vec![toast_overlay_gen.get()]
                }
            },
            |id| *id,
            move |_, _| {
                overlay()
                    .inset_0()
                    .flex_col()
                    .items_center()
                    .pt(px(54.0))
                    .pointer_events(PointerEvents::None)
                    .child(
                        toast_stack()
                            .placement(Anchor::TopCenter)
                            .state(toast_state)
                            .w(move || px((window().width.0 - 24.0).clamp(220.0, 380.0)))
                            .max_w(move || px((window().width.0 - 24.0).max(200.0)))
                            .pointer_events(PointerEvents::Auto)
                            .child(each(
                                toast_shown,
                                |(id, _, _)| *id,
                                move |item, _| {
                                    let id = item.with(|(id, _, _)| *id);
                                    let dismiss_manager = toast_manager;
                                    let dismiss_shown = toast_shown;
                                    let dismiss_one = move || {
                                        let now = nsh_core::time::now();
                                        let next = dismiss_manager.with(|m| {
                                            m.dismiss(&id, now);
                                            m.visible(3)
                                                .map(|(id, data, status)| {
                                                    (*id, data.clone(), status)
                                                })
                                                .collect::<Vec<_>>()
                                        });
                                        dismiss_shown.set(next);
                                    };
                                    toast()
                                        .transition_status(move || item.with(|(_, _, s)| *s))
                                        .on_dismiss_request(dismiss_one)
                                        .rounded(px(10.0))
                                        .border_1()
                                        .border_color(border_strong())
                                        .bg(bg_card())
                                        .shadow_lg()
                                        .transition_all()
                                        .duration(180.0)
                                        .starting_class("opacity-0 -translate-y-2")
                                        .ending_class("opacity-0 scale-95")
                                        .child(
                                            view()
                                                .w_full()
                                                .px_3p5()
                                                .py_3()
                                                .flex_row()
                                                .items_center()
                                                .gap_3()
                                                .cursor(Cursor::Pointer)
                                                .on_press(dismiss_one)
                                                .child(
                                                    view()
                                                        .w(px(32.0))
                                                        .h(px(32.0))
                                                        .rounded(px(8.0))
                                                        .flex_shrink_0()
                                                        .flex_row()
                                                        .items_center()
                                                        .justify_center()
                                                        .bg(move || {
                                                            if item.with(|(_, msg, _)| {
                                                                msg.toast_kind() == Some(true)
                                                            }) {
                                                                status_warn_bg()
                                                            } else {
                                                                status_ok_bg()
                                                            }
                                                        })
                                                        .child(glyph_px(
                                                            16.0,
                                                            move || {
                                                                if item.with(|(_, msg, _)| {
                                                                    msg.toast_kind() == Some(true)
                                                                }) {
                                                                    Glyph::Alert
                                                                } else {
                                                                    Glyph::Check
                                                                }
                                                            },
                                                            move || {
                                                                if item.with(|(_, msg, _)| {
                                                                    msg.toast_kind() == Some(true)
                                                                }) {
                                                                    color(status_warn_fg())
                                                                } else {
                                                                    color(status_ok_fg())
                                                                }
                                                            },
                                                        )),
                                                )
                                                .child(
                                                    view()
                                                        .flex_1()
                                                        .min_w(px(0.0))
                                                        .flex_col()
                                                        .gap(px(2.0))
                                                        .child(
                                                            text(move || {
                                                                let lang = ui_lang(ui_langs());
                                                                item.with(|(_, msg, _)| {
                                                                    msg.toast_title(lang)
                                                                        .to_string()
                                                                })
                                                            })
                                                            .text_size(px(13.0))
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_color(fg_primary()),
                                                        )
                                                        .child(
                                                            text(move || {
                                                                let lang = ui_lang(ui_langs());
                                                                item.with(|(_, msg, _)| {
                                                                    msg.text(lang)
                                                                })
                                                            })
                                                            .number_of_lines(3)
                                                            .text_overflow(EllipsizeMode::Middle)
                                                            .text_size(px(12.0))
                                                            .text_color(fg_secondary()),
                                                        ),
                                                )
                                                .child(
                                                    view()
                                                        .w(px(20.0))
                                                        .h(px(20.0))
                                                        .rounded(px(5.0))
                                                        .flex_shrink_0()
                                                        .flex_row()
                                                        .items_center()
                                                        .justify_center()
                                                        .hover(|s| s.bg(bg_control_hover()))
                                                        .active(|s| s.opacity(0.75))
                                                        .label(tr("关闭", "Dismiss"))
                                                        .child(glyph_px(
                                                            10.5,
                                                            || Glyph::Close,
                                                            || color(fg_muted()),
                                                        )),
                                                ),
                                        )
                                },
                            )),
                    )
            },
        ))
}
