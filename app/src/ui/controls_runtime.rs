#[allow(unused_imports)]
use super::*;

#[cfg(target_arch = "wasm32")]
pub(crate) fn open_controls() -> Arc<Controls> {
    let hub = Hub::new(Arc::new(Ca::unavailable())).expect("hub");
    Arc::new(Controls {
        hub: Arc::new(hub),
        capture: Mutex::new(None),
        status: Mutex::new(StatusMsg::Idle),
        running: Mutex::new(false),
        listen: Mutex::new(String::new()),
        listen_port: Mutex::new(load_listen_port()),
        ca_status: Mutex::new(ca::CaStatus::NotInstalled),
        active_external_ca: Mutex::new(None),
        external_cas: Mutex::new(Vec::new()),
        use_system_proxy: Mutex::new(false),
        proxy_applied: Mutex::new(false),
        ops: Mutex::new(()),
        listen_all: Mutex::new(false),
        update_state: Mutex::new(UpdateState::default()),
        redact_export: Mutex::new(false),
        redact_strict: Mutex::new(false),
        redact_fields: Mutex::new(String::new()),
        relay_url: Mutex::new(String::new()),
        relay_token: Mutex::new(String::new()),
        toast_seq: std::sync::atomic::AtomicU64::new(0),
        toast_queue: Mutex::new(Vec::new()),
        config_error: Mutex::new(None),
        config_path: Mutex::new(String::new()),
        pending_replay_diff: Mutex::new(None),
        session_tabs: Mutex::new(SessionTabsStore::new()),
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn open_controls() -> Arc<Controls> {
    let saved = saved_config();
    let cfg_err = config::last_load_error_for(&ca::data_dir(), config::is_dev_mode());
    let cfg_path = config::config_path_for(&ca::data_dir(), config::is_dev_mode())
        .display()
        .to_string();
    match Ca::open_default().and_then(|ca| Hub::new(Arc::new(ca))) {
        Ok(hub) => {
            apply_saved_prefs(&hub);
            let mut fallback_tool = None;
            if !saved.external_ca.is_empty() {
                let res = external_ca::load_trusted_candidate(&saved.external_ca)
                    .and_then(|ext| hub.ca.switch_to_external(ext));
                if let Err(err) = res {
                    let tool = external_ca::ExternalCaKind::from_id(&saved.external_ca)
                        .map(|k| k.tool_name().to_string())
                        .unwrap_or_else(|| saved.external_ca.clone());
                    eprintln!("zniff: 启动时无法复用 {tool} 根证书，已自动切回内置证书: {err:#}");
                    let _ = save_config(|cfg| cfg.external_ca.clear());
                    fallback_tool = Some(tool);
                }
            }
            let ca_status = hub.ca.trust_status();
            let active_ext = hub.ca.external_source();
            let controls = Arc::new(Controls {
                hub: Arc::new(hub),
                capture: Mutex::new(None),
                status: Mutex::new(StatusMsg::Idle),
                running: Mutex::new(false),
                listen: Mutex::new(String::new()),
                listen_port: Mutex::new(load_listen_port()),
                ca_status: Mutex::new(ca_status),
                active_external_ca: Mutex::new(active_ext),
                external_cas: Mutex::new(Vec::new()),
                use_system_proxy: Mutex::new(load_use_system_proxy()),
                proxy_applied: Mutex::new(false),
                ops: Mutex::new(()),
                listen_all: Mutex::new(load_listen_all()),
                update_state: Mutex::new(UpdateState::from_disk()),
                redact_export: Mutex::new(saved.redact_export),
                redact_strict: Mutex::new(saved.redact_strict),
                redact_fields: Mutex::new(saved.redact_fields.clone()),
                relay_url: Mutex::new(saved.relay_url.clone()),
                relay_token: Mutex::new(saved.relay_token.clone()),
                relay_push: Mutex::new(None),
                toast_seq: std::sync::atomic::AtomicU64::new(0),
                toast_queue: Mutex::new(Vec::new()),
                config_error: Mutex::new(cfg_err),
                config_path: Mutex::new(cfg_path),
                pending_replay_diff: Mutex::new(None),
                session_tabs: Mutex::new(SessionTabsStore::new()),
            });
            if let Some(tool) = fallback_tool {
                set_status(&controls, StatusMsg::ExternalCaFallback { tool });
            }
            refresh_external_cas(controls.clone());
            controls
        }
        Err(err) => {
            // A hub is required for the view. If the CA cannot be created,
            // build one in a temp directory so the window can still explain
            // the failure.
            let dir = std::env::temp_dir().join("zniff-fallback");
            let ca = Arc::new(Ca::open(&dir).expect("临时证书目录"));
            let hub = Hub::new(ca).expect("hub");
            apply_saved_prefs(&hub);
            Arc::new(Controls {
                hub: Arc::new(hub),
                capture: Mutex::new(None),
                status: Mutex::new(StatusMsg::CaUnavailable(format!("{err:#}"))),
                running: Mutex::new(false),
                listen: Mutex::new(String::new()),
                listen_port: Mutex::new(load_listen_port()),
                ca_status: Mutex::new(ca::CaStatus::NotInstalled),
                active_external_ca: Mutex::new(None),
                external_cas: Mutex::new(Vec::new()),
                use_system_proxy: Mutex::new(load_use_system_proxy()),
                proxy_applied: Mutex::new(false),
                ops: Mutex::new(()),
                listen_all: Mutex::new(load_listen_all()),
                update_state: Mutex::new(UpdateState::from_disk()),
                redact_export: Mutex::new(saved.redact_export),
                redact_strict: Mutex::new(saved.redact_strict),
                redact_fields: Mutex::new(saved.redact_fields),
                relay_url: Mutex::new(saved.relay_url),
                relay_token: Mutex::new(saved.relay_token),
                relay_push: Mutex::new(None),
                toast_seq: std::sync::atomic::AtomicU64::new(0),
                toast_queue: Mutex::new(Vec::new()),
                config_error: Mutex::new(cfg_err),
                config_path: Mutex::new(cfg_path),
                pending_replay_diff: Mutex::new(None),
                session_tabs: Mutex::new(SessionTabsStore::new()),
            })
        }
    }
}

pub(crate) fn lock_config_error(controls: &Controls) -> Option<String> {
    controls
        .config_error
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn current_config_error(controls: &Controls) -> Option<String> {
    let err = config::last_load_error_for(&ca::data_dir(), config::is_dev_mode());
    *controls
        .config_error
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = err.clone();
    err
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn current_config_error(controls: &Controls) -> Option<String> {
    lock_config_error(controls)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn redact_options(controls: &Controls) -> redact::RedactOptions {
    let enabled = lock_bool(&controls.redact_export);
    let strict = lock_bool(&controls.redact_strict);
    let extra = controls
        .redact_fields
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    redact::RedactOptions::new(enabled, &extra).with_strict(strict)
}

pub(crate) fn lock_update_state(controls: &Controls) -> UpdateState {
    controls
        .update_state
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

pub(crate) fn with_update_state(controls: &Controls, f: impl FnOnce(&mut UpdateState)) {
    let mut guard = controls
        .update_state
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    f(&mut guard);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn auto_check_update(controls: Arc<Controls>) {
    if lock_update_state(&controls).is_busy() {
        return;
    }
    std::thread::spawn(move || {
        let outcome = updater::check_with_skipped();
        updater::record_outcome(clock::now(), &outcome);
        with_update_state(&controls, |st| {
            st.nudge = updater::nudge();
            st.ignored = updater::ignored();
            st.notes = updater::nudge_notes();
        });
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn check_update_manual(controls: Arc<Controls>) {
    {
        let mut st = controls
            .update_state
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if st.is_busy() {
            return;
        }
        st.phase = UpdatePhase::Checking;
        st.last_check = None;
    }
    set_status(&controls, StatusMsg::UpdateChecking);
    std::thread::spawn(move || {
        let outcome = updater::check_with_skipped();
        updater::record_outcome(clock::now(), &outcome);
        let check_notice = match &outcome {
            updater::UpdateCheck::UpToDate => {
                let ver = updater::version().to_string();
                set_status(&controls, StatusMsg::UpdateUpToDate(ver.clone()));
                UpdateCheckNotice::UpToDate(ver)
            }
            updater::UpdateCheck::Newer { version, .. } => {
                updater::unignore();
                set_status(&controls, StatusMsg::UpdateAvailable(version.clone()));
                UpdateCheckNotice::Available(version.clone())
            }
            updater::UpdateCheck::Failed(err) => {
                set_status(&controls, StatusMsg::UpdateCheckFailed(err.clone()));
                UpdateCheckNotice::Failed(err.clone())
            }
        };
        with_update_state(&controls, |st| {
            st.phase = UpdatePhase::Idle;
            st.nudge = updater::nudge();
            st.ignored = updater::ignored();
            st.notes = updater::nudge_notes();
            st.last_check = Some(check_notice);
        });
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn skip_update(controls: &Controls, version: &str) {
    updater::ignore(version);
    with_update_state(controls, |st| {
        st.nudge = updater::nudge();
        st.ignored = updater::ignored();
        st.notes = updater::nudge_notes();
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn unskip_update(controls: &Controls) {
    updater::unignore();
    with_update_state(controls, |st| {
        st.nudge = updater::nudge();
        st.ignored = updater::ignored();
        st.notes = updater::nudge_notes();
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn start_upgrade(controls: Arc<Controls>, version: String) {
    {
        let mut st = controls
            .update_state
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if st.is_busy() {
            return;
        }
        st.phase = UpdatePhase::Downloading {
            version: version.clone(),
            received: 0,
            total: 0,
        };
    }
    set_status(
        &controls,
        StatusMsg::UpdateDownloading {
            version: version.clone(),
            progress: "0 B".into(),
        },
    );
    std::thread::spawn(move || {
        let controls_progress = controls.clone();
        let ver_progress = version.clone();
        let mut last_step = u64::MAX;
        let downloaded = updater::download(&version, move |received, total| {
            let step = received / (256 * 1024);
            if step != last_step || (total > 0 && received == total) {
                last_step = step;
                with_update_state(&controls_progress, |st| {
                    st.phase = UpdatePhase::Downloading {
                        version: ver_progress.clone(),
                        received,
                        total,
                    };
                });
                let progress = if total > 0 {
                    format!(
                        "{} / {}",
                        format_bytes(received as usize),
                        format_bytes(total as usize)
                    )
                } else {
                    format_bytes(received as usize)
                };
                set_status(
                    &controls_progress,
                    StatusMsg::UpdateDownloading {
                        version: ver_progress.clone(),
                        progress,
                    },
                );
            }
        });
        let path = match downloaded {
            Ok(path) => path,
            Err(err) => {
                with_update_state(&controls, |st| {
                    st.phase = UpdatePhase::Idle;
                });
                set_status(&controls, StatusMsg::UpdateFailed { version, err });
                return;
            }
        };
        with_update_state(&controls, |st| {
            st.phase = UpdatePhase::Installing {
                version: version.clone(),
            };
        });
        set_status(&controls, StatusMsg::UpdateInstalling(version.clone()));
        match updater::install(&path) {
            Ok(updater::Delivery::Replaced) => {
                with_update_state(&controls, |st| {
                    st.phase = UpdatePhase::ReadyToRestart {
                        version: version.clone(),
                    };
                });
                set_status(&controls, StatusMsg::UpdateRestarting(version));
                relaunch_and_exit(&controls);
            }
            Ok(updater::Delivery::OpenedForDrag) => {
                with_update_state(&controls, |st| {
                    st.phase = UpdatePhase::OpenedInstaller {
                        version: version.clone(),
                    };
                });
                set_status(&controls, StatusMsg::UpdateOpenedInstaller(version));
            }
            Err(err) => {
                with_update_state(&controls, |st| {
                    st.phase = UpdatePhase::Idle;
                });
                set_status(&controls, StatusMsg::UpdateFailed { version, err });
            }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn shutdown_capture_for_exit(controls: &Controls) {
    let _ops = lock_ops(controls);
    if let Some(capture) = controls
        .capture
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
    {
        capture.stop();
    }
    if let Some(handle) = controls
        .relay_push
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
    {
        handle.stop_and_wait(Duration::from_secs(3));
    }
    relay::shutdown_relay_push();
    controls.hub.end_share();
    ca::clear_ios_simulator_proxies();
    launcher::clear_android_adb_proxies();
    transparent_proxy::cleanup_on_exit();
    if lock_bool(&controls.proxy_applied) {
        set_bool(&controls.proxy_applied, false);
        let _ = sysproxy::disable();
    } else {
        let _ = sysproxy::restore_if_dirty();
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn relaunch_and_exit(controls: &Controls) {
    shutdown_capture_for_exit(controls);
    updater::relaunch();
    std::process::exit(0);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn quit_for_manual_install(controls: &Controls) {
    shutdown_capture_for_exit(controls);
    std::process::exit(0);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn install_ca(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let ca = controls.hub.ca.clone();
        if ca.is_external() {
            if let Err(err) = ca.switch_to_builtin() {
                set_status(&controls, StatusMsg::InstallFailed(format!("{err:#}")));
                return;
            }
            set_active_external_ca(&controls, None);
            let _ = save_config(|cfg| cfg.external_ca.clear());
        }
        match ca::install(&ca) {
            Ok(_) => {
                let status = ca::trust_status(&ca);
                set_ca_status(&controls, status);
                if status == ca::CaStatus::Trusted {
                    set_status(&controls, StatusMsg::CaInstalled);
                }
            }
            Err(err) => {
                set_ca_status(&controls, ca::trust_status(&ca));
                set_status(&controls, StatusMsg::InstallFailed(format!("{err:#}")));
            }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn refresh_external_cas(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let found = external_ca::detect_trusted_external_cas();
        *controls
            .external_cas
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = found;
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn use_external_ca(controls: Arc<Controls>, id: String) {
    std::thread::spawn(move || {
        let tool = external_ca::ExternalCaKind::from_id(&id)
            .map(|k| k.tool_name().to_string())
            .unwrap_or_else(|| id.clone());
        let ca = controls.hub.ca.clone();
        match external_ca::load_trusted_candidate(&id)
            .and_then(|loaded| ca.switch_to_external(loaded))
        {
            Ok(()) => {
                let source = ca.external_source();
                let next_tool = source
                    .as_ref()
                    .map(|item| item.tool_name.clone())
                    .unwrap_or(tool);
                set_active_external_ca(&controls, source);
                set_ca_status(&controls, ca::trust_status(&ca));
                let _ = save_config(|cfg| cfg.external_ca = id);
                set_status(&controls, StatusMsg::ExternalCaEnabled { tool: next_tool });
            }
            Err(err) => {
                set_status(
                    &controls,
                    StatusMsg::ExternalCaSwitchFailed {
                        tool,
                        err: format!("{err:#}"),
                    },
                );
                refresh_external_cas(controls);
            }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn use_builtin_ca(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let ca = controls.hub.ca.clone();
        match ca.switch_to_builtin() {
            Ok(()) => {
                set_active_external_ca(&controls, None);
                set_ca_status(&controls, ca::trust_status(&ca));
                let _ = save_config(|cfg| cfg.external_ca.clear());
                set_status(&controls, StatusMsg::ExternalCaRestored);
                refresh_external_cas(controls);
            }
            Err(err) => {
                set_status(&controls, StatusMsg::CaUnavailable(format!("{err:#}")));
            }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn remove_ca(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let ca = controls.hub.ca.clone();
        match ca::remove(&ca) {
            Ok(_) => {
                set_ca_status(&controls, ca::trust_status(&ca));
                set_status(&controls, StatusMsg::CaRemoved);
            }
            Err(err) => {
                set_ca_status(&controls, ca::trust_status(&ca));
                set_status(&controls, StatusMsg::RemoveFailed(format!("{err:#}")));
            }
        }
    });
}

#[cfg(target_os = "macos")]
pub(crate) fn platform_ca_extra_menu_item(controls: &Arc<Controls>) -> impl IntoElement {
    let controls = controls.clone();
    menu_action(
        Glyph::Badge,
        tr("安装证书到 iOS 模拟器", "Install on iOS Simulator"),
        move || install_ca_ios_sim(controls.clone()),
    )
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn platform_ca_extra_menu_item(controls: &Arc<Controls>) -> impl IntoElement {
    let controls = controls.clone();
    menu_action(
        Glyph::Prompt,
        tr("复制系统证书命令", "Copy system CA command"),
        move || copy_linux_ca_cmd(&controls),
    )
}

#[cfg(not(unix))]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn platform_ca_extra_menu_item(_controls: &Arc<Controls>) -> impl IntoElement {
    view().w(px(0.0)).h(px(0.0))
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn install_ca_ios_sim(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let ca = controls.hub.ca.clone();
        let port = current_listen_port(&controls);
        match ca::install_ios_simulators(&ca, port) {
            Ok(ca::IosSimOutcome::Installed { count, port }) => {
                set_status(&controls, StatusMsg::IosSimInstalled { count, port });
            }
            Ok(ca::IosSimOutcome::NoneBooted) => {
                set_status(&controls, StatusMsg::IosSimNoneBooted);
            }
            Err(err) => {
                set_status(&controls, StatusMsg::IosSimFailed(format!("{err:#}")));
            }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn ensure_capture_port_for_launcher(controls: &Controls) -> Option<u16> {
    if !lock_bool(&controls.running) {
        start_capture(controls);
    }
    if !lock_bool(&controls.running) {
        return None;
    }
    let port = lock_port(&controls.listen_port);
    (port > 0).then_some(port)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn launch_browser_action(controls: Arc<Controls>, initial_url: Option<String>) {
    std::thread::spawn(move || {
        let Some(port) = ensure_capture_port_for_launcher(&controls) else {
            return;
        };
        let ca = controls.hub.ca.clone();
        match launcher::launch_isolated_browser(&ca, port, initial_url.as_deref()) {
            Ok(name) => set_status(&controls, StatusMsg::BrowserLaunched { name, port }),
            Err(err) => set_status(
                &controls,
                StatusMsg::BrowserLaunchFailed(format!("{err:#}")),
            ),
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn launch_terminal_action(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let Some(port) = ensure_capture_port_for_launcher(&controls) else {
            return;
        };
        let ca = controls.hub.ca.clone();
        let auth = controls.hub.proxy_auth_text();
        match launcher::launch_proxied_terminal(&ca, port, &auth) {
            Ok(name) => set_status(&controls, StatusMsg::TerminalLaunched { name, port }),
            Err(err) => set_status(
                &controls,
                StatusMsg::TerminalLaunchFailed(format!("{err:#}")),
            ),
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn toggle_adb_action(controls: Arc<Controls>) {
    if launcher::configured_adb_count() > 0 {
        std::thread::spawn(move || {
            let cleared = launcher::clear_android_adb_proxies();
            if cleared > 0 {
                set_status(&controls, StatusMsg::AdbCleared(cleared));
            }
        });
        return;
    }
    configure_adb_action(controls);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn configure_adb_action(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let Some(port) = ensure_capture_port_for_launcher(&controls) else {
            return;
        };
        let ca = controls.hub.ca.clone();
        match launcher::configure_android_adb(&ca, port) {
            Ok(launcher::AdbOutcome::Configured {
                count,
                port,
                cert_pushed,
            }) => {
                set_status(
                    &controls,
                    StatusMsg::AdbConfigured {
                        count,
                        port,
                        cert_pushed,
                    },
                );
            }
            Ok(launcher::AdbOutcome::NoneConnected) => {
                set_status(&controls, StatusMsg::AdbNoneConnected);
            }
            Err(err) => set_status(&controls, StatusMsg::AdbFailed(format!("{err:#}"))),
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
pub(crate) fn copy_linux_ca_cmd(controls: &Controls) {
    let cmd = ca::linux_system_ca_command(&controls.hub.ca.cert_path());
    set_clipboard(cmd.clone());
    set_status(controls, StatusMsg::CopiedLinuxCaCmd(cmd));
}

pub(crate) fn set_ca_status(controls: &Controls, status: ca::CaStatus) {
    *controls
        .ca_status
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = status;
}

pub(crate) fn lock_ca_status(controls: &Controls) -> ca::CaStatus {
    *controls
        .ca_status
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

#[cfg(not(target_arch = "wasm32"))]
fn set_active_external_ca(controls: &Controls, active: Option<ca::ExternalCaSummary>) {
    *controls
        .active_external_ca
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = active;
}

pub(crate) fn lock_active_external_ca(controls: &Controls) -> Option<ca::ExternalCaSummary> {
    controls
        .active_external_ca
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

pub(crate) fn lock_external_cas(controls: &Controls) -> Vec<ca::ExternalCaSummary> {
    controls
        .external_cas
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

pub(crate) fn set_status(controls: &Controls, message: StatusMsg) {
    if message.toast_kind().is_some() {
        let id = controls
            .toast_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        controls
            .toast_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push((id, message.clone()));
    }
    *controls
        .status
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = message;
}

pub(crate) fn lock_status(controls: &Controls) -> StatusMsg {
    controls
        .status
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn install_ca(_controls: Arc<Controls>) {}

#[cfg(target_arch = "wasm32")]
pub(crate) fn remove_ca(_controls: Arc<Controls>) {}

#[cfg(target_arch = "wasm32")]
pub(crate) fn toggle_system_proxy(_controls: Arc<Controls>) {}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn toggle_capture(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let running = *controls
            .running
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if running {
            stop_capture(&controls);
        } else {
            start_capture(&controls);
        }
    });
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn toggle_capture(controls: Arc<Controls>) {
    let running = lock_bool(&controls.running);
    let listen_all = lock_bool(&controls.listen_all);
    let port = lock_port(&controls.listen_port);
    spawn(async move {
        let body = if running {
            b"{\"on\":false}".to_vec()
        } else {
            serde_json::json!({"on": true, "listen_all": listen_all, "port": port})
                .to_string()
                .into_bytes()
        };
        let request =
            HttpRequest::post("api/capture", body).header("content-type", "application/json");
        match fetch(request).await {
            Ok(response) => {
                let value: serde_json::Value =
                    serde_json::from_str(&response.text()).unwrap_or(serde_json::json!({}));
                apply_capture_state(&controls, &value, response.ok());
                proxy::notify_poll_wake();
            }
            Err(err) => set_status(&controls, StatusMsg::BindFailed { port, err }),
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn restart_capture(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        if lock_bool(&controls.running) {
            stop_capture(&controls);
        }
        start_capture(&controls);
    });
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn restart_capture(controls: Arc<Controls>) {
    let listen_all = lock_bool(&controls.listen_all);
    let port = lock_port(&controls.listen_port);
    spawn(async move {
        if lock_bool(&controls.running) {
            let stop_req = HttpRequest::post("api/capture", b"{\"on\":false}".to_vec())
                .header("content-type", "application/json");
            let _ = fetch(stop_req).await;
        }
        let body = serde_json::json!({"on": true, "listen_all": listen_all, "port": port})
            .to_string()
            .into_bytes();
        let request =
            HttpRequest::post("api/capture", body).header("content-type", "application/json");
        match fetch(request).await {
            Ok(response) => {
                let value: serde_json::Value =
                    serde_json::from_str(&response.text()).unwrap_or(serde_json::json!({}));
                apply_capture_state(&controls, &value, response.ok());
                proxy::notify_poll_wake();
            }
            Err(err) => set_status(&controls, StatusMsg::BindFailed { port, err }),
        }
    });
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn switch_to_lan(controls: Arc<Controls>) {
    set_bool(&controls.listen_all, true);
    let port = lock_port(&controls.listen_port);
    spawn(async move {
        if lock_bool(&controls.running) {
            let stop_req = HttpRequest::post("api/capture", b"{\"on\":false}".to_vec())
                .header("content-type", "application/json");
            let _ = fetch(stop_req).await;
        }
        let body = serde_json::json!({"on": true, "listen_all": true, "port": port})
            .to_string()
            .into_bytes();
        let request =
            HttpRequest::post("api/capture", body).header("content-type", "application/json");
        match fetch(request).await {
            Ok(response) => {
                let value: serde_json::Value =
                    serde_json::from_str(&response.text()).unwrap_or(serde_json::json!({}));
                apply_capture_state(&controls, &value, response.ok());
                proxy::notify_poll_wake();
            }
            Err(err) => set_status(&controls, StatusMsg::BindFailed { port, err }),
        }
    });
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn url_encode_query_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for &b in text.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn json_port(value: &serde_json::Value) -> Option<u16> {
    value
        .get("listen_port")
        .and_then(serde_json::Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn apply_capture_state(controls: &Controls, value: &serde_json::Value, http_ok: bool) {
    let capturing = value
        .get("capturing")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let listen = value
        .get("listen")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let port = json_port(value);
    let error = value
        .get("error")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    set_bool(&controls.running, capturing);
    *controls
        .listen
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = listen;
    if let Some(port) = port {
        set_port(&controls.listen_port, port);
    }
    if let Some(all) = value.get("listen_all").and_then(serde_json::Value::as_bool) {
        set_bool(&controls.listen_all, all);
    }
    if capturing {
        set_status(
            controls,
            StatusMsg::Listening {
                port: port.unwrap_or(0),
                via_system_proxy: false,
            },
        );
    } else if !http_ok || !error.is_empty() {
        set_status(
            controls,
            StatusMsg::BindFailed {
                port: port.unwrap_or(0),
                err: if error.is_empty() {
                    let lang = load_lang_pref().unwrap_or_else(|| lang_from_tag(locale().as_str()));
                    t(lang, "没有开始", "not started").to_string()
                } else {
                    error.to_string()
                },
            },
        );
    } else {
        set_status(controls, StatusMsg::Stopped);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn start_capture(controls: &Controls) {
    let _ops = lock_ops(controls);
    if lock_bool(&controls.running) {
        return;
    }
    set_status(controls, StatusMsg::Starting);
    let port = lock_port(&controls.listen_port);
    let ip = if lock_bool(&controls.listen_all) {
        [0, 0, 0, 0]
    } else {
        [127, 0, 0, 1]
    };
    let addr = SocketAddr::from((ip, port));
    let capture = match Capture::spawn(controls.hub.clone(), addr) {
        Ok(capture) => capture,
        Err(err) => {
            set_status(
                controls,
                StatusMsg::BindFailed {
                    port,
                    err: format!("{err:#}"),
                },
            );
            return;
        }
    };
    let port = capture.addr.port();
    set_port(&controls.listen_port, port);
    let remembered = store_listen_port(port);
    *controls
        .listen
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = capture.addr.to_string();
    *controls
        .capture
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = Some(capture);
    set_bool(&controls.running, true);
    if lock_bool(&controls.use_system_proxy) {
        apply_system_proxy(controls);
    } else {
        set_bool(&controls.proxy_applied, false);
        set_status(
            controls,
            StatusMsg::Listening {
                port,
                via_system_proxy: false,
            },
        );
    }
    if let Err(err) = remembered {
        let base = lock_status(controls);
        set_status(
            controls,
            StatusMsg::PortUnsaved {
                base: Box::new(base),
                err: err.to_string(),
            },
        );
    }
    remember_capturing(controls, true);
    if transparent_proxy::cached_status().is_on_or_starting() {
        sync_transparent_proxy_config(controls);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn stop_capture(controls: &Controls) {
    let _ops = lock_ops(controls);
    if let Some(capture) = controls
        .capture
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
    {
        capture.stop();
    }
    stop_relay_push(controls);
    controls.hub.end_share();
    ca::clear_ios_simulator_proxies();
    launcher::clear_android_adb_proxies();
    transparent_proxy::cleanup_on_exit();
    *controls
        .listen
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = String::new();
    let applied = lock_bool(&controls.proxy_applied);
    set_bool(&controls.proxy_applied, false);
    set_bool(&controls.running, false);
    if !applied {
        set_status(controls, StatusMsg::Stopped);
    } else {
        match sysproxy::disable() {
            Ok(()) => set_status(controls, StatusMsg::StoppedRestored),
            Err(err) => set_status(controls, StatusMsg::StopRestoreFailed(format!("{err:#}"))),
        }
    }
    remember_capturing(controls, false);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn switch_to_lan(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let _ops = lock_ops(&controls);
        set_bool(&controls.listen_all, true);
        if let Err(err) = store_listen_all(true) {
            set_status(
                &controls,
                StatusMsg::PrefUnsaved {
                    name: PrefName::Lan,
                    err: err.to_string(),
                },
            );
        }
        if !lock_bool(&controls.running) {
            return;
        }
        if let Some(capture) = controls
            .capture
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take()
        {
            capture.stop();
        }
        let port = lock_port(&controls.listen_port);
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        match Capture::spawn(controls.hub.clone(), addr) {
            Ok(capture) => {
                let port = capture.addr.port();
                set_port(&controls.listen_port, port);
                *controls
                    .listen
                    .lock()
                    .unwrap_or_else(|err| err.into_inner()) = capture.addr.to_string();
                *controls
                    .capture
                    .lock()
                    .unwrap_or_else(|err| err.into_inner()) = Some(capture);
            }
            Err(err) => {
                ca::clear_ios_simulator_proxies();
                launcher::clear_android_adb_proxies();
                transparent_proxy::cleanup_on_exit();
                *controls
                    .listen
                    .lock()
                    .unwrap_or_else(|err| err.into_inner()) = String::new();
                let applied = lock_bool(&controls.proxy_applied);
                set_bool(&controls.proxy_applied, false);
                set_bool(&controls.running, false);
                if applied {
                    let _ = sysproxy::disable();
                }
                set_status(
                    &controls,
                    StatusMsg::BindFailed {
                        port,
                        err: format!("{err:#}"),
                    },
                );
                remember_capturing(&controls, false);
            }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn remember_capturing(controls: &Controls, on: bool) {
    if let Err(err) = save_config(|config| config.capturing = on) {
        set_status(
            controls,
            StatusMsg::PrefUnsaved {
                name: PrefName::Capture,
                err: err.to_string(),
            },
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn toggle_system_proxy(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let _ops = lock_ops(&controls);
        let on = !lock_bool(&controls.use_system_proxy);
        set_bool(&controls.use_system_proxy, on);
        let running = lock_bool(&controls.running);
        if let Err(err) = store_use_system_proxy(on)
            && !running
        {
            set_status(&controls, StatusMsg::ProxyOptionUnsaved(err.to_string()));
            return;
        }
        if !running {
            return;
        }
        if on {
            apply_system_proxy(&controls);
        } else {
            clear_system_proxy(&controls);
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn apply_system_proxy(controls: &Controls) {
    let Some(port) = capture_port(controls) else {
        set_bool(&controls.proxy_applied, false);
        return;
    };
    match sysproxy::enable(port) {
        Ok(sysproxy::EnableOutcome::Applied) => {
            set_bool(&controls.proxy_applied, true);
            set_status(
                controls,
                StatusMsg::Listening {
                    port,
                    via_system_proxy: true,
                },
            );
        }
        Ok(sysproxy::EnableOutcome::Manual { reason }) => {
            set_bool(&controls.proxy_applied, false);
            set_status(controls, StatusMsg::ProxyManual(reason));
        }
        Err(err) => {
            set_bool(&controls.proxy_applied, false);
            set_status(
                controls,
                StatusMsg::ProxyApplyFailed {
                    port,
                    err: format!("{err:#}"),
                },
            );
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn clear_system_proxy(controls: &Controls) {
    let port = capture_port(controls);
    if !lock_bool(&controls.proxy_applied) {
        if let Some(port) = port {
            set_status(
                controls,
                StatusMsg::Listening {
                    port,
                    via_system_proxy: false,
                },
            );
        }
        return;
    }
    match sysproxy::disable() {
        Ok(()) => {
            set_bool(&controls.proxy_applied, false);
            if let Some(port) = port {
                set_status(
                    controls,
                    StatusMsg::Listening {
                        port,
                        via_system_proxy: false,
                    },
                );
            }
        }
        Err(err) => {
            set_bool(&controls.use_system_proxy, true);
            let _ = store_use_system_proxy(true);
            set_status(controls, StatusMsg::ProxyRestoreFailed(format!("{err:#}")));
        }
    }
}

pub(crate) fn capture_port(controls: &Controls) -> Option<u16> {
    controls
        .capture
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .as_ref()
        .map(|capture| capture.addr.port())
}

pub(crate) fn lock_ops(controls: &Controls) -> std::sync::MutexGuard<'_, ()> {
    controls.ops.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn web_config_slot() -> &'static Mutex<config::Config> {
    static SLOT: std::sync::OnceLock<Mutex<config::Config>> = std::sync::OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(config::Config::default_for(false)))
}

#[cfg(all(not(target_arch = "wasm32"), test))]
thread_local! {
    static TEST_CONFIG_SLOT: std::cell::RefCell<config::Config> =
        std::cell::RefCell::new(config::Config::default_for(false));
}

#[cfg(all(not(target_arch = "wasm32"), test))]
pub(crate) fn reset_test_config() {
    TEST_CONFIG_SLOT.with(|slot| {
        *slot.borrow_mut() = config::Config::default_for(false);
    });
}

#[cfg(all(not(target_arch = "wasm32"), test))]
pub(crate) fn saved_config() -> config::Config {
    TEST_CONFIG_SLOT.with(|slot| slot.borrow().clone())
}

#[cfg(all(not(target_arch = "wasm32"), not(test)))]
pub(crate) fn saved_config() -> config::Config {
    config::load_mode(&ca::data_dir(), config::is_dev_mode())
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn saved_config() -> config::Config {
    web_config_slot()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

#[cfg(all(not(target_arch = "wasm32"), test))]
pub(crate) fn save_config(change: impl FnOnce(&mut config::Config)) -> std::io::Result<()> {
    TEST_CONFIG_SLOT.with(|slot| {
        let mut cfg = slot.borrow_mut();
        change(&mut cfg);
    });
    Ok(())
}

#[cfg(all(not(target_arch = "wasm32"), not(test)))]
pub(crate) fn save_config(change: impl FnOnce(&mut config::Config)) -> std::io::Result<()> {
    config::update_mode(&ca::data_dir(), config::is_dev_mode(), change)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn save_config(change: impl FnOnce(&mut config::Config)) -> std::io::Result<()> {
    let (before, after) = {
        let mut guard = web_config_slot()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let before = guard.clone();
        change(&mut guard);
        let after = guard.clone();
        (before, after)
    };
    proxy::sync_web_config_diff(&before, &after);
    Ok(())
}

pub(crate) fn save_config_or_notify(
    controls: &Controls,
    name: PrefName,
    change: impl FnOnce(&mut config::Config),
) -> bool {
    match save_config(change) {
        Ok(()) => true,
        Err(err) => {
            set_status(
                controls,
                StatusMsg::PrefUnsaved {
                    name,
                    err: err.to_string(),
                },
            );
            false
        }
    }
}

pub(crate) fn load_use_system_proxy() -> bool {
    saved_config().use_system_proxy
}

pub(crate) fn store_use_system_proxy(on: bool) -> std::io::Result<()> {
    save_config(|config| config.use_system_proxy = on)
}

pub(crate) fn load_listen_port() -> u16 {
    saved_config().listen_port
}

pub(crate) fn store_listen_port(port: u16) -> std::io::Result<()> {
    save_config(|config| config.listen_port = port)
}

pub(crate) fn current_listen_port(controls: &Controls) -> u16 {
    if let Some(capture) = controls
        .capture
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .as_ref()
    {
        return capture.addr.port();
    }
    lock_port(&controls.listen_port)
}

pub(crate) fn lock_port(slot: &Mutex<u16>) -> u16 {
    *slot.lock().unwrap_or_else(|err| err.into_inner())
}

pub(crate) fn set_port(slot: &Mutex<u16>, port: u16) {
    *slot.lock().unwrap_or_else(|err| err.into_inner()) = port;
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn transparent_proxy_config_from(
    controls: &Controls,
) -> transparent_proxy::TransparentProxyConfig {
    let cfg = saved_config();
    transparent_proxy::TransparentProxyConfig {
        port: current_listen_port(controls),
        proxy_auth: controls.hub.proxy_auth_text(),
        capture_all: cfg.tp_capture_all,
        targets: transparent_proxy::parse_targets(&cfg.tp_targets),
        block_quic: cfg.tp_block_quic,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn sync_transparent_proxy_config(controls: &Controls) {
    let status = transparent_proxy::cached_status();
    if !status.embedded || !status.is_on_or_starting() {
        return;
    }
    let tp_cfg = transparent_proxy_config_from(controls);
    std::thread::spawn(move || {
        let _ = transparent_proxy::update_config(&tp_cfg);
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn toggle_transparent_proxy(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let st = transparent_proxy::refresh_status();
        if st.is_on_or_starting() {
            match transparent_proxy::stop() {
                Ok(()) => set_status(&controls, StatusMsg::TransparentProxyStopped),
                Err(err) => set_status(&controls, StatusMsg::TransparentProxyFailed(err)),
            }
            return;
        }
        if !st.embedded {
            set_status(
                &controls,
                StatusMsg::TransparentProxyFailed("当前构建未内嵌系统扩展".to_string()),
            );
            return;
        }
        if !st.in_applications {
            set_status(&controls, StatusMsg::TransparentProxyNeedsApplications);
            return;
        }
        if ensure_capture_port_for_launcher(&controls).is_none() {
            return;
        }
        let tp_cfg = transparent_proxy_config_from(&controls);
        match transparent_proxy::start(&tp_cfg) {
            Ok(transparent_proxy::StartOutcome::Started) => {
                set_status(&controls, StatusMsg::TransparentProxyStarted);
            }
            Ok(transparent_proxy::StartOutcome::NeedsApproval) => {
                set_status(&controls, StatusMsg::TransparentProxyNeedsApproval);
            }
            Err(err) => {
                set_status(&controls, StatusMsg::TransparentProxyFailed(err));
            }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn refresh_transparent_proxy_state_async() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static BUSY: AtomicBool = AtomicBool::new(false);
    if BUSY.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(|| {
        let st = transparent_proxy::refresh_status();
        if st.embedded {
            let _ = transparent_proxy::refresh_running_apps();
        }
        BUSY.store(false, Ordering::Release);
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn poll_transparent_proxy_if_pending(last_poll: &std::cell::Cell<SystemTime>) {
    if clock::elapsed(last_poll.get()) < Duration::from_secs(2) {
        return;
    }
    last_poll.set(clock::now());
    refresh_transparent_proxy_state_async();
}
