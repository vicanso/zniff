//! Runs zniff on the machine's native nsh backend.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
use nsh_platform_linux as platform;
#[cfg(target_os = "macos")]
use nsh_platform_macos as platform;
#[cfg(target_os = "windows")]
use nsh_platform_windows as platform;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn main() -> impl std::process::Termination {
    if let Some(code) = zniff::maybe_run_cli_mode() {
        std::process::exit(code);
    }
    env_logger::init();
    let _ = rustls::crypto::ring::default_provider().install_default();
    if let Err(err) = zniff::restore_if_dirty() {
        log::error!("启动时恢复系统代理失败: {err:#}");
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Err(err) = zniff::restore_if_dirty() {
            eprintln!("panic 时恢复系统代理失败: {err:#}");
        }
        zniff::clear_ios_simulator_proxies();
        zniff::shutdown_relay_push();
        previous(info);
    }));
    let result = platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<zniff::App>()),
        platform::AppOptions {
            title: "zniff".to_owned(),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            app_id: "dev.zniff.app".to_owned(),
            size: zniff::initial_window_size(),
            ..Default::default()
        },
    );
    if let Err(err) = zniff::restore_if_dirty() {
        log::error!("退出时恢复系统代理失败: {err:#}");
    }
    zniff::clear_ios_simulator_proxies();
    zniff::shutdown_relay_push();
    result
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    target_arch = "wasm32"
)))]
fn main() {
    compile_error!("zniff 的桌面入口只在 macOS、Windows 和 Linux 上编译。");
}
