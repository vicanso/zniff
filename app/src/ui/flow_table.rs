#[allow(unused_imports)]
use super::*;

#[derive(Clone)]
pub(crate) struct EmptyFilterState {
    pub(crate) effective_filter: Signal<String>,
    pub(crate) main_filter: Signal<String>,
    pub(crate) filter_enabled: Signal<bool>,
    pub(crate) list_filter: Signal<String>,
    pub(crate) diagnosis: Signal<EmptyFilterDiagnosis>,
    pub(crate) running: Signal<bool>,
    pub(crate) on_clear_primary: Option<Rc<dyn Fn()>>,
}

/// Returns whether `row` lies outside the currently visible `[start, end)` range
/// (or the viewport has not yet reported a non-empty visible range).
pub(crate) fn should_reveal_selected_row(row: u32, visible: DataTableRange) -> bool {
    visible.start >= visible.end || row < visible.start || row >= visible.end
}

pub(crate) fn list_empty_subtitle(lang: Lang, filtered: bool, running: bool) -> &'static str {
    if filtered {
        t(lang, "试试调整上方的过滤关键词", "Try a different filter")
    } else if running {
        t(
            lang,
            "正在监听，等待 HTTP(S) 请求",
            "Listening, waiting for HTTP(S) traffic",
        )
    } else {
        t(
            lang,
            "点击右上角「开始」捕获 HTTP(S) 请求",
            "Press Start to capture HTTP(S)",
        )
    }
}

pub(crate) fn list_empty(state: EmptyFilterState) -> impl IntoElement {
    let filter = state.effective_filter;
    let main_filter = state.main_filter;
    let filter_enabled = state.filter_enabled;
    let list_filter = state.list_filter;
    let diagnosis = state.diagnosis;
    let running = state.running;
    let on_clear_primary = state.on_clear_primary;

    view()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_1p5()
        .px_6()
        .child(glyph_px(28.0, || Glyph::List, || color(fg_muted())))
        .child(
            text({
                let langs = ui_langs();
                move || -> SharedString {
                    let lang = ui_lang(langs);
                    if filter.get().trim().is_empty() {
                        t(lang, "还没有流量", "No traffic yet").into()
                    } else {
                        t(lang, "没有匹配的流量", "No matching traffic").into()
                    }
                }
            })
            .text_center()
            .text_size(px(TEXT_MD))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(fg_secondary()),
        )
        .child(
            text({
                let langs = ui_langs();
                move || -> SharedString {
                    let lang = ui_lang(langs);
                    let filtered = !filter.get().trim().is_empty();
                    if filtered
                        && let Some(diag_text) =
                            diagnosis.with(|d| format_empty_filter_diagnosis(lang, d))
                    {
                        return diag_text.into();
                    }
                    list_empty_subtitle(lang, filtered, running.get()).into()
                }
            })
            .text_center()
            .text_size(px(TEXT_SM))
            .text_color(fg_muted()),
        )
        .child(when(
            move || !filter.get().trim().is_empty(),
            move || {
                let on_clear_primary = on_clear_primary.clone();
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .mt_2()
                    .child(empty_clear_filter_button(
                        main_filter,
                        filter_enabled,
                        list_filter,
                        on_clear_primary,
                    ))
                    .child(when(
                        move || filter_enabled.get() && !main_filter.get().trim().is_empty(),
                        move || empty_pause_primary_button(filter_enabled),
                    ))
            },
        ))
}

fn empty_clear_filter_button(
    main_filter: Signal<String>,
    filter_enabled: Signal<bool>,
    list_filter: Signal<String>,
    on_clear_primary: Option<Rc<dyn Fn()>>,
) -> impl IntoElement {
    view()
        .role(AccessibilityRole::Button)
        .h(px(CONTROL_H))
        .px_3()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(accent_blue_border())
        .bg(accent_blue_soft())
        .hover(|style| style.bg(accent_blue_soft_hover()))
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(move || motion_ms(MOTION_INSTANT_MS))
        .ease_out()
        .focus_visible(apply_focus_ring)
        .label(tr("清空过滤", "Clear filter"))
        .on_press(move || {
            if !list_filter.get_untracked().is_empty() {
                list_filter.set(String::new());
            }
            if filter_enabled.get_untracked() && !main_filter.get_untracked().trim().is_empty() {
                main_filter.set(String::new());
                if let Some(cb) = &on_clear_primary {
                    cb();
                }
            }
        })
        .child(
            control_text(tr("清空过滤", "Clear filter"))
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::MEDIUM)
                .text_color(accent_blue_text()),
        )
}

fn empty_pause_primary_button(filter_enabled: Signal<bool>) -> impl IntoElement {
    view()
        .role(AccessibilityRole::Button)
        .h(px(CONTROL_H))
        .px_3()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border_control())
        .bg(bg_control())
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(move || motion_ms(MOTION_INSTANT_MS))
        .ease_out()
        .focus_visible(apply_focus_ring)
        .label(tr("暂停主过滤", "Pause primary filter"))
        .on_press(move || filter_enabled.set(false))
        .child(
            control_text(tr("暂停主过滤", "Pause primary filter"))
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg_secondary()),
        )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn list_menu(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
    compose: Compose,
    columns: Signal<ListColumns>,
    sort: Signal<Option<ListSort>>,
    controls: Arc<Controls>,
    empty_state: EmptyFilterState,
    shown: Shown,
    baseline: Signal<Option<u64>>,
    profiles: ProfileSignals,
    picked: Signal<Option<u64>>,
    menu_open: Signal<bool>,
    menu_at: Signal<Point<Points>>,
) -> impl IntoElement {
    let header_menu = signal(false);
    let header_controls = controls.clone();
    let list_filter = empty_state.list_filter;
    let layout_mode = profiles.layout_mode;
    // The table is a direct flex child. Wrapping it in `context_menu` gave
    // that component a content-sized root, so the rows collapsed to nothing
    // while the toolbar count still updated.
    view()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .child(flow_table(
            rows,
            selected,
            baseline,
            compose,
            columns,
            sort,
            controls.clone(),
            empty_state,
            layout_mode,
            picked,
            menu_open,
            header_menu,
            menu_at,
        ))
        .child(when(
            move || menu_open.get(),
            move || {
                overlay()
                    .anchor_point(menu_at)
                    .placement(Placement::Bottom)
                    .align(Align::Start)
                    .offset(px(4.0))
                    .margin(px(8.0))
                    .on_press_outside(move |_: &Point<Points>| menu_open.set(false))
                    .on_cancel(move || menu_open.set(false))
                    .child(row_context_menu(
                        picked,
                        selected,
                        paused,
                        compose,
                        controls.clone(),
                        list_filter,
                        shown,
                        baseline,
                        profiles,
                        menu_open,
                    ))
            },
        ))
        .child(when(
            move || header_menu.get(),
            move || {
                overlay()
                    .anchor_point(menu_at)
                    .placement(Placement::Bottom)
                    .align(Align::Start)
                    .offset(px(4.0))
                    .margin(px(8.0))
                    .on_press_outside(move |_: &Point<Points>| header_menu.set(false))
                    .on_cancel(move || header_menu.set(false))
                    .child(column_picker_panel(columns, header_controls.clone()))
            },
        ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn flow_table(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    compose: Compose,
    columns: Signal<ListColumns>,
    sort: Signal<Option<ListSort>>,
    controls: Arc<Controls>,
    empty_state: EmptyFilterState,
    layout_mode: Signal<config::LayoutMode>,
    picked: Signal<Option<u64>>,
    menu_open: Signal<bool>,
    header_menu: Signal<bool>,
    menu_at: Signal<Point<Points>>,
) -> impl IntoElement {
    let filter = empty_state.effective_filter;
    let saved_cols = saved_config();
    let widths = signal(saved_cols.column_widths);
    let widths_top_bottom = signal(saved_cols.column_widths_top_bottom);
    let table = component_ref::<DataTable>();
    let blur_job = stored(None::<Spawned<()>>);
    // A click focuses the table, and its arrow keys would step the row again
    // on top of the window's. Take the keys back, including after the click's
    // own focus lands.
    let blur_table: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(api) = table.api::<DataTableApi>() {
            api.blur();
        }
        let table = table;
        blur_job.replace(Some(set_timeout(Duration::from_millis(0), move || {
            if let Some(api) = table.api::<DataTableApi>() {
                api.blur();
            }
        })));
    });
    let follow = stored(true);
    let followed_count = stored(0_u32);
    let hover = CellHover {
        text: signal(None::<String>),
        at: signal(Point::<Points>::default()),
        intent: stored(None::<Spawned<()>>),
    };
    let save_job = stored(None::<Spawned<()>>);
    let widths_for_cols = widths;
    let widths_tb_for_cols = widths_top_bottom;
    let rows_for_cells = rows;
    let selected_for_cells = selected;
    let baseline_for_cells = baseline;
    let blur_header = Rc::clone(&blur_table);
    let blur_press = Rc::clone(&blur_table);
    let blur_menu = Rc::clone(&blur_table);
    let blur_select = Rc::clone(&blur_table);

    effect(move || {
        if !menu_open.get()
            && !header_menu.get()
            && let Some(api) = table.api::<DataTableApi>()
        {
            api.clear_menu_target();
        }
    });

    // Keep keyboard/programmatic selection inside the visible viewport.
    // Reading `rows` untracked ensures incoming traffic does not yank manual scroll.
    effect(move || {
        let Some(id) = selected.get() else {
            return;
        };
        let (idx, count) = rows.with_untracked(|rows| {
            (
                rows.iter().position(|row| row.id == id).map(|i| i as u32),
                rows.len() as u32,
            )
        });
        let Some(idx) = idx else {
            return;
        };
        if let Some(api) = table.api::<DataTableApi>() {
            let visible = api.visible_rows();
            if should_reveal_selected_row(idx, visible) {
                follow.replace(idx.saturating_add(1) >= count);
            }
            let mut selection = DataTableSelection::default();
            selection.row = Some(idx);
            api.select(Some(selection));
        }
    });

    // The hover bubble stays beside the table. A tooltip inside a cell is
    // disposed when DataTable recycles that cell, and the binding then
    // reads a prop whose owner is gone.
    let table_key = computed(move || {
        columns.with(|c| {
            (
                c.id,
                c.method,
                c.path,
                c.status,
                c.gap,
                c.elapsed,
                c.size,
                c.protocol,
                c.host,
                c.client_ip,
                c.order_rev,
            )
        })
    });
    view()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .child(dynamic(move || {
            let _ = table_key.get();
            let shown_at_mount = untracked(|| columns.get());
            let blur_header = Rc::clone(&blur_header);
            let blur_press = Rc::clone(&blur_press);
            let blur_menu = Rc::clone(&blur_menu);
            let blur_select = Rc::clone(&blur_select);
            let controls = controls.clone();
            let move_controls = controls.clone();
            let empty_state = empty_state.clone();
            Some(
                data_table()
                    .component_ref(&table)
                    .flex_1()
                    .min_h(px(0.0))
                    .w_full()
                    .classes(flow_table_classes())
                    .columns(move || {
                        let shown = columns.get();
                        let display = effective_path_display(shown, &filter.get());
                        let is_tb = layout_mode.get() == config::LayoutMode::TopBottom;
                        let current_widths = untracked(|| {
                            if is_tb {
                                widths_tb_for_cols.get()
                            } else {
                                widths_for_cols.get()
                            }
                        });
                        let mut built = flow_table_columns_for_layout(
                            shown_at_mount,
                            &current_widths,
                            is_tb,
                            window().width.0,
                        );
                        for column in &mut built {
                            let key = column.key.to_string();
                            column.title = Some(flow_column_title(&key, display).into());
                        }
                        built
                    })
                    .data(DataTableRows::default())
                    .row_count(move || Some(rows.with(Vec::len) as u32))
                    .row_height(move || scaled_px(34.0))
                    .header_height(px(28.0))
                    .font_size(move || scaled_px(12.0))
                    .row_header(false)
                    .column_selectable(false)
                    .cell_selectable(false)
                    .movable_columns(true)
                    .loop_selection(false)
                    .stripe(true)
                    .selection(move || {
                        let id = selected.get();
                        let row = rows.with(|rows| rows.iter().position(|row| Some(row.id) == id));
                        row.map(|row| {
                            let mut selection = DataTableSelection::default();
                            selection.row = Some(row as u32);
                            selection
                        })
                    })
                    .render_cell(move |cell: &DataTableCell| {
                        flow_cell(
                            rows_for_cells,
                            selected_for_cells,
                            baseline_for_cells,
                            columns,
                            filter,
                            hover,
                            cell,
                        )
                    })
                    .render_header_cell(move |cell: &DataTableHeaderCell| {
                        let blur_header = Rc::clone(&blur_header);
                        let column = ListColumn::from_key(cell.key.as_ref());
                        flow_header(cell, columns, filter, sort).on_press(move || {
                            if let Some(column) = column {
                                sort.set(ListSort::toggle(sort.get(), column));
                            }
                            blur_header();
                        })
                    })
                    .empty(list_empty(empty_state))
                    .on_row_press(move |_: &DataTablePress| blur_press())
                    .on_selection_change(move |selection: &Option<DataTableSelection>| {
                        blur_select();
                        let next = selection
                            .and_then(|selection| selection.row)
                            .and_then(|row| {
                                rows.with(|rows| rows.get(row as usize).map(|row| row.id))
                            });
                        if selected.get() != next {
                            reset_compose(compose);
                            selected.set(next);
                        }
                    })
                    .on_context_menu(move |event: &DataTableContextMenu| {
                        blur_menu();
                        hover.text.set(None);
                        menu_at.set(Point {
                            x: event.x,
                            y: event.y,
                        });
                        if let Some(row) = event.row {
                            header_menu.set(false);
                            let next = rows.with(|rows| rows.get(row as usize).map(|row| row.id));
                            picked.set(next);
                            menu_open.set(true);
                        } else {
                            menu_open.set(false);
                            header_menu.set(true);
                        }
                    })
                    .on_visible_rows_change(move |range: &DataTableRange| {
                        hover.text.set(None);
                        let count = rows.with(Vec::len) as u32;
                        let auto_follow = matches!(
                            sort.get_untracked(),
                            None | Some(ListSort {
                                column: ListColumn::Id,
                                desc: false
                            })
                        );
                        if count == 0 {
                            follow.replace(auto_follow);
                            followed_count.replace(0);
                            return;
                        }
                        let near = auto_follow && range.end.saturating_add(1) >= count;
                        if auto_follow && follow.get() && count > followed_count.get() {
                            followed_count.replace(count);
                            if !near && let Some(api) = table.api::<DataTableApi>() {
                                api.scroll_to_row(count - 1, Some(false));
                            }
                            return;
                        }
                        if !auto_follow {
                            followed_count.replace(count);
                        }
                        follow.replace(near);
                    })
                    .on_column_move(move |mv: &DataTableColumnMove| {
                        let current = columns.get();
                        let next = current.apply_visible_order(&mv.order);
                        if next == current {
                            return;
                        }
                        columns.set(next);
                        if let Err(err) = save_config(|config| next.write(config)) {
                            set_status(
                                &move_controls,
                                StatusMsg::PrefUnsaved {
                                    name: PrefName::List,
                                    err: err.to_string(),
                                },
                            );
                        }
                    })
                    .on_column_resize(move |event: &DataTableEvent| {
                        let DataTableEvent::ColumnResize(key, width) = event else {
                            return;
                        };
                        let Some(limit) = config::column_width_limit(key.as_ref()) else {
                            return;
                        };
                        let px = (width.0.round() as u32).clamp(limit.min, limit.max);
                        let is_tb = layout_mode.get() == config::LayoutMode::TopBottom;
                        let target_widths = if is_tb { widths_top_bottom } else { widths };
                        target_widths.update(|map| {
                            if px == limit.default {
                                map.remove(key.as_ref());
                            } else {
                                map.insert(key.to_string(), px);
                            }
                        });
                        let controls = controls.clone();
                        save_job.replace(Some(set_timeout(
                            Duration::from_millis(300),
                            move || {
                                let map = target_widths.get();
                                if let Err(err) = save_config(|config| {
                                    if is_tb {
                                        config.column_widths_top_bottom = map;
                                    } else {
                                        config.column_widths = map;
                                    }
                                }) {
                                    set_status(
                                        &controls,
                                        StatusMsg::PrefUnsaved {
                                            name: PrefName::List,
                                            err: err.to_string(),
                                        },
                                    );
                                }
                            },
                        )));
                    }),
            )
        }))
        .child(when(
            move || hover.text.get().is_some(),
            move || {
                overlay()
                    .pointer_events(PointerEvents::None)
                    .anchor_point(hover.at)
                    .placement(Placement::Top)
                    .align(Align::Start)
                    .offset(px(6.0))
                    .margin(px(8.0))
                    .child(cell_tip(move || hover.text.get().unwrap_or_default()))
            },
        ))
}

#[derive(Clone, Copy)]
pub(crate) struct CellHover {
    pub(crate) text: Signal<Option<String>>,
    pub(crate) at: Signal<Point<Points>>,
    pub(crate) intent: Stored<Option<Spawned<()>>>,
}

pub(crate) fn hide_cell_hover(hover: CellHover) {
    hover.intent.replace(None);
    if hover.text.with_untracked(Option::is_some) {
        hover.text.set(None);
    }
}

pub(crate) fn show_cell_hover(hover: CellHover, event: &MouseEvent, text: String) {
    if text.is_empty() {
        hide_cell_hover(hover);
        return;
    }
    hover.at.set(event.page_position);
    if hover
        .text
        .with_untracked(|cur| cur.as_deref() == Some(text.as_str()))
    {
        hover.intent.replace(None);
        return;
    }
    if hover.text.with_untracked(Option::is_some) {
        hover.intent.replace(None);
        hover.text.set(Some(text));
    } else {
        hover
            .intent
            .replace(Some(set_timeout(Duration::from_millis(280), move || {
                hover.text.set(Some(text));
            })));
    }
}
