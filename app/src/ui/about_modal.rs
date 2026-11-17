#[allow(unused_imports)]
use super::*;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const GIT_SHA: &str = env!("ZNIFF_GIT_SHA");
const BUILD_YEAR: &str = env!("ZNIFF_BUILD_YEAR");
const ICON_PNG: &[u8] = include_bytes!("../../assets/icon.png");

const GITHUB_URL: &str = "https://github.com/vicanso/zniff";
const DOCS_URL: &str = "https://github.com/vicanso/zniff#readme";
const ISSUES_URL: &str = "https://github.com/vicanso/zniff/issues";

#[cfg(not(target_arch = "wasm32"))]
static GPU_INFO: std::sync::OnceLock<String> = std::sync::OnceLock::new();

#[cfg_attr(
    any(target_arch = "wasm32", not(target_os = "macos")),
    allow(dead_code)
)]
pub(crate) fn parse_macos_gpu_info(info: &str) -> Option<String> {
    for line in info.lines() {
        if let Some(model) = line.trim().strip_prefix("Chipset Model:") {
            let model = model.trim();
            if !model.is_empty() {
                return Some(model.to_string());
            }
        }
    }
    None
}

#[cfg_attr(
    any(target_arch = "wasm32", not(target_os = "windows")),
    allow(dead_code)
)]
pub(crate) fn parse_windows_gpu_info(info: &str) -> Option<String> {
    info.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.eq_ignore_ascii_case("Name"))
        .map(str::to_string)
}

#[cfg_attr(
    any(target_arch = "wasm32", not(target_os = "linux")),
    allow(dead_code)
)]
pub(crate) fn parse_linux_gpu_info(info: &str) -> Option<String> {
    let trimmed = info.trim();
    if trimmed.is_empty() {
        return None;
    }
    let name = trimmed.rsplit(": ").next().unwrap_or(trimmed).trim();
    (!name.is_empty()).then(|| name.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn get_basic_gpu_info() -> &'static str {
    GPU_INFO
        .get_or_init(|| {
            #[cfg(target_os = "macos")]
            {
                if let Ok(output) = std::process::Command::new("system_profiler")
                    .arg("SPDisplaysDataType")
                    .output()
                {
                    let info = String::from_utf8_lossy(&output.stdout);
                    if let Some(model) = parse_macos_gpu_info(&info) {
                        return model;
                    }
                }
            }
            #[cfg(target_os = "windows")]
            {
                if let Ok(output) = std::process::Command::new("wmic")
                    .args(["path", "win32_VideoController", "get", "name"])
                    .output()
                {
                    let info = String::from_utf8_lossy(&output.stdout);
                    if let Some(name) = parse_windows_gpu_info(&info) {
                        return name;
                    }
                }
            }
            #[cfg(target_os = "linux")]
            {
                if let Ok(output) = std::process::Command::new("sh")
                    .args(["-c", "lspci | grep -i vga"])
                    .output()
                {
                    let info = String::from_utf8_lossy(&output.stdout);
                    if let Some(name) = parse_linux_gpu_info(&info) {
                        return name;
                    }
                }
            }
            "Unknown GPU".to_string()
        })
        .as_str()
}

#[cfg(target_arch = "wasm32")]
fn get_basic_gpu_info() -> &'static str {
    "Browser Web"
}

pub(crate) fn copyright_notice(build_year: &str) -> String {
    let years = if build_year == "2026" || build_year.is_empty() {
        "2026".to_string()
    } else {
        format!("2026 - {build_year}")
    };
    format!("© {years} Tree xie. All rights reserved.")
}

fn renderer_label() -> &'static str {
    if cfg!(target_arch = "wasm32") {
        "NSH Web (Wasm)"
    } else {
        "NSH Desktop"
    }
}

fn system_info_rows(lang: Lang) -> Vec<(&'static str, String)> {
    let win = window();
    let ratio = pixel_ratio();
    let theme = if dark() {
        t(lang, "深色 (Dark)", "Dark")
    } else {
        t(lang, "浅色 (Light)", "Light")
    };
    let (os_major, os_minor) = os_version();
    let os_line = if os_major > 0 {
        format!("{} {os_major}.{os_minor}", platform_name())
    } else {
        platform_name().to_string()
    };
    let loc = locale().to_string();
    let locale_str = if loc.is_empty() {
        "unknown".to_string()
    } else {
        loc
    };

    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut rows = vec![
        ("Version", format!("v{APP_VERSION}")),
        ("Git SHA", GIT_SHA.to_string()),
        ("Renderer", renderer_label().to_string()),
        ("OS", os_line),
        ("Arch", std::env::consts::ARCH.to_string()),
        ("GPU", get_basic_gpu_info().to_string()),
        (
            "Window",
            format!("{:.0}×{:.0} @ {ratio:.2}x", win.width.0, win.height.0),
        ),
        ("Theme", theme.to_string()),
        ("Locale", locale_str),
    ];
    #[cfg(not(target_arch = "wasm32"))]
    {
        rows.push(("Data Dir", ca::data_dir().display().to_string()));
    }
    rows
}

pub(crate) fn format_system_info_text(lang: Lang) -> String {
    system_info_rows(lang)
        .into_iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn open_about_link(controls: Arc<Controls>, url: &'static str) {
    spawn(async move {
        if !open_url(url).await {
            set_clipboard(url);
            set_status(&controls, StatusMsg::CopiedUrl);
        }
    });
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

pub(crate) fn about_logo_uri() -> String {
    static LOGO_URI: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    LOGO_URI
        .get_or_init(|| format!("data:image/png;base64,{}", encode_base64(ICON_PNG)))
        .clone()
}

fn about_link_chip(
    glyph: Glyph,
    label: impl IntoReactive<SharedString>,
    url: &'static str,
    controls: Arc<Controls>,
) -> impl IntoElement {
    view()
        .h(px(28.0))
        .px_2p5()
        .gap_1p5()
        .flex_row()
        .items_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(border_control())
        .bg(bg_control())
        .hover(|s| s.bg(bg_control_hover()))
        .active(|s| s.opacity(0.8))
        .cursor(Cursor::Pointer)
        .on_press(move || open_about_link(controls.clone(), url))
        .child(glyph_px(12.0, move || glyph, || color(fg_muted())))
        .child(control_text(label).text_color(fg_primary()))
}

fn about_meta_badge(label: impl IntoReactive<SharedString>) -> impl IntoElement {
    view()
        .h(px(22.0))
        .px_2()
        .flex_row()
        .items_center()
        .rounded(px(5.0))
        .border_1()
        .border_color(border_subtle())
        .bg(bg_control())
        .child(
            text(label)
                .text_size(px(11.0))
                .text_color(fg_muted())
                .number_of_lines(1),
        )
}

/// Registers a custom macOS application menu bar so "About zniff" opens our in-app modal
/// instead of macOS's default `orderFrontStandardAboutPanel:`, while keeping standard
/// Hide/Quit/Edit/Window roles and shortcuts intact.
#[cfg(target_os = "macos")]
pub(crate) fn install_macos_application_menu(about_open: Signal<bool>) {
    use nsh_core::desktop::MenuRole;
    use nsh_core::element::desktop::{MenuItem, set_application_menu};

    let langs = ui_langs();
    effect(move || {
        let lang = ui_lang(langs);
        set_application_menu(vec![
            MenuItem::submenu(
                "zniff",
                [
                    MenuItem::new(t(lang, "关于 zniff", "About zniff"))
                        .on_select(move || about_open.set(true)),
                    MenuItem::separator(),
                    MenuItem::role(MenuRole::Hide),
                    MenuItem::role(MenuRole::HideOthers),
                    MenuItem::role(MenuRole::ShowAll),
                    MenuItem::separator(),
                    MenuItem::role(MenuRole::Quit),
                ],
            ),
            MenuItem::submenu(
                "Edit",
                [
                    MenuItem::role(MenuRole::Undo),
                    MenuItem::role(MenuRole::Redo),
                    MenuItem::separator(),
                    MenuItem::role(MenuRole::Cut),
                    MenuItem::role(MenuRole::Copy),
                    MenuItem::role(MenuRole::Paste),
                    MenuItem::role(MenuRole::SelectAll),
                ],
            ),
            MenuItem::submenu(
                "Window",
                [
                    MenuItem::role(MenuRole::Minimize),
                    MenuItem::role(MenuRole::Zoom),
                    MenuItem::role(MenuRole::ToggleFullScreen),
                    MenuItem::separator(),
                    MenuItem::role(MenuRole::Close),
                ],
            ),
        ]);
    });
}

/// Modal dialog displaying zniff branding, build metadata, project links, and runtime environment info.
pub(crate) fn about_modal(profiles: ProfileSignals, controls: Arc<Controls>) -> impl IntoElement {
    when(
        move || profiles.about_open.get(),
        move || {
            let langs = ui_langs();
            let controls_copy = controls.clone();
            let controls_links = controls.clone();
            #[cfg(not(target_arch = "wasm32"))]
            let controls_check = controls.clone();

            let backdrop = view()
                .absolute()
                .inset_0()
                .bg(AdaptiveColor::new(rgba(0x0F172A5C), rgba(0x0000008F)))
                .on_press(move || profiles.about_open.set(false));

            let header = view()
                .w_full()
                .px_4()
                .py_3()
                .flex_row()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_b_color(border_subtle())
                .child(glyph_px(
                    14.0,
                    || Glyph::Badge,
                    || color(accent_blue_text()),
                ))
                .child(
                    text(tr("关于 zniff", "About zniff"))
                        .flex_1()
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg_primary()),
                )
                .child(
                    view()
                        .w(px(24.0))
                        .h(px(24.0))
                        .rounded(px(6.0))
                        .flex_shrink_0()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(bg_control_hover()))
                        .active(|s| s.opacity(0.75))
                        .cursor(Cursor::Pointer)
                        .label(tr("关闭", "Close"))
                        .on_press(move || profiles.about_open.set(false))
                        .child(glyph_px(11.0, || Glyph::Close, || color(fg_muted()))),
                );

            let hero = view()
                .w_full()
                .flex_row()
                .items_center()
                .gap_3p5()
                .child(
                    view()
                        .w(px(64.0))
                        .h(px(64.0))
                        .flex_shrink_0()
                        .rounded(px(14.0))
                        .border_1()
                        .border_color(border_subtle())
                        .bg(bg_control())
                        .overflow_hidden()
                        .child(
                            image(about_logo_uri())
                                .w_full()
                                .h_full()
                                .object_fit(ResizeMode::Contain),
                        ),
                )
                .child(
                    view()
                        .flex_1()
                        .gap_1()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(
                                    text("zniff")
                                        .text_size(px(18.0))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(fg_primary()),
                                )
                                .child(
                                    view()
                                        .h(px(20.0))
                                        .px_2()
                                        .flex_row()
                                        .items_center()
                                        .rounded(px(5.0))
                                        .border_1()
                                        .border_color(accent_blue_border())
                                        .bg(accent_blue_soft())
                                        .child(
                                            mono(format!("v{APP_VERSION}"))
                                                .text_size(px(11.0))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(accent_blue_text()),
                                        ),
                                )
                                .child(about_meta_badge(format!("@{GIT_SHA}"))),
                        )
                        .child(
                            text(tr(
                                "轻量级桌面与 Web HTTP(S) 抓包调试工具",
                                "Lightweight desktop & web HTTP(S) traffic inspector",
                            ))
                            .text_size(px(12.5))
                            .text_color(fg_secondary()),
                        )
                        .child(
                            view()
                                .pt_0p5()
                                .flex_row()
                                .flex_wrap()
                                .gap_1p5()
                                .child(about_meta_badge(format!("Rust & {}", renderer_label())))
                                .child(about_meta_badge("Apache License 2.0")),
                        ),
                );

            let links_row = view()
                .w_full()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(about_link_chip(
                    Glyph::ChevronRight,
                    "GitHub",
                    GITHUB_URL,
                    controls_links.clone(),
                ))
                .child(about_link_chip(
                    Glyph::List,
                    tr("文档", "Documentation"),
                    DOCS_URL,
                    controls_links.clone(),
                ))
                .child(about_link_chip(
                    Glyph::Alert,
                    tr("反馈问题", "Report Issue"),
                    ISSUES_URL,
                    controls_links,
                ));

            let sys_header = view()
                .w_full()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    text(tr("系统与运行环境", "System & Runtime"))
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg_muted()),
                )
                .child(
                    view()
                        .h(px(24.0))
                        .px_2()
                        .gap_1()
                        .flex_row()
                        .items_center()
                        .rounded(px(5.0))
                        .border_1()
                        .border_color(border_control())
                        .bg(bg_control())
                        .hover(|s| s.bg(bg_control_hover()))
                        .active(|s| s.opacity(0.8))
                        .cursor(Cursor::Pointer)
                        .on_press(move || {
                            set_clipboard(format_system_info_text(ui_lang(langs)));
                            set_status(&controls_copy, StatusMsg::CopiedSystemInfo);
                        })
                        .child(
                            text(tr("复制环境信息", "Copy System Info"))
                                .text_size(px(11.0))
                                .text_color(fg_primary()),
                        ),
                );

            let rows_list = system_info_rows(ui_lang(langs)).into_iter().fold(
                view().w_full().gap_1(),
                |col, (key, val)| {
                    col.child(
                        view()
                            .w_full()
                            .flex_row()
                            .items_baseline()
                            .gap_2()
                            .child(
                                text(key)
                                    .w(px(76.0))
                                    .flex_shrink_0()
                                    .text_size(px(11.5))
                                    .text_color(fg_muted()),
                            )
                            .child(
                                mono(val)
                                    .flex_1()
                                    .selectable(true)
                                    .text_size(px(11.5))
                                    .text_color(fg_primary()),
                            ),
                    )
                },
            );

            let sys_card = view()
                .w_full()
                .p_3()
                .gap_2()
                .rounded(px(8.0))
                .border_1()
                .border_color(border_subtle())
                .bg(bg_code())
                .child(sys_header)
                .child(rows_list);

            let copyright_line = text(copyright_notice(BUILD_YEAR))
                .text_size(px(11.0))
                .text_color(fg_muted());

            let body = view()
                .w_full()
                .p_4()
                .gap_3p5()
                .flex_col()
                .child(hero)
                .child(links_row)
                .child(sys_card)
                .child(copyright_line);

            let footer = view()
                .w_full()
                .px_4()
                .py_3()
                .flex_row()
                .items_center()
                .justify_end()
                .gap_2()
                .border_t_1()
                .border_t_color(border_subtle());

            #[cfg(not(target_arch = "wasm32"))]
            let footer = footer.child(
                view()
                    .h(px(28.0))
                    .px_3()
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(border_control())
                    .bg(bg_control())
                    .hover(|s| s.bg(bg_control_hover()))
                    .active(|s| s.opacity(0.8))
                    .cursor(Cursor::Pointer)
                    .on_press(move || {
                        profiles.about_open.set(false);
                        check_update_manual(controls_check.clone());
                    })
                    .child(
                        control_text(tr("检查更新", "Check for Updates")).text_color(fg_primary()),
                    ),
            );

            let footer = footer.child(
                view()
                    .h(px(28.0))
                    .px_3p5()
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(accent_blue_border())
                    .bg(accent_blue_soft())
                    .hover(|s| s.bg(accent_blue_soft_hover()))
                    .active(|s| s.opacity(0.8))
                    .cursor(Cursor::Pointer)
                    .on_press(move || profiles.about_open.set(false))
                    .child(control_text(tr("关闭", "Close")).text_color(accent_blue_text())),
            );

            let card = view()
                .w(move || px((window().width.0 - 32.0).clamp(340.0, 520.0)))
                .max_h(move || px((window().height.0 - 56.0).max(260.0)))
                .flex_col()
                .overflow_hidden()
                .rounded(px(12.0))
                .border_1()
                .border_color(border_strong())
                .bg(bg_card())
                .shadow_lg()
                .child(header)
                .child(
                    scroll()
                        .w_full()
                        .max_h(move || px((window().height.0 - 156.0).max(160.0)))
                        .child(body),
                )
                .child(footer);

            overlay().inset_0().child(
                view()
                    .w_full()
                    .h_full()
                    .items_center()
                    .justify_center()
                    .child(backdrop)
                    .child(card),
            )
        },
    )
}
