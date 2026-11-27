//! Native modal rendering `onRun` script hook summary and declarative `view`.

#[allow(unused_imports)]
use super::*;

use crate::hook_spec::PreviewCardNode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HookRunModalData {
    pub(crate) hook_id: String,
    pub(crate) summary: String,
    pub(crate) cards: Vec<PreviewCardNode>,
    pub(crate) duration_ms: u64,
}

/// Modal dialog presenting declarative `onRun` script output triggered via Command Palette.
pub(crate) fn hook_run_modal(
    data: Signal<Option<HookRunModalData>>,
    selected: Signal<Option<u64>>,
    profiles: ProfileSignals,
) -> impl IntoElement {
    bind_escape_while(
        move || data.with(Option::is_some),
        nsh_core::keymap::When::Always,
        move || data.set(None),
    );
    let dismiss = popover_dismiss_context();
    effect(move || {
        if data.with(Option::is_some)
            && let Some(d) = dismiss
        {
            d.dismiss();
        }
    });

    when(
        move || data.with(Option::is_some),
        move || {
            let Some(current) = data.get() else {
                return view().into_element();
            };
            let backdrop = ui_modal_backdrop(move || data.set(None));
            let ctx = HookViewActionCtx::for_inspector(
                selected,
                profiles.filter,
                Some(profiles.filter_enabled),
            )
            .with_after_action(move || data.set(None));

            let header = view()
                .w_full()
                .px_4()
                .py_3()
                .flex_row()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_b_color(border_subtle())
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            text(tr("脚本运行报告", "Script Run Report"))
                                .text_size(px(TEXT_LG))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(fg_primary()),
                        )
                        .child(ui_badge(format!("@{}", current.hook_id), BadgeTone::Accent))
                        .child(
                            text(format!("{} ms", current.duration_ms))
                                .tabular_nums()
                                .text_size(px(TEXT_XS))
                                .text_color(fg_muted()),
                        ),
                )
                .child(ui_modal_close_button(move || data.set(None)));

            let mut body = uncull_nested_scroll(view()).w_full().p_4().gap_3();
            if !current.summary.trim().is_empty() {
                body = body.child(
                    view()
                        .w_full()
                        .p_2p5()
                        .rounded(px(RADIUS_MD))
                        .border_1()
                        .border_color(border_subtle())
                        .bg(bg_code())
                        .child(
                            text(current.summary)
                                .selectable(true)
                                .text_size(scaled_px(TEXT_SM))
                                .text_color(fg_primary()),
                        ),
                );
            }
            for node in current.cards {
                body = body.child(render_card_node(node, &ctx));
            }

            let footer = view()
                .w_full()
                .px_4()
                .py_2p5()
                .flex_row()
                .items_center()
                .justify_between()
                .border_t_1()
                .border_t_color(border_subtle())
                .child(
                    text(tr("按 Esc 关闭", "Press Esc to close"))
                        .text_size(px(TEXT_XS))
                        .text_color(fg_muted()),
                )
                .child(
                    button()
                        .h(px(ControlSize::Sm.height()))
                        .px_3()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .rounded(px(RADIUS_MD))
                        .border_1()
                        .border_color(accent_blue_border())
                        .bg(accent_blue_soft())
                        .hover(|s| s.bg(accent_blue_soft_hover()))
                        .active(|s| s.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .on_press(move |_: &()| data.set(None))
                        .child(control_text(tr("关闭", "Close")).text_color(accent_blue_text())),
                );

            let card = ui_modal_card()
                .w(move || px((window().width.0 - 32.0).clamp(360.0, 720.0)))
                .max_h(move || px((window().height.0 - 64.0).max(260.0)))
                .child(header)
                .child(
                    scroll()
                        .w_full()
                        .max_h(move || px((window().height.0 - 176.0).max(160.0)))
                        .child(body),
                )
                .child(footer);

            dialog()
                .animation(nsh_core::ModalAnimation::None)
                .close_on_escape(false)
                .optional_label(tr_opt("脚本运行报告", "Script Run Report"))
                .on_cancel(move || data.set(None))
                .child(
                    overlay().inset_0().on_cancel(move || data.set(None)).child(
                        view()
                            .w_full()
                            .h_full()
                            .items_center()
                            .justify_center()
                            .child(backdrop)
                            .child(card),
                    ),
                )
                .into_element()
        },
    )
}
