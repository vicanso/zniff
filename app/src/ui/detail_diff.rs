#[allow(unused_imports)]
use super::*;

pub(crate) fn diff_card(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    folded: Signal<u8>,
) -> impl IntoElement {
    let scope_sig = shown.diff_scope;
    let langs = ui_langs();
    let clear_controls = controls;
    view()
        .rounded(px(RADIUS_LG))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .overflow_hidden()
        .child(
            view()
                .flex_shrink_0()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .px_3p5()
                .py_2()
                .border_b(move || {
                    if card_folded(folded, FOLD_DIFF) {
                        px(0.0)
                    } else {
                        px(1.0)
                    }
                })
                .border_b_color(border_subtle())
                .child(
                    view()
                        .role(AccessibilityRole::Button)
                        .flex_1()
                        .min_w(px(120.0))
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .px_1()
                        .py_0p5()
                        .rounded(px(RADIUS_SM))
                        .hover(|style| style.bg(bg_control_hover()))
                        .active(|style| style.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .on_press(move || toggle_card(folded, FOLD_DIFF))
                        .child(fold_mark(folded, FOLD_DIFF))
                        .child(section_title(tr("对比", "Diff")))
                        .child(when(
                            move || !shown.diff_baseline_label.with(String::is_empty),
                            move || {
                                mono(move || -> SharedString {
                                    let lang = ui_lang(langs);
                                    let base = shown.diff_baseline_label.get();
                                    let base_prefix = t(lang, "基准", "Baseline");
                                    match selected.get() {
                                        Some(cur_id) => {
                                            let cur_prefix = t(lang, "当前", "Current");
                                            format!("{base_prefix} {base} · {cur_prefix} #{cur_id}")
                                                .into()
                                        }
                                        None => format!("{base_prefix} {base}").into(),
                                    }
                                })
                                .flex_1()
                                .min_w(px(0.0))
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Tail)
                                .text_size(px(TEXT_XS))
                                .text_color(fg_muted())
                            },
                        )),
                )
                .child(when(
                    move || !card_folded(folded, FOLD_DIFF),
                    move || {
                        let clear_controls = clear_controls.clone();
                        view()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .child(
                                view()
                                    .flex_row()
                                    .h(px(ControlSize::Xs.height()))
                                    .rounded(px(RADIUS_MD))
                                    .border_1()
                                    .border_color(border_control())
                                    .overflow_hidden()
                                    .child(msg_filter_chip(scope_sig, 0, "全部", "All"))
                                    .child(msg_filter_chip(scope_sig, 1, "仅正文", "Bodies"))
                                    .child(msg_filter_chip(scope_sig, 2, "仅头部", "Headers")),
                            )
                            .child(
                                ui_button(ButtonVariant::Outline, ControlSize::Xs)
                                    .accessibility_label(move || {
                                        Some(t(ui_lang(langs), "清除基准", "Clear baseline").into())
                                    })
                                    .on_press(move || {
                                        {
                                            let mut store = clear_controls
                                                .session_tabs
                                                .lock()
                                                .unwrap_or_else(|err| err.into_inner());
                                            store.clear_baseline();
                                        }
                                        baseline.set(None);
                                        set_status(&clear_controls, StatusMsg::BaselineCleared);
                                        publish_detail_for_controls(
                                            &shown,
                                            &clear_controls,
                                            selected.get_untracked(),
                                            ui_lang(langs),
                                            None,
                                        );
                                    })
                                    .child(
                                        control_text(tr("清除基准", "Clear baseline"))
                                            .text_color(fg_secondary()),
                                    ),
                            )
                    },
                )),
        )
        .child(when(
            move || !card_folded(folded, FOLD_DIFF),
            move || {
                when(move || shown.diff_loading.get(), detail_loading_box).otherwise(move || {
                    view()
                        .bg(bg_code())
                        .h(px(320.0))
                        .flex_col()
                        .overflow_hidden()
                        .child(diff_legend())
                        .child(
                            guard_culled_pointer_events(view())
                                .flex_1()
                                .min_h(px(0.0))
                                .overflow_hidden()
                                .child(
                                    diff_view()
                                        .old_value(move || SharedString::from(shown.diff_old.get()))
                                        .new_value(move || SharedString::from(shown.diff_new.get()))
                                        .layout(DiffLayout::Unified)
                                        .language("plaintext")
                                        .soft_wrap(true)
                                        .font_size(move || scaled_px(12.0))
                                        .line_height(move || scaled_px(18.0))
                                        .size_full(),
                                ),
                        )
                })
            },
        ))
}

pub(crate) fn diff_legend() -> impl IntoElement {
    view()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_3()
        .px_3()
        .py_1p5()
        .border_b_1()
        .border_b_color(border_subtle())
        .child(diff_key(baseline_mark(), tr("基准", "Baseline")))
        .child(diff_key(diff_current_mark(), tr("当前", "Current")))
}

pub(crate) fn diff_key(
    color: AdaptiveColor,
    label: impl Fn() -> String + 'static,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(
            view()
                .w(px(2.0))
                .h(px(12.0))
                .rounded(px(1.0))
                .flex_shrink_0()
                .bg(color),
        )
        .child(
            text(label)
                .number_of_lines(1)
                .text_size(px(TEXT_XS))
                .text_color(fg_muted()),
        )
}
