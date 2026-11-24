//! Styled interactive button primitives built on `nsh_base::button`.

#[allow(unused_imports)]
use super::super::*;

use super::tokens::{
    ButtonVariant, ControlSize, MOTION_INSTANT_MS, MOTION_PRESS_MS, PRESS_SCALE, PRESS_SCALE_ICON,
    motion_ms,
};

fn active_press_scale(scale: f32) -> f32 {
    if reduce_motion() { 1.0 } else { scale }
}

fn button_variant_colors(
    variant: ButtonVariant,
) -> (
    AdaptiveColor,
    AdaptiveColor,
    AdaptiveColor,
    Option<AdaptiveColor>,
) {
    match variant {
        ButtonVariant::Primary => (
            accent_blue(),
            accent_blue_hover(),
            accent_blue_active(),
            None,
        ),
        ButtonVariant::Secondary => (bg_control(), bg_control_hover(), bg_control_active(), None),
        ButtonVariant::Outline => (
            bg_card(),
            bg_control_hover(),
            bg_control_active(),
            Some(border_control()),
        ),
        ButtonVariant::Ghost => (
            AdaptiveColor::TRANSPARENT,
            bg_control_hover(),
            bg_control_active(),
            None,
        ),
        ButtonVariant::Danger => (
            danger_button_bg(),
            danger_button_hover(),
            danger_button_active(),
            None,
        ),
    }
}

fn style_variant_button(
    base: nsh_base::ButtonElement,
    variant: ButtonVariant,
    press_scale: f32,
) -> nsh_base::ButtonElement {
    let (bg, hover_bg, active_bg, border) = button_variant_colors(variant);
    let base = if let Some(border_col) = border {
        base.border_1().border_color(border_col)
    } else {
        base
    };
    base.bg(bg).hover(move |s| s.bg(hover_bg)).active(move |s| {
        s.bg(active_bg)
            .duration(move || motion_ms(MOTION_PRESS_MS))
            .scale(move || active_press_scale(press_scale))
    })
}

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
        .transition()
        .duration(move || motion_ms(MOTION_INSTANT_MS))
        .ease_bezier(0.22, 1.0, 0.36, 1.0)
        .focus_visible(apply_focus_ring)
        .disabled_style(|s| s.opacity(0.45));

    style_variant_button(base, variant, PRESS_SCALE)
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
        .transition()
        .duration(move || motion_ms(MOTION_INSTANT_MS))
        .ease_bezier(0.22, 1.0, 0.36, 1.0)
        .focus_visible(apply_focus_ring)
        .disabled_style(|s| s.opacity(0.45));

    style_variant_button(base, variant, PRESS_SCALE_ICON)
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
        .active(|s| {
            s.bg(bg_control_active())
                .duration(move || motion_ms(MOTION_PRESS_MS))
                .scale(move || active_press_scale(PRESS_SCALE))
        })
        .transition()
        .duration(move || motion_ms(MOTION_INSTANT_MS))
        .ease_bezier(0.22, 1.0, 0.36, 1.0)
        .focus_visible(apply_focus_ring)
        .disabled_style(|s| s.opacity(0.45))
}
