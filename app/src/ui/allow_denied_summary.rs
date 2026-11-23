//! Shared summary of requests blocked by `allow_hosts`, with reset and one-click allow actions.

#[allow(unused_imports)]
use super::*;

const VISIBLE_DENIED_HOSTS: usize = 10;

/// Displays the live count of `allow_hosts`-blocked requests, top denied hosts, a Reset button,
/// and (when not locked by CLI/env) a one-click `+ Allow` button per host.
pub(crate) fn allow_denied_summary_section(
    denied: Signal<allow_hosts::AllowDeniedSummary>,
    allow_hosts: Signal<String>,
    allow_hosts_locked: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    when(
        move || denied.with(|summary| summary.total > 0),
        move || {
            let header_controls = controls.clone();
            let list_controls = controls.clone();
            view()
                .w_full()
                .p_2()
                .gap_1p5()
                .rounded(px(6.0))
                .border_1()
                .border_color(status_warn_fg())
                .bg(status_warn_bg())
                .child(denied_summary_header(denied, header_controls))
                .child(denied_hosts_list(
                    denied,
                    allow_hosts,
                    allow_hosts_locked,
                    list_controls,
                ))
                .child(denied_overflow_note(denied))
        },
    )
}

fn denied_summary_header(
    denied: Signal<allow_hosts::AllowDeniedSummary>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let langs = ui_langs();
    view()
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(glyph_px(12.0, || Glyph::Alert, || color(status_warn_fg())))
                .child(
                    text(move || -> SharedString {
                        let total = present::format_thousands(denied.with(|summary| summary.total));
                        match ui_lang(langs) {
                            Lang::Zh => format!("已拦截 {total} 次请求").into(),
                            Lang::En => format!("{total} requests denied").into(),
                        }
                    })
                    .tabular_nums()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(status_warn_fg()),
                ),
        )
        .child(
            view()
                .h(px(20.0))
                .px_1p5()
                .rounded(px(4.0))
                .border_1()
                .border_color(border_control())
                .bg(bg_card())
                .hover(|s| s.bg(bg_control_hover()))
                .active(|s| s.opacity(0.8))
                .cursor(Cursor::Pointer)
                .items_center()
                .justify_center()
                .on_press(move || {
                    controls.hub.reset_allow_denied();
                    denied.set(controls.hub.allow_denied_summary());
                })
                .child(
                    text(tr("清零", "Reset"))
                        .text_size(px(10.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg_secondary()),
                ),
        )
}

fn denied_hosts_list(
    denied: Signal<allow_hosts::AllowDeniedSummary>,
    allow_hosts: Signal<String>,
    allow_hosts_locked: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    view().w_full().gap_1().child(each(
        move || {
            denied.with(|summary| {
                summary
                    .hosts
                    .iter()
                    .take(VISIBLE_DENIED_HOSTS)
                    .cloned()
                    .collect::<Vec<_>>()
            })
        },
        |(host, _)| host.clone(),
        move |entry, _| denied_host_row(entry, allow_hosts, allow_hosts_locked, controls.clone()),
    ))
}

fn denied_host_row(
    entry: ReadSignal<(String, u64)>,
    allow_hosts: Signal<String>,
    allow_hosts_locked: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    view()
        .w_full()
        .min_h(px(22.0))
        .px_1p5()
        .py_0p5()
        .rounded(px(4.0))
        .bg(bg_card())
        .flex_row()
        .items_center()
        .gap_1p5()
        .child(
            mono(move || entry.with(|(host, _)| host.clone()))
                .flex_1()
                .min_w(px(0.0))
                .number_of_lines(1)
                .text_overflow(EllipsizeMode::Middle)
                .text_size(px(11.0))
                .text_color(fg_primary()),
        )
        .child(
            mono(move || {
                entry.with(|(_, count)| format!("×{}", present::format_thousands(*count)))
            })
            .flex_shrink_0()
            .tabular_nums()
            .text_size(px(10.5))
            .text_color(fg_muted()),
        )
        .child(when(
            move || !allow_hosts_locked.get(),
            move || {
                let row_controls = controls.clone();
                view()
                    .h(px(18.0))
                    .px_1p5()
                    .flex_shrink_0()
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(accent_blue_border())
                    .bg(accent_blue_soft())
                    .hover(|s| s.bg(accent_blue_soft_hover()))
                    .active(|s| s.opacity(0.8))
                    .cursor(Cursor::Pointer)
                    .items_center()
                    .justify_center()
                    .on_press(move || {
                        let host = entry.with_untracked(|(host, _)| host.clone());
                        let next =
                            allow_hosts::append_allow_host(&allow_hosts.get_untracked(), &host);
                        apply_allow_hosts_change(&row_controls, allow_hosts, &next);
                    })
                    .child(
                        text(tr("+ 允许", "+ Allow"))
                            .number_of_lines(1)
                            .text_size(px(10.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(accent_blue_text()),
                    )
            },
        ))
}

fn denied_overflow_note(denied: Signal<allow_hosts::AllowDeniedSummary>) -> impl IntoElement {
    let langs = ui_langs();
    when(
        move || denied.with(|summary| summary.hosts.len() > VISIBLE_DENIED_HOSTS),
        move || {
            text(move || -> SharedString {
                let extra = present::format_thousands(
                    denied.with(|s| s.hosts.len().saturating_sub(VISIBLE_DENIED_HOSTS)) as u64,
                );
                match ui_lang(langs) {
                    Lang::Zh => format!("另有 {extra} 个被拦截域名未展开").into(),
                    Lang::En => format!("+{extra} more denied hosts").into(),
                }
            })
            .text_size(px(10.5))
            .text_color(fg_muted())
        },
    )
}
