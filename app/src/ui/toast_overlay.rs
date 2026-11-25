#[allow(unused_imports)]
use super::*;

pub(crate) fn toast_overlay(
    toast_shown: Signal<Vec<(u64, StatusMsg, ToastTransitionStatus)>>,
    toast_overlay_gen: Signal<u64>,
    toast_state: ToastStackState,
    toast_manager: Stored<ToastManager<u64, StatusMsg>>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    each(
        move || {
            if toast_shown.with(Vec::is_empty) {
                Vec::new()
            } else {
                vec![toast_overlay_gen.get()]
            }
        },
        |id| *id,
        move |_, _| {
            let controls = controls.clone();
            overlay()
                .inset_0()
                .flex_col()
                .items_end()
                .justify_end()
                .pb(px(40.0))
                .pr(px(14.0))
                .pointer_events(PointerEvents::None)
                .child(
                    toast_stack()
                        .placement(Anchor::BottomRight)
                        .state(toast_state)
                        .w(move || px((window().width.0 - 24.0).clamp(220.0, 380.0)))
                        .max_w(move || px((window().width.0 - 24.0).max(200.0)))
                        .pointer_events(PointerEvents::Auto)
                        .child(each(
                            toast_shown,
                            |(id, _, _)| *id,
                            move |item, _| {
                                toast_card(item, toast_manager, toast_shown, controls.clone())
                            },
                        )),
                )
        },
    )
}

fn toast_card(
    item: ReadSignal<(u64, StatusMsg, ToastTransitionStatus)>,
    toast_manager: Stored<ToastManager<u64, StatusMsg>>,
    toast_shown: Signal<Vec<(u64, StatusMsg, ToastTransitionStatus)>>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let id = item.with(|(id, _, _)| *id);
    let is_clear_undo = item.with(|(_, msg, _)| matches!(msg, StatusMsg::ClearedFlows { .. }));
    let dismiss_one = move || {
        let now = nsh_core::time::now();
        let next = toast_manager.with(|m| {
            m.dismiss(&id, now);
            m.visible(3)
                .map(|(id, data, status)| (*id, data.clone(), status))
                .collect::<Vec<_>>()
        });
        toast_shown.set(next);
    };
    let mut row = view()
        .w_full()
        .px_3p5()
        .py_3()
        .flex_row()
        .items_center()
        .gap_3()
        .child(toast_content(item, dismiss_one));
    if is_clear_undo {
        row = row.child(undo_clear_button(controls, dismiss_one));
    }
    row = row.child(toast_close_button(dismiss_one));

    toast()
        .transition_status(move || item.with(|(_, _, s)| *s))
        .on_dismiss_request(dismiss_one)
        .rounded(px(RADIUS_LG))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_lg()
        .transition_all()
        .duration(if reduce_motion() { 0.0 } else { MOTION_FAST_MS })
        .starting_class(if reduce_motion() {
            ""
        } else {
            "opacity-0 translate-y-2"
        })
        .ending_class(if reduce_motion() {
            ""
        } else {
            "opacity-0 scale-95"
        })
        .child(row)
}

fn toast_content(
    item: ReadSignal<(u64, StatusMsg, ToastTransitionStatus)>,
    on_dismiss: impl Fn() + Copy + 'static,
) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .gap_3()
        .on_press(on_dismiss)
        .child(
            view()
                .w(px(ControlSize::Md.height()))
                .h(px(ControlSize::Md.height()))
                .rounded(px(RADIUS_MD))
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .justify_center()
                .bg(move || {
                    if item.with(|(_, msg, _)| msg.toast_kind() == Some(true)) {
                        status_warn_bg()
                    } else {
                        status_ok_bg()
                    }
                })
                .child(glyph_px(
                    16.0,
                    move || {
                        if item.with(|(_, msg, _)| msg.toast_kind() == Some(true)) {
                            Glyph::Alert
                        } else {
                            Glyph::Check
                        }
                    },
                    move || {
                        if item.with(|(_, msg, _)| msg.toast_kind() == Some(true)) {
                            color(status_warn_fg())
                        } else {
                            color(status_ok_fg())
                        }
                    },
                )),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_col()
                .gap(px(2.0))
                .child(
                    text(move || {
                        let lang = ui_lang(ui_langs());
                        item.with(|(_, msg, _)| msg.toast_title(lang).to_string())
                    })
                    .text_size(px(TEXT_MD))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(fg_primary()),
                )
                .child(
                    text(move || {
                        let lang = ui_lang(ui_langs());
                        item.with(|(_, msg, _)| msg.text(lang))
                    })
                    .number_of_lines(3)
                    .text_overflow(EllipsizeMode::Middle)
                    .text_size(px(TEXT_SM))
                    .text_color(fg_secondary()),
                ),
        )
}

fn undo_clear_button(
    controls: Arc<Controls>,
    on_dismiss: impl Fn() + Copy + 'static,
) -> impl IntoElement {
    button()
        .px_2p5()
        .py_1()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(accent_blue_border())
        .bg(accent_blue_soft())
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_center()
        .hover(|s| s.bg(accent_blue_soft_hover()))
        .active(|s| s.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .on_press(move |_: &()| {
            undo_clear_captured(&controls);
            on_dismiss();
        })
        .child(
            text(tr("撤销", "Undo"))
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(accent_blue_text()),
        )
}

fn toast_close_button(on_dismiss: impl Fn() + Copy + 'static) -> impl IntoElement {
    button()
        .w(px(20.0))
        .h(px(20.0))
        .rounded(px(RADIUS_SM))
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
        .accessibility_label(tr_opt("关闭", "Dismiss"))
        .on_press(move |_: &()| on_dismiss())
        .child(glyph_px(11.0, || Glyph::Close, || color(fg_muted())))
}
