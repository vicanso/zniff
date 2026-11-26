//! Process-lifetime root runtime, tray close/reopen window lifecycle, and background maintenance.

#[allow(unused_imports)]
use super::*;

#[derive(Clone)]
pub(crate) struct AppRuntime {
    pub(crate) controls: Arc<Controls>,
    pub(crate) langs: Signal<Option<Lang>>,
    pub(crate) show_tray: Signal<bool>,
    pub(crate) minimize_to_tray: Signal<bool>,
    pub(crate) main_open: Signal<bool>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) main_win: Signal<Option<nsh_core::element::desktop::WindowRef>>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) last_placement: Stored<Option<nsh_core::desktop::WindowPlacement>>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) has_mounted_once: Stored<bool>,
    pub(crate) running: Signal<bool>,
    pub(crate) listen: Signal<String>,
    pub(crate) use_system_proxy: Signal<bool>,
    pub(crate) about_open: Signal<bool>,
    pub(crate) settings_open: Signal<bool>,
    pub(crate) watch_rules: Signal<Vec<config::WatchRule>>,
    pub(crate) watch_eval: Signal<crate::watch::WatchEvaluation>,
    pub(crate) watch_tracker: Stored<crate::watch::WatchTracker>,
    pub(crate) seen_watch_rules: Stored<Vec<config::WatchRule>>,
    pub(crate) seen_watch_gen: Stored<u64>,
    pub(crate) spill_gc_at: Stored<SystemTime>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) update_probe_at: Stored<SystemTime>,
    pub(crate) tray_clear_rev: Signal<u64>,
}

/// Initializes process-lifetime contexts, `Controls`/`Hub`, startup tasks, and root tray signals once.
pub(crate) fn init_app_runtime(initial_cfg: &config::Config) -> AppRuntime {
    let langs = signal(load_lang_pref());
    provide_context(langs);
    hydrate_web_local_prefs();
    let langs_after_hydrate = load_lang_pref();
    if langs.get_untracked() != langs_after_hydrate {
        langs.set(langs_after_hydrate);
    }
    let ui_cfg = saved_config();
    provide_appearance_prefs(ui_cfg.theme_mode, ui_cfg.ui_zoom);
    provide_font_prefs(
        ui_cfg.font_preset,
        ui_cfg.ui_font.clone(),
        ui_cfg.mono_font.clone(),
        ui_cfg.system_mono_font,
    );

    let controls = open_controls();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let exit_controls = controls.clone();
        cleanup(move || {
            #[cfg(not(test))]
            {
                shutdown_capture_for_exit(&exit_controls);
                crate::cleanup_desktop_runtime();
            }
            #[cfg(test)]
            let _ = &exit_controls;
        });
    }

    if let Some(err) = lock_config_error(&controls) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let cfg_path = lock_string(&controls.config_path);
            eprintln!("zniff: 配置文件 {cfg_path} 格式错误，已使用默认配置（原文件已保留）: {err}");
        }
        set_status(&controls, StatusMsg::ConfigBroken { err });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::spawn(updater::sweep_installer_mounts);
        if updater::auto_check_due(clock::now()) {
            auto_check_update(controls.clone());
        }
        if initial_cfg.capturing {
            let controls_boot = controls.clone();
            crate::startup_metrics::mark_auto_capture_start();
            std::thread::spawn(move || {
                start_capture(&controls_boot);
                crate::startup_metrics::mark_auto_capture_finish(lock_bool(&controls_boot.running));
            });
        }
    }

    let running = signal(lock_bool(&controls.running));
    let listen = signal(lock_string(&controls.listen));
    let use_system_proxy = signal(lock_bool(&controls.use_system_proxy));
    let now = clock::now();

    AppRuntime {
        controls,
        langs,
        show_tray: signal(initial_cfg.show_tray),
        minimize_to_tray: signal(initial_cfg.minimize_to_tray),
        main_open: signal(true),
        #[cfg(not(target_arch = "wasm32"))]
        main_win: signal(None),
        #[cfg(not(target_arch = "wasm32"))]
        last_placement: stored(None),
        #[cfg(not(target_arch = "wasm32"))]
        has_mounted_once: stored(false),
        running,
        listen,
        use_system_proxy,
        about_open: signal(false),
        settings_open: signal(false),
        watch_rules: signal(ui_cfg.watch_rules),
        watch_eval: signal(crate::watch::WatchEvaluation::default()),
        watch_tracker: stored(crate::watch::WatchTracker::default()),
        seen_watch_rules: stored(Vec::new()),
        seen_watch_gen: stored(u64::MAX),
        spill_gc_at: stored(now),
        #[cfg(not(target_arch = "wasm32"))]
        update_probe_at: stored(now),
        tray_clear_rev: signal(0),
    }
}

/// Shows or recreates the main window when triggered from the system tray, Dock reopen, or menu bar.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn reveal_main_window(
    main_win: Signal<Option<nsh_core::element::desktop::WindowRef>>,
    main_open: Signal<bool>,
) {
    if !main_open.get_untracked() {
        main_open.set(true);
    } else if let Some(win) = main_win.get_untracked() {
        win.show();
        win.focus();
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn capture_placement_snapshot(
    state: Option<nsh_core::desktop::WindowState>,
) -> Option<(nsh_core::desktop::WindowPlacement, WindowPlacementSnapshot)> {
    let st = state?;
    let b = st.placement.bounds;
    if !(b.size.width.0.is_finite()
        && b.size.height.0.is_finite()
        && b.size.width.0 > 0.0
        && b.size.height.0 > 0.0
        && b.origin.x.0.is_finite()
        && b.origin.y.0.is_finite())
    {
        return None;
    }
    let mode = match st.placement.mode {
        nsh_core::desktop::WindowMode::Maximized => "maximized".to_string(),
        nsh_core::desktop::WindowMode::FullScreen => "fullscreen".to_string(),
        nsh_core::desktop::WindowMode::Normal => String::new(),
    };
    let snapshot = WindowPlacementSnapshot {
        width: config::clamp_window_width(b.size.width.0.round() as u32),
        height: config::clamp_window_height(b.size.height.0.round() as u32),
        x: Some(b.origin.x.0.round() as i32),
        y: Some(b.origin.y.0.round() as i32),
        display_id: st.display.map(|id| config::format_window_display_id(id.0)),
        mode,
    };
    Some((st.placement, snapshot))
}

#[cfg(not(target_arch = "wasm32"))]
fn schedule_tray_memory_trim() {
    crate::session::trim_memory();
    set_timeout(Duration::from_millis(50), || {
        crate::session::trim_memory();
    });
}

/// Closes and unmounts the main window to minimize to tray while keeping background capture & tray alive.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn close_main_window_to_tray(rt: &AppRuntime) {
    let win_state = rt
        .main_win
        .get_untracked()
        .and_then(|win| win.state())
        .or_else(nsh_core::element::desktop::window_state);
    if let Some((placement, snapshot)) = capture_placement_snapshot(win_state) {
        rt.last_placement.replace(Some(placement));
        remember_window_placement(&rt.controls, &snapshot);
    }
    rt.settings_open.set(false);
    rt.about_open.set(false);
    rt.main_win.set(None);
    rt.main_open.set(false);
    schedule_tray_memory_trim();
}

/// Handles `desktop::window().on_open_change` on the root main window.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn on_main_window_open_change(rt: &AppRuntime, open: bool) {
    if !open {
        rt.settings_open.set(false);
        rt.about_open.set(false);
        rt.main_win.set(None);
        schedule_tray_memory_trim();
    }
    rt.main_open.set(open);
}

/// Evaluates watch rules against the live session without dropping or duplicating alerts across close/reopen.
pub(crate) fn run_watch_evaluation(rt: &AppRuntime, generation: u64) {
    let watch_rules_now = rt.watch_rules.get_untracked();
    if generation == rt.seen_watch_gen.get()
        && rt.seen_watch_rules.with(|prev| prev == &watch_rules_now)
    {
        return;
    }
    rt.seen_watch_gen.replace(generation);
    rt.seen_watch_rules.replace(watch_rules_now.clone());
    let eval = rt.watch_tracker.with(|tracker| {
        rt.controls
            .hub
            .with_session(|session| tracker.evaluate(&watch_rules_now, session))
    });
    for alert in &eval.new_alerts {
        set_status(
            &rt.controls,
            StatusMsg::WatchAlert {
                rule: alert.rule_name.clone(),
                target: alert.target.clone(),
                count: alert.new_count,
            },
        );
    }
    rt.watch_eval.set(eval);
}

/// Lightweight maintenance pass run only while the main window is closed to tray.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn run_background_maintenance(rt: &AppRuntime) {
    if rt.main_open.get_untracked() {
        return;
    }
    rt.controls.hub.expire_clear_undo();
    let running_now = lock_bool(&rt.controls.running);
    if rt.running.get_untracked() != running_now {
        rt.running.set(running_now);
    }
    let listen_now = lock_string(&rt.controls.listen);
    if rt.listen.with_untracked(|cur| cur != &listen_now) {
        rt.listen.set(listen_now);
    }
    let sys_proxy_now = lock_bool(&rt.controls.use_system_proxy);
    if rt.use_system_proxy.get_untracked() != sys_proxy_now {
        rt.use_system_proxy.set(sys_proxy_now);
    }

    let now = clock::now();
    if clock::elapsed(rt.spill_gc_at.get()) >= Duration::from_secs(15) {
        rt.spill_gc_at.replace(now);
        rt.controls
            .hub
            .with_session(|session| session.gc_spill(now));
    }
    if !rt.watch_rules.with_untracked(Vec::is_empty) {
        let gen_now = rt.controls.hub.generation();
        run_watch_evaluation(rt, gen_now);
    }
    if clock::elapsed(rt.update_probe_at.get()) >= Duration::from_secs(3600) {
        rt.update_probe_at.replace(now);
        if updater::auto_check_due(now) {
            auto_check_update(rt.controls.clone());
        }
    }
}

/// Installs the low-frequency background timer owned by `App()` so tray state, spill GC, and watch alerts
/// keep running with minimal CPU/memory while the main window is closed.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn install_background_maintenance(rt: &AppRuntime) {
    let rt_tick = rt.clone();
    set_interval(Duration::from_millis(500), move || {
        run_background_maintenance(&rt_tick);
    });
}
