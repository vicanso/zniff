//! System tray / menu bar icon and opt-in minimize-to-tray lifecycle (ADR-23).
//!
//! Uses `nsh_core::element::desktop::tray()`, `set_tray()`, and
//! `main_window().on_close_press(...)` on desktop targets; compiles cleanly as a no-op on wasm32.

#[allow(unused_imports)]
use super::*;

/// Localized labels for the lightweight system tray menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrayMenuLabels {
    pub(crate) tooltip: String,
    pub(crate) show_window: &'static str,
    pub(crate) toggle_capture: &'static str,
    pub(crate) system_proxy: &'static str,
    pub(crate) quit: &'static str,
}

/// Computes localized tray tooltip and menu labels from live capture state.
pub(crate) fn tray_menu_labels(lang: Lang, running: bool, listen_addr: &str) -> TrayMenuLabels {
    let tooltip = if running {
        let addr = listen_addr.trim();
        if addr.is_empty() {
            t(lang, "zniff · 抓包中", "zniff · Capturing").to_string()
        } else {
            match lang {
                Lang::Zh => format!("zniff · 抓包中 ({addr})"),
                Lang::En => format!("zniff · Capturing ({addr})"),
            }
        }
    } else {
        t(lang, "zniff · 已停止", "zniff · Stopped").to_string()
    };
    TrayMenuLabels {
        tooltip,
        show_window: t(lang, "显示窗口", "Show Window"),
        toggle_capture: if running {
            t(lang, "停止抓包", "Stop Capture")
        } else {
            t(lang, "开始抓包", "Start Capture")
        },
        system_proxy: t(lang, "系统代理", "System Proxy"),
        quit: t(lang, "退出 zniff", "Quit zniff"),
    }
}

/// Installs the reactive desktop system tray / menu bar icon and lightweight menu.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn install_system_tray(
    controls: Arc<Controls>,
    langs: Signal<Option<Lang>>,
    running: Signal<bool>,
    listen: Signal<String>,
    use_system_proxy: Signal<bool>,
) {
    use nsh_core::element::desktop::{MenuItem, main_window, quit, set_tray, tray};

    effect(move || {
        let lang = ui_lang(langs);
        let is_running = running.get();
        let listen_addr = listen.get();
        let proxy_on = use_system_proxy.get();
        let labels = tray_menu_labels(lang, is_running, &listen_addr);

        let cap_controls = controls.clone();
        let proxy_controls = controls.clone();
        let items = [
            MenuItem::new(labels.show_window).on_select(|| {
                let win = main_window();
                win.show();
                win.focus();
            }),
            MenuItem::separator(),
            MenuItem::new(labels.toggle_capture).on_select(move || {
                toggle_capture(cap_controls.clone());
            }),
            MenuItem::new(labels.system_proxy)
                .checked(proxy_on)
                .on_select(move || {
                    toggle_system_proxy(proxy_controls.clone());
                }),
            MenuItem::separator(),
            MenuItem::new(labels.quit).on_select(quit),
        ];

        set_tray(tray().tooltip(labels.tooltip).menu(items));
    });

    cleanup(|| {
        set_tray(None);
    });
}

/// Syncs `main_window` `close_on_press` with `minimize_to_tray` and hides the window
/// instead of quitting when the user has opted into tray residency.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn install_minimize_to_tray_handler(minimize_to_tray: Signal<bool>) {
    use nsh_core::element::desktop::main_window;

    effect(move || {
        let keep_in_tray = minimize_to_tray.get();
        let mut attrs = nsh_core::desktop::WindowAttributes::default();
        attrs.close_on_press = Some(!keep_in_tray);
        main_window().set(attrs);
    });

    main_window().on_close_press(move |_| {
        if minimize_to_tray.get_untracked() {
            main_window().hide();
        } else {
            main_window().close();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_labels_reflect_capture_state_and_language() {
        let zh_idle = tray_menu_labels(Lang::Zh, false, "");
        assert_eq!(zh_idle.tooltip, "zniff · 已停止");
        assert_eq!(zh_idle.show_window, "显示窗口");
        assert_eq!(zh_idle.toggle_capture, "开始抓包");
        assert_eq!(zh_idle.system_proxy, "系统代理");
        assert_eq!(zh_idle.quit, "退出 zniff");

        let zh_on = tray_menu_labels(Lang::Zh, true, "127.0.0.1:8877");
        assert_eq!(zh_on.tooltip, "zniff · 抓包中 (127.0.0.1:8877)");
        assert_eq!(zh_on.toggle_capture, "停止抓包");

        let en_idle = tray_menu_labels(Lang::En, false, "");
        assert_eq!(en_idle.tooltip, "zniff · Stopped");
        assert_eq!(en_idle.show_window, "Show Window");
        assert_eq!(en_idle.toggle_capture, "Start Capture");
        assert_eq!(en_idle.system_proxy, "System Proxy");
        assert_eq!(en_idle.quit, "Quit zniff");

        let en_on = tray_menu_labels(Lang::En, true, "127.0.0.1:8877");
        assert_eq!(en_on.tooltip, "zniff · Capturing (127.0.0.1:8877)");
        assert_eq!(en_on.toggle_capture, "Stop Capture");
    }
}
