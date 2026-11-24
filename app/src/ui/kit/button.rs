//! Styled interactive button primitives built on `nsh_base::button`.

#[allow(unused_imports)]
use super::super::*;

use super::tokens::{ButtonVariant, ControlSize, MOTION_INSTANT_MS};

/// Wraps a title-bar interactive control so presses do not trigger window dragging.
pub(crate) fn no_drag(child: impl IntoElement) -> View {
    view().window_drag(false).flex_shrink_0().child(child)
}

/// Standard semantic button styled strictly from theme tokens and `ControlSize`.
/// Uses the desktop default arrow cursor (non-link actions never set `Cursor::Pointer`).
#[allow(dead_code)]
pub(crate) fn ui_button(variant: ButtonVariant, size: ControlSize) -> nsh_base::ButtonElement {
    let base = nsh_base::button()
        .h(px(size.height()))
        .px(px(size.padding_x()))
        .gap(px(size.gap()))
        .rounded(px(size.radius()))
        .flex_row()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .disabled_style(|s| s.opacity(0.45));

    match variant {
        ButtonVariant::Primary => base
            .bg(accent_blue())
            .hover(|s| s.bg(accent_blue_hover()))
            .active(|s| s.bg(accent_blue_active())),
        ButtonVariant::Secondary => base
            .bg(bg_control())
            .hover(|s| s.bg(bg_control_hover()))
            .active(|s| s.bg(bg_control_active())),
        ButtonVariant::Outline => base
            .border_1()
            .border_color(border_control())
            .bg(bg_card())
            .hover(|s| s.bg(bg_control_hover()))
            .active(|s| s.bg(bg_control_active())),
        ButtonVariant::Ghost => base
            .bg(AdaptiveColor::TRANSPARENT)
            .hover(|s| s.bg(bg_control_hover()))
            .active(|s| s.bg(bg_control_active())),
        ButtonVariant::Danger => base
            .bg(danger_button_bg())
            .hover(|s| s.bg(danger_button_hover()))
            .active(|s| s.bg(danger_button_active())),
    }
}

/// Square icon-only button whose dimensions lock to `size.height()`.
#[allow(dead_code)]
pub(crate) fn ui_icon_button(variant: ButtonVariant, size: ControlSize) -> nsh_base::ButtonElement {
    let base = nsh_base::button()
        .w(px(size.height()))
        .h(px(size.height()))
        .rounded(px(size.radius()))
        .flex_row()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .disabled_style(|s| s.opacity(0.45));

    match variant {
        ButtonVariant::Primary => base
            .bg(accent_blue())
            .hover(|s| s.bg(accent_blue_hover()))
            .active(|s| s.bg(accent_blue_active())),
        ButtonVariant::Secondary => base
            .bg(bg_control())
            .hover(|s| s.bg(bg_control_hover()))
            .active(|s| s.bg(bg_control_active())),
        ButtonVariant::Outline => base
            .border_1()
            .border_color(border_control())
            .bg(bg_card())
            .hover(|s| s.bg(bg_control_hover()))
            .active(|s| s.bg(bg_control_active())),
        ButtonVariant::Ghost => base
            .bg(AdaptiveColor::TRANSPARENT)
            .hover(|s| s.bg(bg_control_hover()))
            .active(|s| s.bg(bg_control_active())),
        ButtonVariant::Danger => base
            .bg(danger_button_bg())
            .hover(|s| s.bg(danger_button_hover()))
            .active(|s| s.bg(danger_button_active())),
    }
}

/// Ghost button at rest that switches to accent tint when active or owning an open popup.
#[allow(dead_code)]
pub(crate) fn ui_toggle_button(
    size: ControlSize,
    active: impl Fn() -> bool + Copy + 'static,
) -> nsh_base::ButtonElement {
    nsh_base::button()
        .selected(active)
        .h(px(size.height()))
        .px(px(size.padding_x()))
        .gap(px(size.gap()))
        .rounded(px(size.radius()))
        .flex_row()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .border_1()
        .border_color(move || {
            if active() {
                accent_blue_border()
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .bg(move || {
            if active() {
                accent_blue_soft()
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .hover(move |s| {
            s.bg(move || {
                if active() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|s| s.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .disabled_style(|s| s.opacity(0.45))
}
