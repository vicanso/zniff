//! zniff: a desktop HTTP(S) capture app. The window is an nsh Rust view.
//! The proxy runs on its own tokio runtime in this process.

#![allow(clippy::too_many_arguments)]
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

mod ai_context;
mod allow_hosts;
mod android_guide;
#[cfg(not(target_arch = "wasm32"))]
mod bypass;
#[cfg(not(target_arch = "wasm32"))]
mod ca;
#[cfg(target_arch = "wasm32")]
#[path = "ca_web.rs"]
mod ca;
#[cfg(not(target_arch = "wasm32"))]
mod client_hello;
mod clock;
mod collections;
mod config;
mod copy_formats;
#[cfg(not(target_arch = "wasm32"))]
mod decompress;
mod diagnostics;
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
mod external_ca;
#[cfg(not(target_arch = "wasm32"))]
mod har;
mod i18n;
mod icon;
mod import_charles;
mod import_whistle;
#[cfg(not(target_arch = "wasm32"))]
mod ios_guide;
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
mod launcher;
mod llm_stream;
#[cfg(not(target_arch = "wasm32"))]
mod local_proc;
#[cfg(not(target_arch = "wasm32"))]
mod mcp;
#[cfg(not(target_arch = "wasm32"))]
mod mcp_resources;
#[cfg(not(target_arch = "wasm32"))]
mod mcp_tools;
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
mod mock_gen;
mod openapi;
#[cfg(not(target_arch = "wasm32"))]
pub mod perf_bench;
mod present;
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
mod proto_schema;
#[cfg(not(target_arch = "wasm32"))]
mod proxy;
#[cfg(target_arch = "wasm32")]
#[path = "hub_web.rs"]
mod proxy;
mod redact;
#[cfg(not(target_arch = "wasm32"))]
mod relay;
mod rule_vars;
mod rules;
mod sample_har;
mod session;
#[cfg(not(target_arch = "wasm32"))]
mod share;
mod startup_metrics;
#[cfg(not(target_arch = "wasm32"))]
mod sysproxy;
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
mod transparent_proxy;
mod ui;
#[cfg(not(target_arch = "wasm32"))]
mod updater;
mod watch;
#[cfg(not(target_arch = "wasm32"))]
mod web;
#[cfg(not(target_arch = "wasm32"))]
mod webhook;
#[cfg(not(target_arch = "wasm32"))]
mod ws;

#[cfg(not(target_arch = "wasm32"))]
pub use ca::clear_ios_simulator_proxies;
#[cfg(not(target_arch = "wasm32"))]
pub use launcher::cleanup_on_exit as cleanup_launchers;
use nsh_core::prelude::{Registry, native_module};
#[cfg(not(target_arch = "wasm32"))]
pub use relay::shutdown_relay_push;
pub use startup_metrics::mark_process_start;
#[cfg(not(target_arch = "wasm32"))]
pub use sysproxy::restore_if_dirty;
#[cfg(not(target_arch = "wasm32"))]
pub use transparent_proxy::cleanup_on_exit as cleanup_transparent_proxy;
pub use ui::{App, initial_window_size};
#[cfg(not(target_arch = "wasm32"))]
pub use ui::{
    apply_initial_window_options, apply_runtime_cli_flags_best_effort, maybe_run_cli_mode,
};

#[cfg(all(not(target_arch = "wasm32"), not(test)))]
static DESKTOP_CLEANED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Runs idempotent desktop-process cleanup (system proxy restore, simulator/ADB/sysext cleanup,
/// and relay shutdown). No-ops in unit tests so test runs never touch the host machine.
#[cfg(not(target_arch = "wasm32"))]
pub fn cleanup_desktop_runtime() {
    #[cfg(not(test))]
    if !DESKTOP_CLEANED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        if let Err(err) = restore_if_dirty() {
            log::error!("退出时恢复系统代理失败: {err:#}");
        }
        clear_ios_simulator_proxies();
        cleanup_launchers();
        cleanup_transparent_proxy();
        shutdown_relay_push();
    }
}

#[cfg(all(not(target_arch = "wasm32"), unix, not(test)))]
extern "C" fn atexit_cleanup_desktop_runtime() {
    cleanup_desktop_runtime();
}

/// Initializes process-wide desktop runtime state (startup timestamp, rustls crypto provider,
/// CLI flag overrides, logger, dirty proxy recovery, panic hook, and exit handler) idempotently.
/// Safe to invoke both from `app/src/main.rs` and from `nsh package` / `nsh run` generated runners.
#[cfg(not(target_arch = "wasm32"))]
pub fn ensure_desktop_runtime_init() {
    static DESKTOP_INIT: std::sync::Once = std::sync::Once::new();
    DESKTOP_INIT.call_once(|| {
        mark_process_start();
        let _ = rustls::crypto::ring::default_provider().install_default();
        #[cfg(not(test))]
        {
            apply_runtime_cli_flags_best_effort();
            let _ = env_logger::try_init();
            if let Err(err) = restore_if_dirty() {
                log::error!("启动时恢复系统代理失败: {err:#}");
            }
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                if let Err(err) = restore_if_dirty() {
                    eprintln!("panic 时恢复系统代理失败: {err:#}");
                }
                clear_ios_simulator_proxies();
                cleanup_launchers();
                cleanup_transparent_proxy();
                shutdown_relay_push();
                DESKTOP_CLEANED.store(true, std::sync::atomic::Ordering::SeqCst);
                previous(info);
            }));
            #[cfg(unix)]
            unsafe {
                libc::atexit(atexit_cleanup_desktop_runtime);
            }
        }
    });
}

/// The app's UI set (`nsh.json` `"rust"`). The web package calls this entry
/// even though `rust_app!` also sets the root.
#[native_module("zniff/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}
