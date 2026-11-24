#[allow(unused_imports)]
use super::*;

pub(crate) fn flow_cell(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    columns: Signal<ListColumns>,
    effective_filter: Signal<String>,
    hover: CellHover,
    cell: &DataTableCell,
) -> AnyElement {
    let index = cell.row as usize;
    match cell.key.as_ref() {
        "id" => id_cell(rows, selected, baseline, columns, index).into_element(),
        "method" => method_cell(rows, selected, baseline, columns, index).into_element(),
        "path" => path_cell(
            rows,
            selected,
            baseline,
            columns,
            effective_filter,
            hover,
            index,
        )
        .into_element(),
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
    effective_filter: Signal<String>,
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
    let mut row = view()
        .role(AccessibilityRole::Button)
        .w_full()
        .h(px(28.0))
        .min_w(px(0.0))
        .pl(px(pad_left))
        .pr(move || column_pad_right(column, align_right, columns.get()))
        .gap_0p5()
        .flex_row()
        .items_center()
        .overflow_hidden()
        .transition_colors()
        .duration(move || motion_ms(MOTION_INSTANT_MS))
        .ease_out()
        .focus_visible(apply_focus_ring);
    if column.is_some() {
        row = row
            .hover(|s| s.bg(bg_control_hover()))
            .active(|s| s.bg(bg_control_active()));
    }
    let row = if align_right { row.justify_end() } else { row };
    row.child(
        text(move || -> SharedString {
            let display = effective_path_display(columns.get(), &effective_filter.get());
            flow_column_title(&key, display).into()
        })
        .number_of_lines(1)
        .text_size(move || scaled_px(11.0))
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
            10.0,
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
    let row = rows.with(|rows| rows.get(index).map(|row| (row.id, row.is_marker)));
    match row {
        Some((id, _)) if selected.get() == Some(id) => accent_blue(),
        Some((_, true)) => accent_blue_border(),
        Some((id, _)) if baseline.get() == Some(id) => baseline_mark(),
        _ => AdaptiveColor::TRANSPARENT,
    }
}

fn marker_row_bg(rows: Signal<Vec<Arc<RowLine>>>, index: usize) -> AdaptiveColor {
    if rows.with(|rows| rows.get(index).is_some_and(|row| row.is_marker)) {
        accent_blue_soft()
    } else {
        AdaptiveColor::TRANSPARENT
    }
}

fn marker_row_border(rows: Signal<Vec<Arc<RowLine>>>, index: usize) -> AdaptiveColor {
    if rows.with(|rows| rows.get(index).is_some_and(|row| row.is_marker)) {
        accent_blue_border()
    } else {
        AdaptiveColor::TRANSPARENT
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
        .bg(move || marker_row_bg(rows, index))
        .border_t_1()
        .border_b_1()
        .border_t_color(move || marker_row_border(rows, index))
        .border_b_color(move || marker_row_border(rows, index))
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
                .rounded(px(RADIUS_SM))
                .bg(move || {
                    rows.with(|rows| {
                        rows.get(index)
                            .map(|row| {
                                if row.is_marker {
                                    accent_blue()
                                } else {
                                    method_bg(&row.method)
                                }
                            })
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
                    .text_size(move || scaled_px(10.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(move || {
                        rows.with(|rows| {
                            rows.get(index)
                                .map(|row| {
                                    if row.is_marker {
                                        fg_on_accent()
                                    } else {
                                        method_fg(&row.method)
                                    }
                                })
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
        .bg(move || marker_row_bg(rows, index))
        .border_t_1()
        .border_b_1()
        .border_t_color(move || marker_row_border(rows, index))
        .border_b_color(move || marker_row_border(rows, index))
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
                        .map(|row| {
                            if row.is_marker {
                                "—".to_string()
                            } else {
                                row.id.to_string()
                            }
                        })
                        .unwrap_or_default()
                })
            })
            .min_w(px(0.0))
            .tabular_nums()
            .number_of_lines(1)
            .text_overflow(EllipsizeMode::Tail)
            .text_size(move || scaled_px(12.0))
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
        .bg(move || marker_row_bg(rows, index))
        .border_t_1()
        .border_b_1()
        .border_t_color(move || marker_row_border(rows, index))
        .border_b_color(move || marker_row_border(rows, index))
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
                        .map(|row| {
                            if row.is_marker {
                                String::new()
                            } else {
                                list_status(row.status, row.failed)
                            }
                        })
                        .unwrap_or_default()
                })
            })
            .min_w(px(0.0))
            .tabular_nums()
            .number_of_lines(1)
            .text_overflow(EllipsizeMode::Tail)
            .text_size(move || scaled_px(12.0))
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
        .bg(move || marker_row_bg(rows, index))
        .border_t_1()
        .border_b_1()
        .border_t_color(move || marker_row_border(rows, index))
        .border_b_color(move || marker_row_border(rows, index))
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
        .on_mouse_leave(move |_: &MouseEvent| hide_cell_hover(hover))
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
            .text_size(move || scaled_px(12.0))
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
        .bg(move || marker_row_bg(rows, index))
        .border_t_1()
        .border_b_1()
        .border_t_color(move || marker_row_border(rows, index))
        .border_b_color(move || marker_row_border(rows, index))
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
            .text_size(move || scaled_px(12.0))
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
        .bg(move || marker_row_bg(rows, index))
        .border_t_1()
        .border_b_1()
        .border_t_color(move || marker_row_border(rows, index))
        .border_b_color(move || marker_row_border(rows, index))
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
        .on_mouse_leave(move |_: &MouseEvent| hide_cell_hover(hover))
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
            .text_size(move || scaled_px(12.0))
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
        .bg(move || marker_row_bg(rows, index))
        .border_t_1()
        .border_b_1()
        .border_t_color(move || marker_row_border(rows, index))
        .border_b_color(move || marker_row_border(rows, index))
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
        .on_mouse_leave(move |_: &MouseEvent| hide_cell_hover(hover))
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
            .text_size(move || scaled_px(12.0))
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
        .bg(move || marker_row_bg(rows, index))
        .border_t_1()
        .border_b_1()
        .border_t_color(move || marker_row_border(rows, index))
        .border_b_color(move || marker_row_border(rows, index))
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
            .text_size(move || scaled_px(12.0))
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
        .bg(move || marker_row_bg(rows, index))
        .border_t_1()
        .border_b_1()
        .border_t_color(move || marker_row_border(rows, index))
        .border_b_color(move || marker_row_border(rows, index))
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
            .text_size(move || scaled_px(12.0))
            .text_color(fg_muted()),
        )
}

pub(crate) fn path_cell(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    columns: Signal<ListColumns>,
    effective_filter: Signal<String>,
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
        .bg(move || marker_row_bg(rows, index))
        .border_t_1()
        .border_b_1()
        .border_t_color(move || marker_row_border(rows, index))
        .border_b_color(move || marker_row_border(rows, index))
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
                    .h(move || scaled_px(18.0))
                    .mr(px(6.0))
                    .px(px(4.0))
                    .rounded(px(RADIUS_SM))
                    .flex_shrink_0()
                    .flex_row()
                    .items_center()
                    .justify_center()
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
                        .text_size(move || scaled_px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(accent_blue_text()),
                    )
            },
        ))
        .child(when(
            move || {
                rows.with(|rows| {
                    rows.get(index)
                        .is_some_and(|row| !row.rule_marker.is_empty())
                })
            },
            move || {
                view()
                    .h(move || scaled_px(18.0))
                    .mr(px(6.0))
                    .px(px(5.0))
                    .rounded(px(RADIUS_SM))
                    .flex_shrink_0()
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .bg(accent_blue_soft())
                    .child(icon::glyph_strip(
                        move || scaled_px(12.0).0,
                        move || scaled_px(3.5).0,
                        move || {
                            rows.with(|rows| {
                                rows.get(index)
                                    .map(|row| rule_marker_glyphs(&row.rule_marker))
                                    .unwrap_or_default()
                            })
                        },
                        || color(accent_blue_text()),
                    ))
            },
        ))
        .child(
            mono(move || {
                let display = effective_path_display(columns.get(), &effective_filter.get());
                rows.with(|rows| {
                    rows.get(index)
                        .map(|row| row_target_display(row, display))
                        .unwrap_or_default()
                })
            })
            .flex_1()
            .min_w(px(0.0))
            .number_of_lines(1)
            .text_overflow(move || columns.get().ellipsize_mode())
            .text_size(move || scaled_px(12.0))
            .text_color(move || {
                rows.with(|rows| match rows.get(index) {
                    Some(row) if row.is_marker => accent_blue_text(),
                    Some(row) if row.failed => status_err_fg(),
                    _ => fg_secondary(),
                })
            }),
        )
        .on_mouse_move(move |event: &MouseEvent| {
            let cols = columns.get();
            let eff_filter = effective_filter.get();
            let shown_display = effective_path_display(cols, &eff_filter);
            let tip_display = effective_path_hover_display(cols, &eff_filter);
            let path = tip_rows.with(|rows| {
                rows.get(index)
                    .map(|row| {
                        let effective =
                            if tip_display == EffectivePathDisplay::FullUrl && row.url.is_empty() {
                                EffectivePathDisplay::HostAndPath
                            } else {
                                tip_display
                            };
                        let shown = row_target_display(row, shown_display);
                        let target = row_target_display(row, effective);
                        let with_relay = if row.relay_short.is_empty() {
                            target
                        } else {
                            format!(
                                "{} {} · {target}",
                                t(ui_lang(langs), "中继", "relay"),
                                row.relay_short
                            )
                        };
                        let full = if row.rule_summary.is_empty() {
                            with_relay
                        } else {
                            format!("[{}] {with_relay}", row.rule_summary)
                        };
                        if !row.is_marker
                            && row.relay_short.is_empty()
                            && row.rule_summary.is_empty()
                            && full == shown
                            && shown.chars().count() <= 18
                        {
                            String::new()
                        } else {
                            full
                        }
                    })
                    .unwrap_or_default()
            });
            show_cell_hover(hover, event, path);
        })
        .on_mouse_leave(move |_: &MouseEvent| hide_cell_hover(hover))
}

pub(crate) fn row_target_text(row: &RowLine, display: EffectivePathDisplay) -> &str {
    match display {
        EffectivePathDisplay::PathOnly => &row.path,
        EffectivePathDisplay::HostAndPath => &row.host_path,
        EffectivePathDisplay::FullUrl => {
            if !row.url.is_empty() {
                &row.url
            } else {
                &row.path
            }
        }
    }
}

pub(crate) fn row_target_display(row: &RowLine, display: EffectivePathDisplay) -> String {
    if row.is_marker {
        return format!("── {} ──", row.path);
    }
    let base = row_target_text(row, display);
    if row.comment.is_empty() {
        base.to_string()
    } else {
        format!("{base} · #{}", row.comment)
    }
}

pub(crate) fn label_tip(zh: &'static str, en: &'static str) -> impl IntoTooltip {
    tooltip().child(ui_tooltip_surface(
        text(tr(zh, en))
            .text_size(move || scaled_px(TEXT_SM))
            .text_color(fg_primary()),
    ))
}

/// The list's one hover bubble. It lives beside the table, not inside a
/// cell: DataTable disposes a cell when the row in that slot changes.
pub(crate) fn cell_tip(full: impl Fn() -> String + 'static) -> impl IntoElement {
    view()
        .max_w(move || px((window().width.0 - 16.0).clamp(180.0, 420.0)))
        .px_2p5()
        .py_1p5()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
        .child(
            mono(full)
                .max_w(move || px((window().width.0 - 36.0).clamp(160.0, 396.0)))
                .number_of_lines(8)
                .text_overflow(EllipsizeMode::Tail)
                .text_size(move || scaled_px(12.0))
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
