#[allow(unused_imports)]
use super::*;

pub(crate) fn flow_cell(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    columns: Signal<ListColumns>,
    hover: CellHover,
    cell: &DataTableCell,
) -> AnyElement {
    let index = cell.row as usize;
    match cell.key.as_ref() {
        "id" => id_cell(rows, selected, baseline, columns, index).into_element(),
        "method" => method_cell(rows, selected, baseline, columns, index).into_element(),
        "path" => path_cell(rows, selected, baseline, columns, hover, index).into_element(),
        "status" => status_cell(rows, selected, baseline, columns, index).into_element(),
        "protocol" => protocol_cell(rows, selected, baseline, columns, index).into_element(),
        "gap" => time_cell(rows, selected, baseline, columns, hover, index).into_element(),
        "elapsed" => elapsed_cell(rows, selected, baseline, columns, index).into_element(),
        "host" => host_cell(rows, selected, baseline, columns, hover, index).into_element(),
        "client_ip" => {
            client_ip_cell(rows, selected, baseline, columns, hover, index).into_element()
        }
        "size" => size_cell(rows, selected, baseline, columns, index).into_element(),
        _ => view().into_element(),
    }
}

pub(crate) fn column_pad_right(
    column: Option<ListColumn>,
    align_right: bool,
    columns: ListColumns,
) -> Points {
    let base = match column {
        Some(ListColumn::Method) => 2.0,
        _ if align_right => 6.0,
        _ => 4.0,
    };
    if column.is_some() && columns.last_visible() == column {
        px(base + 8.0)
    } else {
        px(base)
    }
}

pub(crate) fn flow_header(
    cell: &DataTableHeaderCell,
    columns: Signal<ListColumns>,
    sort: Signal<Option<ListSort>>,
) -> View {
    let key = cell.key.to_string();
    let column = ListColumn::from_key(&key);
    let align_right = matches!(key.as_str(), "gap" | "elapsed" | "size");
    let pad_left = match key.as_str() {
        "method" => 14.0,
        "id" => 6.0,
        _ => 4.0,
    };
    let is_sorted =
        move || column.is_some_and(|col| sort.get().is_some_and(|current| current.column == col));
    let row = view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(px(pad_left))
        .pr(move || column_pad_right(column, align_right, columns.get()))
        .gap_0p5()
        .flex_row()
        .items_center()
        .cursor(Cursor::Pointer)
        .overflow_hidden();
    let row = if align_right { row.justify_end() } else { row };
    row.child(
        text(move || -> SharedString { flow_column_title(&key, columns.get().full_url).into() })
            .number_of_lines(1)
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || {
                if is_sorted() {
                    accent_blue_text()
                } else {
                    fg_muted()
                }
            }),
    )
    .child(when(is_sorted, move || {
        glyph_px(
            10.5,
            move || {
                if sort.get().is_some_and(|current| current.desc) {
                    Glyph::Chevron
                } else {
                    Glyph::ChevronUp
                }
            },
            move || color(accent_blue_text()),
        )
    }))
}

pub(crate) fn row_edge_mark(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    index: usize,
) -> AdaptiveColor {
    let id = rows.with(|rows| rows.get(index).map(|row| row.id));
    match id {
        Some(id) if selected.get() == Some(id) => accent_blue(),
        Some(id) if baseline.get() == Some(id) => baseline_mark(),
        _ => AdaptiveColor::TRANSPARENT,
    }
}

pub(crate) fn method_cell(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    columns: Signal<ListColumns>,
    index: usize,
) -> impl IntoElement {
    view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(px(2.0))
        .pr(move || column_pad_right(Some(ListColumn::Method), false, columns.get()))
        .border_l(px(2.0))
        .border_l_color(move || {
            if columns.get().first_visible() == Some(ListColumn::Method) {
                row_edge_mark(rows, selected, baseline, index)
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .flex_row()
        .items_center()
        .overflow_hidden()
        .child(
            view()
                .w(px(10.0))
                .h_full()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .justify_center()
                .child(when(
                    move || rows.with(|rows| rows.get(index).is_some_and(|row| row.pinned)),
                    || {
                        view()
                            .w(px(6.0))
                            .h(px(6.0))
                            .rounded_full()
                            .flex_shrink_0()
                            .bg(accent_blue())
                    },
                )),
        )
        .child(
            view()
                .px(px(5.0))
                .py_0p5()
                .rounded(px(4.0))
                .bg(move || {
                    rows.with(|rows| {
                        rows.get(index)
                            .map(|row| method_bg(&row.method))
                            .unwrap_or_else(bg_control)
                    })
                })
                .child(
                    mono(move || {
                        rows.with(|rows| {
                            rows.get(index)
                                .map(|row| row.method.clone())
                                .unwrap_or_default()
                        })
                    })
                    .number_of_lines(1)
                    .text_size(px(10.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(move || {
                        rows.with(|rows| {
                            rows.get(index)
                                .map(|row| method_fg(&row.method))
                                .unwrap_or_else(fg_secondary)
                        })
                    }),
                ),
        )
}

pub(crate) fn id_cell(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    columns: Signal<ListColumns>,
    index: usize,
) -> impl IntoElement {
    view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(px(4.0))
        .pr(move || column_pad_right(Some(ListColumn::Id), false, columns.get()))
        .border_l(px(2.0))
        .border_l_color(move || {
            if columns.get().first_visible() == Some(ListColumn::Id) {
                row_edge_mark(rows, selected, baseline, index)
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .flex_row()
        .items_center()
        .overflow_hidden()
        .child(
            mono(move || {
                rows.with(|rows| {
                    rows.get(index)
                        .map(|row| row.id.to_string())
                        .unwrap_or_default()
                })
            })
            .min_w(px(0.0))
            .tabular_nums()
            .number_of_lines(1)
            .text_overflow(EllipsizeMode::Tail)
            .text_size(px(12.0))
            .text_color(fg_muted()),
        )
}

pub(crate) fn status_cell(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    columns: Signal<ListColumns>,
    index: usize,
) -> impl IntoElement {
    let is_first = move || columns.get().first_visible() == Some(ListColumn::Status);
    view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(move || if is_first() { px(2.0) } else { px(4.0) })
        .pr(move || column_pad_right(Some(ListColumn::Status), false, columns.get()))
        .border_l(move || if is_first() { px(2.0) } else { px(0.0) })
        .border_l_color(move || row_edge_mark(rows, selected, baseline, index))
        .flex_row()
        .items_center()
        .overflow_hidden()
        .child(
            mono(move || {
                rows.with(|rows| {
                    rows.get(index)
                        .map(|row| list_status(row.status, row.failed))
                        .unwrap_or_default()
                })
            })
            .min_w(px(0.0))
            .tabular_nums()
            .number_of_lines(1)
            .text_overflow(EllipsizeMode::Tail)
            .text_size(px(12.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || {
                rows.with(|rows| {
                    rows.get(index)
                        .map(|row| http_status_fg(row.status, row.failed))
                        .unwrap_or_else(fg_muted)
                })
            }),
        )
}

pub(crate) fn time_cell(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    columns: Signal<ListColumns>,
    hover: CellHover,
    index: usize,
) -> impl IntoElement {
    let clock_rows = rows;
    let is_first = move || columns.get().first_visible() == Some(ListColumn::Gap);
    view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(move || if is_first() { px(2.0) } else { px(4.0) })
        .pr(move || column_pad_right(Some(ListColumn::Gap), true, columns.get()))
        .border_l(move || if is_first() { px(2.0) } else { px(0.0) })
        .border_l_color(move || row_edge_mark(rows, selected, baseline, index))
        .flex_row()
        .items_center()
        .justify_end()
        .overflow_hidden()
        .on_mouse_move(move |event: &MouseEvent| {
            let clock = clock_rows.with(|rows| {
                rows.get(index)
                    .map(|row| row.clock.clone())
                    .unwrap_or_default()
            });
            show_cell_hover(hover, event, clock);
        })
        .on_mouse_leave(move |_: &MouseEvent| hover.text.set(None))
        .child(
            mono(move || {
                rows.with(|rows| {
                    rows.get(index)
                        .map(|row| row.time.clone())
                        .unwrap_or_default()
                })
            })
            .min_w(px(0.0))
            .tabular_nums()
            .number_of_lines(1)
            .text_overflow(EllipsizeMode::Tail)
            .text_size(px(12.0))
            .text_color(fg_muted()),
        )
}

pub(crate) fn elapsed_cell(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    columns: Signal<ListColumns>,
    index: usize,
) -> impl IntoElement {
    let is_first = move || columns.get().first_visible() == Some(ListColumn::Elapsed);
    view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(move || if is_first() { px(2.0) } else { px(4.0) })
        .pr(move || column_pad_right(Some(ListColumn::Elapsed), true, columns.get()))
        .border_l(move || if is_first() { px(2.0) } else { px(0.0) })
        .border_l_color(move || row_edge_mark(rows, selected, baseline, index))
        .flex_row()
        .items_center()
        .justify_end()
        .overflow_hidden()
        .child(
            mono(move || {
                rows.with(|rows| {
                    rows.get(index)
                        .map(|row| row.elapsed.clone())
                        .unwrap_or_default()
                })
            })
            .min_w(px(0.0))
            .tabular_nums()
            .number_of_lines(1)
            .text_overflow(EllipsizeMode::Tail)
            .text_size(px(12.0))
            .text_color(move || {
                if rows.with(|rows| rows.get(index).is_some_and(|row| row.elapsed_slow)) {
                    status_warn_fg()
                } else {
                    fg_muted()
                }
            }),
        )
}

pub(crate) fn host_cell(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    columns: Signal<ListColumns>,
    hover: CellHover,
    index: usize,
) -> impl IntoElement {
    let tip_rows = rows;
    let is_first = move || columns.get().first_visible() == Some(ListColumn::Host);
    view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(move || if is_first() { px(2.0) } else { px(4.0) })
        .pr(move || column_pad_right(Some(ListColumn::Host), false, columns.get()))
        .border_l(move || if is_first() { px(2.0) } else { px(0.0) })
        .border_l_color(move || row_edge_mark(rows, selected, baseline, index))
        .overflow_hidden()
        .flex_row()
        .items_center()
        .on_mouse_move(move |event: &MouseEvent| {
            let host = tip_rows.with(|rows| {
                rows.get(index)
                    .map(|row| row.host.clone())
                    .unwrap_or_default()
            });
            show_cell_hover(hover, event, host);
        })
        .on_mouse_leave(move |_: &MouseEvent| hover.text.set(None))
        .child(
            mono(move || {
                rows.with(|rows| {
                    rows.get(index)
                        .map(|row| row.host.clone())
                        .unwrap_or_default()
                })
            })
            .w_full()
            .min_w(px(0.0))
            .number_of_lines(1)
            .text_overflow(move || columns.get().ellipsize_mode())
            .text_size(px(12.0))
            .text_color(fg_secondary()),
        )
}

pub(crate) fn client_ip_cell(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    columns: Signal<ListColumns>,
    hover: CellHover,
    index: usize,
) -> impl IntoElement {
    let tip_rows = rows;
    let is_first = move || columns.get().first_visible() == Some(ListColumn::ClientIp);
    view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(move || if is_first() { px(2.0) } else { px(4.0) })
        .pr(move || column_pad_right(Some(ListColumn::ClientIp), false, columns.get()))
        .border_l(move || if is_first() { px(2.0) } else { px(0.0) })
        .border_l_color(move || row_edge_mark(rows, selected, baseline, index))
        .overflow_hidden()
        .flex_row()
        .items_center()
        .on_mouse_move(move |event: &MouseEvent| {
            let ip = tip_rows.with(|rows| {
                rows.get(index)
                    .map(|row| {
                        if row.client_ip == "—" {
                            String::new()
                        } else {
                            row.client_ip.clone()
                        }
                    })
                    .unwrap_or_default()
            });
            show_cell_hover(hover, event, ip);
        })
        .on_mouse_leave(move |_: &MouseEvent| hover.text.set(None))
        .child(
            mono(move || {
                rows.with(|rows| {
                    rows.get(index)
                        .map(|row| row.client_ip.clone())
                        .unwrap_or_default()
                })
            })
            .w_full()
            .min_w(px(0.0))
            .number_of_lines(1)
            .text_overflow(move || columns.get().ellipsize_mode())
            .text_size(px(12.0))
            .text_color(fg_secondary()),
        )
}

pub(crate) fn protocol_cell(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    columns: Signal<ListColumns>,
    index: usize,
) -> impl IntoElement {
    let is_first = move || columns.get().first_visible() == Some(ListColumn::Protocol);
    view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(move || if is_first() { px(2.0) } else { px(4.0) })
        .pr(move || column_pad_right(Some(ListColumn::Protocol), false, columns.get()))
        .border_l(move || if is_first() { px(2.0) } else { px(0.0) })
        .border_l_color(move || row_edge_mark(rows, selected, baseline, index))
        .flex_row()
        .items_center()
        .overflow_hidden()
        .child(
            mono(move || {
                rows.with(|rows| {
                    rows.get(index)
                        .map(|row| row.protocol.clone())
                        .unwrap_or_default()
                })
            })
            .min_w(px(0.0))
            .number_of_lines(1)
            .text_overflow(EllipsizeMode::Tail)
            .text_size(px(12.0))
            .text_color(fg_secondary()),
        )
}

pub(crate) fn size_cell(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    columns: Signal<ListColumns>,
    index: usize,
) -> impl IntoElement {
    let is_first = move || columns.get().first_visible() == Some(ListColumn::Size);
    view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(move || if is_first() { px(2.0) } else { px(4.0) })
        .pr(move || column_pad_right(Some(ListColumn::Size), true, columns.get()))
        .border_l(move || if is_first() { px(2.0) } else { px(0.0) })
        .border_l_color(move || row_edge_mark(rows, selected, baseline, index))
        .flex_row()
        .items_center()
        .justify_end()
        .overflow_hidden()
        .child(
            mono(move || {
                rows.with(|rows| {
                    rows.get(index)
                        .map(|row| row.size.clone())
                        .unwrap_or_default()
                })
            })
            .min_w(px(0.0))
            .tabular_nums()
            .number_of_lines(1)
            .text_overflow(EllipsizeMode::Tail)
            .text_size(px(12.0))
            .text_color(fg_muted()),
        )
}

pub(crate) fn path_cell(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    columns: Signal<ListColumns>,
    hover: CellHover,
    index: usize,
) -> impl IntoElement {
    let langs = ui_langs();
    let tip_rows = rows;
    let is_first = move || columns.get().first_visible() == Some(ListColumn::Path);
    view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(move || if is_first() { px(2.0) } else { px(4.0) })
        .pr(move || column_pad_right(Some(ListColumn::Path), false, columns.get()))
        .border_l(move || if is_first() { px(2.0) } else { px(0.0) })
        .border_l_color(move || row_edge_mark(rows, selected, baseline, index))
        .overflow_hidden()
        .flex_row()
        .items_center()
        .child(when(
            move || {
                rows.with(|rows| {
                    rows.get(index)
                        .is_some_and(|row| !row.relay_short.is_empty())
                })
            },
            move || {
                view()
                    .mr(px(6.0))
                    .px(px(4.0))
                    .rounded(px(4.0))
                    .flex_shrink_0()
                    .bg(accent_blue_soft())
                    .child(
                        text(move || {
                            let short = rows.with(|rows| {
                                rows.get(index)
                                    .map(|row| row.relay_short.clone())
                                    .unwrap_or_default()
                            });
                            format!("{} {short}", t(ui_lang(langs), "中继", "relay"))
                        })
                        .number_of_lines(1)
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(accent_blue_text()),
                    )
            },
        ))
        .child(
            mono(move || {
                let full_url = columns.get().full_url;
                rows.with(|rows| {
                    rows.get(index)
                        .map(|row| row_target_display(row, full_url))
                        .unwrap_or_default()
                })
            })
            .w_full()
            .min_w(px(0.0))
            .number_of_lines(1)
            .text_overflow(move || columns.get().ellipsize_mode())
            .text_size(px(12.0))
            .text_color(move || {
                if rows.with(|rows| rows.get(index).is_some_and(|row| row.failed)) {
                    status_err_fg()
                } else {
                    fg_secondary()
                }
            }),
        )
        .on_mouse_move(move |event: &MouseEvent| {
            let full_url = columns.get().full_url;
            let path = tip_rows.with(|rows| {
                rows.get(index)
                    .map(|row| {
                        let target = row_target_display(row, full_url);
                        if row.relay_short.is_empty() {
                            target
                        } else {
                            format!(
                                "{} {} · {target}",
                                t(ui_lang(langs), "中继", "relay"),
                                row.relay_short
                            )
                        }
                    })
                    .unwrap_or_default()
            });
            show_cell_hover(hover, event, path);
        })
        .on_mouse_leave(move |_: &MouseEvent| hover.text.set(None))
}

pub(crate) fn row_target_text(row: &RowLine, full_url: bool) -> &str {
    if full_url && !row.url.is_empty() {
        &row.url
    } else {
        &row.path
    }
}

pub(crate) fn row_target_display(row: &RowLine, full_url: bool) -> String {
    let base = row_target_text(row, full_url);
    if row.comment.is_empty() {
        base.to_string()
    } else {
        format!("{base} · #{}", row.comment)
    }
}

pub(crate) fn label_tip(zh: &'static str, en: &'static str) -> impl IntoTooltip {
    tooltip().child(
        view()
            .px_2()
            .py_1()
            .rounded(px(6.0))
            .bg(bg_card())
            .border_1()
            .border_color(border_strong())
            .child(
                text(tr(zh, en))
                    .text_size(px(12.0))
                    .text_color(fg_primary()),
            ),
    )
}

/// The list's one hover bubble. It lives beside the table, not inside a
/// cell: DataTable disposes a cell when the row in that slot changes.
pub(crate) fn cell_tip(full: impl Fn() -> String + 'static) -> impl IntoElement {
    view()
        .max_w(move || px((window().width.0 - 16.0).clamp(180.0, 420.0)))
        .px_2p5()
        .py_1p5()
        .rounded(px(6.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
        .child(
            mono(full)
                .max_w(move || px((window().width.0 - 36.0).clamp(160.0, 396.0)))
                .number_of_lines(8)
                .text_overflow(EllipsizeMode::Tail)
                .text_size(px(12.0))
                .text_color(fg_primary()),
        )
}

/// Opts an outer `scroll()`'s content subtree out of `nsh-core` 0.5's `cull_scope`
/// and `pace::sight` deferral.
///
/// With `content_culling: true` (enabled on macOS, Linux, and Windows in `nsh` 0.5),
/// nodes mounted inside a `scroll()` via `sync_culled` or `sync_paced` go through
/// `ensure_created -> mount_state`, where `mount_state` omits `dirty::VIEW_PROPS` when
/// `node.props.is_none()` even if `node.class_pointer != PointerEvents::Auto`. For
/// `editor()` (whose `Layer::Paint` overlay covers `vscroll` with
/// `.pointer_events(PointerEvents::None)`) and `diff_view()`, missing `ViewProps` leaves
/// the paint overlay at `PointerEvents::Auto` natively, intercepting wheel/trackpad and
/// pointer hit-tests so scrolling falls through to the outer `scroll()`. Recording an
/// identity `translate_x(px(0.0))` op makes `cull_scope` return `None` and `pace::sight`
/// return `0.0` (immediate creation even below the fold) while `Transform::is_identity()`
/// stays `true` (so native backends allocate no layer transform matrix).
pub(crate) fn uncull_nested_scroll(el: View) -> View {
    el.translate_x(px(0.0))
}

/// Ensures class-driven `pointer_events` (`PointerEvents::None` / `BoxNone`) on internal
/// child overlays of `editor()` and `diff_view()` are emitted as `MountItem::ViewProps`
/// even when `pace_hold` (`PACE_AFTER = 64`) defers view creation inside a `scroll()`.
///
/// Starting the wrapper at `PointerEvents::None` keeps descendant `Layer::Paint` nodes at
/// `class_pointer == PointerEvents::Auto` (`blocked = true` in `update_pointer_events`)
/// during initial creation; flipping to `PointerEvents::Auto` on first layout (once
/// `sync_culled` / `sync_paced` has created the native views) transitions `Layer::Paint`
/// to `PointerEvents::None` while `created == true`, guaranteeing `MountItem::ViewProps`.
pub(crate) fn guard_culled_pointer_events(el: View) -> View {
    let ready = signal(false);
    el.pointer_events(move || {
        if ready.get() {
            PointerEvents::Auto
        } else {
            PointerEvents::None
        }
    })
    .on_layout(move |_: &Bounds<Points>| {
        if !ready.get_untracked() {
            ready.set(true);
        }
    })
}
