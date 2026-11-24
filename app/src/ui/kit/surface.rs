//! Shared overlay surfaces, section headers, hairlines, and semantic badges.

#[allow(unused_imports)]
use super::super::*;

use nsh_core::element::StyleRefinement;

use super::tokens::{
    BadgeTone, ControlSize, MOTION_FAST_MS, MOTION_INSTANT_MS, RADIUS_LG, RADIUS_MD, RADIUS_SM,
    TEXT_MD, TEXT_SM, TEXT_XS,
};

/// Full-viewport modal backdrop scrim that dismisses on press.
#[allow(dead_code)]
pub(crate) fn ui_modal_backdrop(on_close: impl Fn() + 'static) -> View {
    view()
        .absolute()
        .inset_0()
        .bg(backdrop_scrim())
        .on_press(on_close)
}

/// Shared modal dialog card chrome.
#[allow(dead_code)]
pub(crate) fn ui_modal_card() -> View {
    view()
        .flex_col()
        .overflow_hidden()
        .rounded(px(RADIUS_LG))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_lg()
}

/// Shared accessible icon close button for modal dialog headers.
#[allow(dead_code)]
pub(crate) fn ui_modal_close_button(on_close: impl Fn() + 'static) -> impl IntoElement {
    view()
        .role(AccessibilityRole::Button)
        .w(px(24.0))
        .h(px(24.0))
        .rounded(px(RADIUS_MD))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_center()
        .hover(|s| s.bg(bg_control_hover()))
        .active(|s| s.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .label(tr("关闭", "Close"))
        .on_press(on_close)
        .child(glyph_px(12.0, || Glyph::Close, || color(fg_muted())))
}

/// Full-width semantic toggle switch row built on `nsh_base::switch()`.
#[allow(dead_code)]
pub(crate) fn ui_switch_row(
    label: impl IntoReactive<SharedString>,
    checked: impl Fn() -> bool + 'static + Clone,
    on_toggle: impl Fn(bool) + 'static,
) -> impl IntoElement {
    let checked_text = checked.clone();
    switch()
        .w_full()
        .min_h(px(ControlSize::Md.height()))
        .px_2p5()
        .py_1()
        .gap_2p5()
        .rounded(px(RADIUS_MD))
        .flex_row()
        .items_center()
        .hover(|s| s.bg(bg_control_hover()))
        .active(|s| s.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .checked(checked)
        .on_change(move |next: &bool| on_toggle(*next))
        .track_style(|s| {
            s.w(px(28.0))
                .h(px(16.0))
                .p(px(2.0))
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .rounded_full()
                .bg(switch_track_off())
                .checked(|c| c.bg(accent_blue()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
        })
        .thumb_style(|s| {
            s.w(px(12.0))
                .h(px(12.0))
                .rounded_full()
                .bg(switch_thumb())
                .checked(|c| c.translate_x(px(12.0)))
                .transition_transform()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
        })
        .child(
            text(label)
                .flex_1()
                .min_w(px(0.0))
                .text_size(px(TEXT_SM))
                .text_color(move || {
                    if checked_text() {
                        fg_primary()
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

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

/// Broadcast handle that asks all mounted toolbar/footer popovers to close when focus enters an
/// outside native input/editor (whose mouse-down events AppKit consumes before `on_press_outside`).
#[derive(Clone, Copy)]
pub(crate) struct PopoverDismiss(Signal<u64>);

impl PopoverDismiss {
    /// Dismisses any currently open popover listening on this bus.
    pub(crate) fn dismiss(self) {
        self.0.update(|n| *n = n.wrapping_add(1));
    }
}

/// Provides the root-level [`PopoverDismiss`] bus to descendant UI components.
#[allow(dead_code)]
pub(crate) fn provide_popover_dismiss() -> PopoverDismiss {
    let handle = PopoverDismiss(signal(0));
    provide_context(handle);
    handle
}

/// Returns the root [`PopoverDismiss`] bus when mounted under [`App`], or `None` in isolated tests.
pub(crate) fn popover_dismiss_context() -> Option<PopoverDismiss> {
    use_context::<PopoverDismiss>()
}

/// Connects a popover's `open` signal to the outside-input dismiss bus and clears any previously
/// focused workspace input when opening so clicking back into that input triggers `on_focus_in`.
pub(crate) fn bind_popover_dismiss(open: Signal<bool>) {
    let was_open = Cell::new(open.get_untracked());
    effect(move || {
        let now = open.get();
        if now && !was_open.replace(true) {
            dismiss_keyboard();
        } else if !now {
            was_open.set(false);
        }
    });
    if let Some(PopoverDismiss(rev)) = popover_dismiss_context() {
        let seen = Cell::new(rev.get_untracked());
        effect(move || {
            let cur = rev.get();
            if cur != seen.replace(cur) && open.get_untracked() {
                open.set(false);
            }
        });
    }
}
