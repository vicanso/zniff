//! Runs zniff on the machine's native nsh backend.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(all(feature = "desktop", target_os = "linux"))]
use nsh_platform_linux as platform;
#[cfg(all(feature = "desktop", target_os = "macos"))]
use nsh_platform_macos as platform;
#[cfg(all(feature = "desktop", target_os = "windows"))]
use nsh_platform_windows as platform;

#[cfg(all(
    feature = "desktop",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
fn main() -> impl std::process::Termination {
    zniff::mark_process_start();
    if let Some(code) = zniff::maybe_run_cli_mode() {
        std::process::exit(code);
    }
    zniff::ensure_desktop_runtime_init();
    let mut title_bar = nsh_core::desktop::TitleBarOptions::default();
    title_bar.style = nsh_core::desktop::TitleBarStyle::Hidden;
    title_bar.height = Some(nsh_core::geometry::px(42.0));
    let mut attributes = nsh_core::desktop::WindowAttributes::default();
    attributes.title_bar = Some(title_bar);
    let mut options = nsh_core::desktop::MainWindowOptions::default();
    options.attributes = attributes;
    zniff::apply_initial_window_options(&mut options);
    let root = platform::Root::of::<zniff::App>().main_window(options);
    platform::set_ui_sets(vec![nsh_base::registry_of([
        nsh_base::parts::core as nsh_base::parts::Part,
        nsh_base::parts::command as nsh_base::parts::Part,
    ])]);
    let result = platform::run_app(
        platform::AppSource::Rust(root),
        platform::AppOptions {
            title: "zniff".to_owned(),
            app_id: "dev.zniff.app".to_owned(),
            size: zniff::initial_window_size(),
            ..Default::default()
        },
    );
    zniff::cleanup_desktop_runtime();
    result
}

#[cfg(all(not(feature = "desktop"), not(target_arch = "wasm32")))]
fn main() {
    if let Some(code) = zniff::maybe_run_cli_mode() {
        std::process::exit(code);
    }
    eprintln!("当前二进制为无界面构建 (--no-default-features)，请指定 --web、--relay 或 --mcp");
    std::process::exit(2);
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(all(
    feature = "desktop",
    not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_arch = "wasm32"
    ))
))]
fn main() {
    compile_error!("zniff 的桌面入口只在 macOS、Windows 和 Linux 上编译。");
}
