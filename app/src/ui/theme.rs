#[allow(unused_imports)]
use super::*;

pub(crate) fn bg_canvas() -> AdaptiveColor {
    AdaptiveColor::rgb(0xf8fafc, 0x0d1017)
}

pub(crate) fn bg_chrome() -> AdaptiveColor {
    AdaptiveColor::rgb(0xffffff, 0x131822)
}

pub(crate) fn bg_list() -> AdaptiveColor {
    AdaptiveColor::rgb(0xffffff, 0x10141d)
}

pub(crate) fn bg_card() -> AdaptiveColor {
    AdaptiveColor::rgb(0xffffff, 0x131822)
}

pub(crate) fn bg_code() -> AdaptiveColor {
    AdaptiveColor::rgb(0xf8fafc, 0x0f131b)
}

pub(crate) fn bg_control() -> AdaptiveColor {
    AdaptiveColor::rgb(0xf1f5f9, 0x1c2331)
}

pub(crate) fn bg_control_hover() -> AdaptiveColor {
    AdaptiveColor::rgb(0xe2e8f0, 0x252e40)
}

pub(crate) fn border_strong() -> AdaptiveColor {
    AdaptiveColor::rgb(0xe2e8f0, 0x222938)
}

pub(crate) fn border_subtle() -> AdaptiveColor {
    AdaptiveColor::rgb(0xf1f5f9, 0x19202e)
}

pub(crate) fn border_control() -> AdaptiveColor {
    AdaptiveColor::rgb(0xcbd5e1, 0x2b3448)
}

pub(crate) fn fg_primary() -> AdaptiveColor {
    AdaptiveColor::rgb(0x0f172a, 0xf1f5f9)
}

pub(crate) fn fg_secondary() -> AdaptiveColor {
    AdaptiveColor::rgb(0x334155, 0xcbd5e1)
}

pub(crate) fn fg_muted() -> AdaptiveColor {
    AdaptiveColor::rgb(0x526075, 0x8e9bb3)
}

pub(crate) fn accent_blue() -> AdaptiveColor {
    AdaptiveColor::rgb(0x2563eb, 0x2563eb)
}

pub(crate) fn accent_blue_hover() -> AdaptiveColor {
    AdaptiveColor::rgb(0x1d4ed8, 0x1d4ed8)
}

pub(crate) fn accent_blue_soft() -> AdaptiveColor {
    AdaptiveColor::rgb(0xdbeafe, 0x172554)
}

pub(crate) fn accent_blue_soft_hover() -> AdaptiveColor {
    AdaptiveColor::rgb(0xbfdbfe, 0x1e3a8a)
}

pub(crate) fn accent_blue_border() -> AdaptiveColor {
    AdaptiveColor::rgb(0x93c5fd, 0x1e40af)
}

pub(crate) fn accent_blue_text() -> AdaptiveColor {
    AdaptiveColor::rgb(0x1d4ed8, 0x93c5fd)
}

/// The baseline row's leading bar. Same red as the diff view's old side.
pub(crate) fn baseline_mark() -> AdaptiveColor {
    AdaptiveColor::rgb(0xdc2626, 0xf87171)
}

/// The current row in a diff. Same green as the diff view's new side.
pub(crate) fn diff_current_mark() -> AdaptiveColor {
    AdaptiveColor::rgb(0x16a34a, 0x4ade80)
}

pub(crate) fn status_ok_fg() -> AdaptiveColor {
    AdaptiveColor::rgb(0x047857, 0x34d399)
}

pub(crate) fn status_ok_bg() -> AdaptiveColor {
    AdaptiveColor::rgb(0xd1fae5, 0x0a2e26)
}

pub(crate) fn status_ok_bg_hover() -> AdaptiveColor {
    AdaptiveColor::rgb(0xc6f6e2, 0x0f3d33)
}

pub(crate) fn status_warn_fg() -> AdaptiveColor {
    AdaptiveColor::rgb(0xb45309, 0xfbbf24)
}

pub(crate) fn status_warn_bg() -> AdaptiveColor {
    AdaptiveColor::rgb(0xfef3c7, 0x33240b)
}

pub(crate) fn status_err_fg() -> AdaptiveColor {
    AdaptiveColor::rgb(0x9f1239, 0xfb7185)
}

pub(crate) fn status_err_bg() -> AdaptiveColor {
    AdaptiveColor::rgb(0xffe4e6, 0x361320)
}

pub(crate) fn status_err_bg_hover() -> AdaptiveColor {
    AdaptiveColor::rgb(0xfecdd3, 0x4c1d2e)
}

pub(crate) fn danger_button_bg() -> AdaptiveColor {
    AdaptiveColor::rgb(0xdc2626, 0xe11d48)
}

pub(crate) fn danger_button_hover() -> AdaptiveColor {
    AdaptiveColor::rgb(0xb91c1c, 0xbe123c)
}

pub(crate) fn method_fg(method: &str) -> AdaptiveColor {
    match method {
        "GET" => AdaptiveColor::rgb(0x0369a1, 0x38bdf8),
        "POST" => AdaptiveColor::rgb(0x047857, 0x34d399),
        "PUT" | "PATCH" => AdaptiveColor::rgb(0xb45309, 0xfbbf24),
        "DELETE" => AdaptiveColor::rgb(0x9f1239, 0xfb7185),
        "CONNECT" => AdaptiveColor::rgb(0x6d28d9, 0xa78bfa),
        _ => fg_secondary(),
    }
}

pub(crate) fn method_bg(method: &str) -> AdaptiveColor {
    match method {
        "GET" => AdaptiveColor::rgb(0xe0f2fe, 0x0c2a42),
        "POST" => AdaptiveColor::rgb(0xd1fae5, 0x0a2e26),
        "PUT" | "PATCH" => AdaptiveColor::rgb(0xfef3c7, 0x33240b),
        "DELETE" => AdaptiveColor::rgb(0xffe4e6, 0x361320),
        "CONNECT" => AdaptiveColor::rgb(0xede9fe, 0x241842),
        _ => bg_control(),
    }
}

pub(crate) fn http_status_fg(status: Option<u16>, failed: bool) -> AdaptiveColor {
    match status {
        _ if failed => status_err_fg(),
        Some(code) if code >= 400 => status_err_fg(),
        Some(code) if (300..400).contains(&code) => status_warn_fg(),
        Some(code) if (200..300).contains(&code) => status_ok_fg(),
        _ => fg_muted(),
    }
}

pub(crate) fn http_status_bg(status: Option<u16>, failed: bool) -> AdaptiveColor {
    match status {
        _ if failed => status_err_bg(),
        Some(code) if code >= 400 => status_err_bg(),
        Some(code) if (300..400).contains(&code) => status_warn_bg(),
        Some(code) if (200..300).contains(&code) => status_ok_bg(),
        _ => bg_control(),
    }
}
