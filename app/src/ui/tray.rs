//! System tray / menu bar icon and opt-in minimize-to-tray lifecycle (ADR-23).
//!
//! Uses `nsh_core::element::desktop::tray()`, `set_tray()`, and
//! `main_window().on_close_press(...)` on desktop targets; compiles cleanly as a no-op on wasm32.

#[allow(unused_imports)]
use super::*;

const TRAY_ICON_PNG: &[u8] = include_bytes!("../../assets/tray-icon.png");

/// Localized labels for the lightweight system tray menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrayMenuLabels {
    pub(crate) tooltip: String,
    pub(crate) show_window: &'static str,
    pub(crate) toggle_capture: &'static str,
    pub(crate) system_proxy: &'static str,
    pub(crate) clear_traffic: &'static str,
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
        clear_traffic: t(lang, "清除抓包记录", "Clear Traffic"),
        quit: t(lang, "退出 zniff", "Quit zniff"),
    }
}

/// Returns the tightly cropped 64×64 zniff app icon data URI for the system tray across all platforms.
pub(crate) fn tray_icon_uri() -> String {
    static TRAY_ICON_URI: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TRAY_ICON_URI
        .get_or_init(|| format!("data:image/png;base64,{}", encode_base64(TRAY_ICON_PNG)))
        .clone()
}

/// Builds a monochrome 16×16 stroke SVG data URI for tray menu items.
/// On macOS, SVG menu icons render as template images adapting to light/dark menu bars.
pub(crate) fn tray_menu_svg_uri(paths: &str) -> String {
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{paths}</svg>"##
    );
    format!(
        "data:image/svg+xml;base64,{}",
        encode_base64(svg.as_bytes())
    )
}

fn tray_menu_icons(running: bool) -> [String; 5] {
    let show = tray_menu_svg_uri(
        r#"<rect x="2" y="3" width="12" height="10" rx="2"/><path d="M2 6.2 H14"/>"#,
    );
    let capture = if running {
        tray_menu_svg_uri(r#"<rect x="4" y="4" width="8" height="8" rx="1.5"/>"#)
    } else {
        tray_menu_svg_uri(r#"<path d="M5 3.5 L12.5 8 L5 12.5 Z"/>"#)
    };
    let proxy = tray_menu_svg_uri(
        r#"<circle cx="8" cy="8" r="5.5"/><path d="M2.6 8 H13.4"/><ellipse cx="8" cy="8" rx="2.6" ry="5.5"/>"#,
    );
    let clear = tray_menu_svg_uri(
        r#"<path d="M3 4.5 H13 M6 4.5 V3 H10 V4.5 M4.5 4.5 L5.2 13 H10.8 L11.5 4.5 M6.8 7.2 V10.8 M9.2 7.2 V10.8"/>"#,
    );
    let quit = tray_menu_svg_uri(r#"<path d="M8 2.5 V7.8 M4.8 4.4 A5 5 0 1 0 11.2 4.4"/>"#);
    [show, capture, proxy, clear, quit]
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct TrayProps {
    pub(crate) controls: Arc<Controls>,
    pub(crate) langs: Signal<Option<Lang>>,
    pub(crate) show_tray: Signal<bool>,
    pub(crate) running: Signal<bool>,
    pub(crate) listen: Signal<String>,
    pub(crate) use_system_proxy: Signal<bool>,
    pub(crate) on_clear: Rc<dyn Fn()>,
}

/// Installs the reactive desktop system tray / menu bar icon and lightweight menu.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn install_system_tray(props: TrayProps) {
    use nsh_core::element::desktop::{MenuItem, main_window, quit, set_tray, tray};

    let TrayProps {
        controls,
        langs,
        show_tray,
        running,
        listen,
        use_system_proxy,
        on_clear,
    } = props;

    effect(move || {
        if !show_tray.get() {
            set_tray(None);
            return;
        }
        let lang = ui_lang(langs);
        let is_running = running.get();
        let listen_addr = listen.get();
        let proxy_on = use_system_proxy.get();
        let labels = tray_menu_labels(lang, is_running, &listen_addr);
        let [show_icon, capture_icon, proxy_icon, clear_icon, quit_icon] =
            tray_menu_icons(is_running);

        let cap_controls = controls.clone();
        let proxy_controls = controls.clone();
        let clear_action = on_clear.clone();
        let items = [
            MenuItem::new(labels.show_window)
                .icon(show_icon)
                .on_select(|| {
                    let win = main_window();
                    win.show();
                    win.focus();
                }),
            MenuItem::separator(),
            MenuItem::new(labels.toggle_capture)
                .icon(capture_icon)
                .on_select(move || {
                    toggle_capture(cap_controls.clone());
                }),
            MenuItem::new(labels.system_proxy)
                .icon(proxy_icon)
                .checked(proxy_on)
                .on_select(move || {
                    toggle_system_proxy(proxy_controls.clone());
                }),
            MenuItem::new(labels.clear_traffic)
                .icon(clear_icon)
                .on_select(move || {
                    clear_action();
                }),
            MenuItem::separator(),
            MenuItem::new(labels.quit).icon(quit_icon).on_select(quit),
        ];

        set_tray(
            tray()
                .icon(tray_icon_uri())
                .tooltip(labels.tooltip)
                .menu(items),
        );
    });

    cleanup(|| {
        set_tray(None);
    });
}

/// Syncs `main_window` `close_on_press` with `show_tray && minimize_to_tray` and hides the window
/// instead of quitting when the user has opted into tray residency.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn install_minimize_to_tray_handler(
    show_tray: Signal<bool>,
    minimize_to_tray: Signal<bool>,
) {
    use nsh_core::element::desktop::main_window;

    effect(move || {
        let keep_in_tray = show_tray.get() && minimize_to_tray.get();
        let mut attrs = nsh_core::desktop::WindowAttributes::default();
        attrs.close_on_press = Some(!keep_in_tray);
        main_window().set(attrs);
    });

    main_window().on_close_press(move |_| {
        if show_tray.get_untracked() && minimize_to_tray.get_untracked() {
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
        assert_eq!(zh_idle.clear_traffic, "清除抓包记录");
        assert_eq!(zh_idle.quit, "退出 zniff");

        let zh_on = tray_menu_labels(Lang::Zh, true, "127.0.0.1:8877");
        assert_eq!(zh_on.tooltip, "zniff · 抓包中 (127.0.0.1:8877)");
        assert_eq!(zh_on.toggle_capture, "停止抓包");

        let en_idle = tray_menu_labels(Lang::En, false, "");
        assert_eq!(en_idle.tooltip, "zniff · Stopped");
        assert_eq!(en_idle.show_window, "Show Window");
        assert_eq!(en_idle.toggle_capture, "Start Capture");
        assert_eq!(en_idle.system_proxy, "System Proxy");
        assert_eq!(en_idle.clear_traffic, "Clear Traffic");
        assert_eq!(en_idle.quit, "Quit zniff");

        let en_on = tray_menu_labels(Lang::En, true, "127.0.0.1:8877");
        assert_eq!(en_on.tooltip, "zniff · Capturing (127.0.0.1:8877)");
        assert_eq!(en_on.toggle_capture, "Stop Capture");
    }

    #[test]
    fn tray_icon_uri_is_embedded_png_data_uri() {
        assert!(TRAY_ICON_PNG.len() < 20_000);
        assert!(TRAY_ICON_PNG.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(&TRAY_ICON_PNG[16..20], &64u32.to_be_bytes());
        assert_eq!(&TRAY_ICON_PNG[20..24], &64u32.to_be_bytes());

        let uri = tray_icon_uri();
        assert!(uri.starts_with("data:image/png;base64,"));
        assert!(uri.len() > 64);

        let icons_idle = tray_menu_icons(false);
        let icons_running = tray_menu_icons(true);
        for icon in icons_idle.iter().chain(icons_running.iter()) {
            assert!(icon.starts_with("data:image/svg+xml;base64,"));
        }
        assert_ne!(icons_idle[1], icons_running[1]);
    }
}
