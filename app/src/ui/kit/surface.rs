//! Shared overlay surfaces, section headers, hairlines, and semantic badges.

#[allow(unused_imports)]
use super::super::*;

use nsh_core::element::StyleRefinement;

use super::tokens::{BadgeTone, MOTION_FAST_MS, RADIUS_LG, RADIUS_MD, RADIUS_SM, TEXT_MD, TEXT_XS};

/// Shared popup container treatment for every `Popover` and `Menu`.
#[allow(dead_code)]
pub(crate) fn ui_popup_style(s: StyleRefinement) -> StyleRefinement {
    s.rounded(px(RADIUS_LG))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_lg()
        .overflow_hidden()
        .transition_opacity()
        .duration(MOTION_FAST_MS)
        .ease_out()
}

/// Shared popup reveal starting state honoring reduced-motion preferences.
#[allow(dead_code)]
pub(crate) fn ui_popup_starting_style(s: StyleRefinement) -> StyleRefinement {
    if reduce_motion() { s } else { s.opacity(0.0) }
}

/// Shared surface for anchored tooltips.
#[allow(dead_code)]
pub(crate) fn ui_tooltip_surface(content: impl IntoElement) -> View {
    view()
        .max_w(px(420.0))
        .px_2p5()
        .py_1p5()
        .rounded(px(RADIUS_MD))
        .bg(bg_card())
        .border_1()
        .border_color(border_strong())
        .shadow_md()
        .child(content)
}

/// Semantic section heading inside panels and popovers.
#[allow(dead_code)]
pub(crate) fn ui_section_header(title: impl IntoReactive<SharedString>) -> Text {
    text(title)
        .text_size(px(TEXT_MD))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg_primary())
}

/// Single 1px horizontal divider owned by the enclosing container.
#[allow(dead_code)]
pub(crate) fn ui_divider() -> View {
    view().w_full().h(px(1.0)).bg(border_strong())
}

/// Compact semantic badge with restrained neutral or status coloring.
#[allow(dead_code)]
pub(crate) fn ui_badge(label: impl IntoReactive<SharedString>, tone: BadgeTone) -> View {
    let (bg, fg) = match tone {
        BadgeTone::Neutral => (bg_control(), fg_secondary()),
        BadgeTone::Accent => (accent_blue_soft(), accent_blue_text()),
        BadgeTone::Ok => (status_ok_bg(), status_ok_fg()),
        BadgeTone::Warn => (status_warn_bg(), status_warn_fg()),
        BadgeTone::Err => (status_err_bg(), status_err_fg()),
    };
    view()
        .px_1p5()
        .py(px(1.0))
        .flex_shrink_0()
        .rounded(px(RADIUS_SM))
        .bg(bg)
        .child(
            text(label)
                .number_of_lines(1)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg),
        )
}
