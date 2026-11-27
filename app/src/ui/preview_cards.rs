//! Declarative TSX Inspector view renderer (`<Section>`, `<KeyValue>`, `<Badge>`,
//! `<Code>`, `<View>`, `<Row>`, `<Col>`, `<Text>`, `<Divider>`, `<Callout>`,
//! `<Table>`, `<Collapsible>`, `<Tabs>`, `<Tab>`) with zero-JS-residency host actions.

#[allow(unused_imports)]
use super::*;

use crate::hook_spec::{
    HookViewAction, HookViewTabItem, PreviewCardNode, PreviewTone, TextSizeTier,
};

#[derive(Clone, Default)]
pub(crate) struct HookViewActionCtx {
    pub(crate) on_select_flow: Option<Rc<dyn Fn(u64)>>,
    pub(crate) on_set_filter: Option<Rc<dyn Fn(String)>>,
    pub(crate) on_copy: Option<Rc<dyn Fn(String)>>,
}

impl HookViewActionCtx {
    pub(crate) fn for_inspector(
        selected: Signal<Option<u64>>,
        filter: Signal<String>,
        filter_enabled: Option<Signal<bool>>,
    ) -> Self {
        Self {
            on_select_flow: Some(Rc::new(move |id| selected.set(Some(id)))),
            on_set_filter: Some(Rc::new(move |query| {
                if let Some(enabled) = filter_enabled {
                    enabled.set(true);
                }
                filter.set(query);
            })),
            on_copy: Some(Rc::new(set_clipboard)),
        }
    }

    pub(crate) fn with_after_action(self, after: impl Fn() + Clone + 'static) -> Self {
        let prev_select = self.on_select_flow;
        let prev_filter = self.on_set_filter;
        let after_select = after.clone();
        Self {
            on_select_flow: prev_select.map(|f| {
                Rc::new(move |id| {
                    f(id);
                    after_select();
                }) as Rc<dyn Fn(u64)>
            }),
            on_set_filter: prev_filter.map(|f| {
                Rc::new(move |q| {
                    f(q);
                    after();
                }) as Rc<dyn Fn(String)>
            }),
            on_copy: self.on_copy,
        }
    }
}

fn map_badge_tone(tone: PreviewTone) -> BadgeTone {
    match tone {
        PreviewTone::Neutral => BadgeTone::Neutral,
        PreviewTone::Ok => BadgeTone::Ok,
        PreviewTone::Warn => BadgeTone::Warn,
        PreviewTone::Danger => BadgeTone::Err,
        PreviewTone::Accent => BadgeTone::Accent,
    }
}

fn tone_fg(tone: PreviewTone) -> AdaptiveColor {
    match tone {
        PreviewTone::Neutral => fg_primary(),
        PreviewTone::Ok => status_ok_fg(),
        PreviewTone::Warn => status_warn_fg(),
        PreviewTone::Danger => status_err_fg(),
        PreviewTone::Accent => accent_blue_text(),
    }
}

fn callout_colors(tone: PreviewTone) -> (AdaptiveColor, AdaptiveColor, AdaptiveColor) {
    match tone {
        PreviewTone::Neutral => (bg_code(), border_subtle(), fg_primary()),
        PreviewTone::Ok => (status_ok_bg(), status_ok_fg(), status_ok_fg()),
        PreviewTone::Warn => (status_warn_bg(), status_warn_fg(), status_warn_fg()),
        PreviewTone::Danger => (status_err_bg(), status_err_fg(), status_err_fg()),
        PreviewTone::Accent => (accent_blue_soft(), accent_blue_border(), accent_blue_text()),
    }
}

fn copy_text_with_ctx(ctx: &HookViewActionCtx, value: &str) {
    if let Some(ref on_copy) = ctx.on_copy {
        on_copy(value.to_string());
    } else {
        set_clipboard(value);
    }
}

fn action_chip_button(
    label: impl IntoReactive<SharedString>,
    a11y_label: impl Fn() -> Option<SharedString> + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .h(px(ControlSize::Xs.height()))
        .px_1p5()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .rounded(px(RADIUS_SM))
        .border_1()
        .border_color(border_control())
        .bg(bg_control())
        .hover(|s| s.bg(bg_control_hover()))
        .active(|s| s.bg(bg_control_active()))
        .transition_colors()
        .duration(move || motion_ms(MOTION_INSTANT_MS))
        .ease_out()
        .focus_visible(apply_focus_ring)
        .accessibility_label(a11y_label)
        .on_press(move |_: &()| on_press())
        .child(
            text(label)
                .number_of_lines(1)
                .tabular_nums()
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg_secondary()),
        )
}

fn render_action_strip(action: &HookViewAction, ctx: &HookViewActionCtx) -> Option<AnyElement> {
    if action.is_empty() {
        return None;
    }
    let mut row = view()
        .flex_row()
        .flex_wrap()
        .flex_shrink_0()
        .items_center()
        .gap_1();
    let mut has_any = false;
    if let Some(ref copy_val) = action.copy_value {
        has_any = true;
        let copied = signal(false);
        let value = copy_val.clone();
        let ctx_copy = ctx.clone();
        let langs = ui_langs();
        row = row.child(action_chip_button(
            move || -> SharedString {
                if copied.get() {
                    t(ui_lang(langs), "✓ 已复制", "✓ Copied").into()
                } else {
                    t(ui_lang(langs), "复制", "Copy").into()
                }
            },
            tr_opt("复制内容", "Copy value"),
            move || {
                copy_text_with_ctx(&ctx_copy, &value);
                copied.set(true);
            },
        ));
    }
    if let Some(flow_id) = action.select_flow_id
        && let Some(ref on_select) = ctx.on_select_flow
    {
        has_any = true;
        let on_select = Rc::clone(on_select);
        row = row.child(action_chip_button(
            format!("#{flow_id}"),
            move || Some(SharedString::from(format!("Flow #{flow_id}"))),
            move || on_select(flow_id),
        ));
    }
    if let Some(ref filter_q) = action.set_filter
        && let Some(ref on_filter) = ctx.on_set_filter
    {
        has_any = true;
        let query = filter_q.clone();
        let on_filter = Rc::clone(on_filter);
        row = row.child(action_chip_button(
            tr("过滤", "Filter"),
            tr_opt("应用过滤条件", "Apply filter query"),
            move || on_filter(query.clone()),
        ));
    }
    has_any.then(|| row.into_element())
}

fn trigger_primary_action(action: &HookViewAction, ctx: &HookViewActionCtx, copied: Signal<bool>) {
    if let Some(ref copy_val) = action.copy_value {
        copy_text_with_ctx(ctx, copy_val);
        copied.set(true);
    }
    if let Some(flow_id) = action.select_flow_id
        && let Some(ref on_select) = ctx.on_select_flow
    {
        on_select(flow_id);
    }
    if let Some(ref filter_q) = action.set_filter
        && let Some(ref on_filter) = ctx.on_set_filter
    {
        on_filter(filter_q.clone());
    }
}

fn attach_action_row(content: View, action: &HookViewAction, ctx: &HookViewActionCtx) -> View {
    if let Some(strip) = render_action_strip(action, ctx) {
        view()
            .w_full()
            .flex_row()
            .flex_wrap()
            .items_start()
            .justify_between()
            .gap_2()
            .child(content.flex_1().min_w(px(0.0)))
            .child(strip)
    } else {
        content
    }
}

fn render_badge_node(
    tone: PreviewTone,
    label: String,
    action: HookViewAction,
    ctx: &HookViewActionCtx,
) -> AnyElement {
    if action.is_empty() {
        return view()
            .w_full()
            .flex_row()
            .items_center()
            .child(ui_badge(label, map_badge_tone(tone)))
            .into_element();
    }
    let copied = signal(false);
    let has_copy = action.copy_value.is_some();
    let ctx_click = ctx.clone();
    let a11y_label = label.clone();
    button()
        .rounded(px(RADIUS_SM))
        .focus_visible(apply_focus_ring)
        .accessibility_label(move || Some(SharedString::from(a11y_label.clone())))
        .on_press(move |_: &()| trigger_primary_action(&action, &ctx_click, copied))
        .child(ui_badge(
            move || -> SharedString {
                if has_copy && copied.get() {
                    format!("✓ {label}").into()
                } else {
                    label.clone().into()
                }
            },
            map_badge_tone(tone),
        ))
        .into_element()
}

fn render_table_node(columns: Vec<String>, rows: Vec<Vec<String>>) -> AnyElement {
    let mut table = view()
        .w_full()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border_subtle())
        .bg(bg_card())
        .overflow_hidden();
    if !columns.is_empty() {
        let mut head = view()
            .w_full()
            .flex_row()
            .items_center()
            .gap_2()
            .px_2p5()
            .py_1p5()
            .bg(bg_control())
            .border_b_1()
            .border_b_color(border_subtle());
        for col in &columns {
            head = head.child(
                text(col.clone())
                    .flex_1()
                    .min_w(px(48.0))
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(fg_secondary()),
            );
        }
        table = table.child(head);
    }
    let total_rows = rows.len();
    let col_count = columns.len();
    for (r_idx, row_cells) in rows.into_iter().enumerate() {
        let mut row_el = view()
            .w_full()
            .flex_row()
            .items_start()
            .gap_2()
            .px_2p5()
            .py_1p5();
        if r_idx + 1 < total_rows {
            row_el = row_el.border_b_1().border_b_color(border_subtle());
        }
        let target_cols = col_count.max(row_cells.len());
        for c_idx in 0..target_cols {
            let cell_val = row_cells.get(c_idx).cloned().unwrap_or_default();
            row_el = row_el.child(
                mono(cell_val)
                    .flex_1()
                    .min_w(px(48.0))
                    .tabular_nums()
                    .text_size(scaled_px(TEXT_XS))
                    .text_color(fg_primary()),
            );
        }
        table = table.child(row_el);
    }
    table.into_element()
}

fn render_collapsible_node(
    title: String,
    default_open: bool,
    children: Vec<PreviewCardNode>,
    ctx: &HookViewActionCtx,
) -> AnyElement {
    let open = signal(default_open);
    let mut inner = view().w_full().p_2p5().gap_2();
    for child in children {
        inner = inner.child(render_card_node(child, ctx));
    }
    let header_title = if title.is_empty() {
        "Details".to_string()
    } else {
        title
    };
    let a11y_title = header_title.clone();
    collapsible()
        .open(move || open.get())
        .reveal(true)
        .w_full()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border_subtle())
        .bg(bg_card())
        .overflow_hidden()
        .content(view().w_full().child(ui_divider()).child(inner))
        .child(
            button()
                .w_full()
                .px_2p5()
                .py_1p5()
                .flex_row()
                .items_center()
                .gap_1p5()
                .hover(|s| s.bg(bg_control_hover()))
                .active(|s| s.bg(bg_control_active()))
                .transition_colors()
                .duration(move || motion_ms(MOTION_INSTANT_MS))
                .ease_out()
                .focus_visible(apply_focus_ring)
                .accessibility_label(move || Some(SharedString::from(a11y_title.clone())))
                .on_press(move |_: &()| open.update(|v| *v = !*v))
                .child(ui_rotating_chevron(
                    10.0,
                    move || open.get(),
                    || color(fg_secondary()),
                ))
                .child(
                    text(header_title)
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg_primary()),
                ),
        )
        .into_element()
}

fn render_tabs_node(items: Vec<HookViewTabItem>, ctx: &HookViewActionCtx) -> AnyElement {
    if items.is_empty() {
        return view().into_element();
    }
    let active = signal(0usize);
    let total = items.len();
    let mut strip = tabs()
        .orientation(Axis::Horizontal)
        .accessibility_label(tr_opt("脚本标签页", "Hook tabs"))
        .flex_row()
        .flex_wrap()
        .items_center()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border_control())
        .overflow_hidden();
    for (idx, item) in items.iter().enumerate() {
        let lbl = if item.label.is_empty() {
            format!("Tab {}", idx + 1)
        } else {
            item.label.clone()
        };
        let is_last = idx + 1 == total;
        strip = strip.child(
            tab()
                .selected(move || active.get() == idx)
                .h(px(ControlSize::Xs.height()))
                .px_2()
                .flex_row()
                .items_center()
                .border_r(if is_last { px(0.0) } else { px(1.0) })
                .border_r_color(border_control())
                .bg(move || {
                    if active.get() == idx {
                        accent_blue()
                    } else {
                        bg_card()
                    }
                })
                .hover(|s| {
                    s.bg(move || {
                        if active.get() == idx {
                            accent_blue_hover()
                        } else {
                            bg_control_hover()
                        }
                    })
                })
                .focus_visible(apply_focus_ring)
                .on_press(move |_: &()| active.set(idx))
                .child(
                    text(lbl)
                        .number_of_lines(1)
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if active.get() == idx {
                                fg_on_accent()
                            } else {
                                fg_secondary()
                            }
                        }),
                ),
        );
    }
    let panels = items;
    let ctx_panel = ctx.clone();
    view()
        .w_full()
        .gap_2()
        .child(view().flex_row().child(strip))
        .child(dynamic(move || {
            let cur = active.get().min(panels.len().saturating_sub(1));
            let mut body = view().w_full().gap_2();
            if let Some(panel) = panels.get(cur) {
                for child in &panel.children {
                    body = body.child(render_card_node(child.clone(), &ctx_panel));
                }
            }
            body
        }))
        .into_element()
}

fn render_section_node(
    title: String,
    children: Vec<PreviewCardNode>,
    action: HookViewAction,
    ctx: &HookViewActionCtx,
) -> AnyElement {
    let mut body = view().w_full().gap_2();
    if !title.is_empty() || !action.is_empty() {
        let heading = if title.is_empty() {
            view()
        } else {
            view().child(
                text(title)
                    .text_size(px(TEXT_SM))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(fg_primary()),
            )
        };
        body = body.child(attach_action_row(heading, &action, ctx));
    }
    for child in children {
        body = body.child(render_card_node(child, ctx));
    }
    view()
        .w_full()
        .p_3()
        .rounded(px(RADIUS_LG))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .child(body)
        .into_element()
}

fn render_code_node(
    language: Option<String>,
    code: String,
    action: HookViewAction,
    ctx: &HookViewActionCtx,
) -> AnyElement {
    let mut box_view = view()
        .w_full()
        .p_2p5()
        .gap_1()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border_subtle())
        .bg(bg_code());
    let lang_clean = language.filter(|l| !l.trim().is_empty());
    if lang_clean.is_some() || !action.is_empty() {
        let header = match lang_clean {
            Some(lang) => view().child(
                text(lang)
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg_muted()),
            ),
            None => view(),
        };
        box_view = box_view.child(attach_action_row(header, &action, ctx));
    }
    box_view
        .child(
            mono(code)
                .selectable(true)
                .text_size(scaled_px(TEXT_XS))
                .text_color(fg_primary()),
        )
        .into_element()
}

fn render_view_box_node(
    gap: Option<u8>,
    padding: Option<u8>,
    bordered: bool,
    children: Vec<PreviewCardNode>,
    action: HookViewAction,
    ctx: &HookViewActionCtx,
) -> AnyElement {
    let mut container = view()
        .w_full()
        .gap(px(f32::from(gap.unwrap_or(8).min(64))))
        .p(px(f32::from(padding.unwrap_or(0).min(64))));
    if bordered {
        container = container
            .rounded(px(RADIUS_MD))
            .border_1()
            .border_color(border_subtle())
            .bg(bg_card());
    }
    for child in children {
        container = container.child(render_card_node(child, ctx));
    }
    attach_action_row(container, &action, ctx).into_element()
}

fn render_row_node(
    gap: Option<u8>,
    wrap: bool,
    align: Option<String>,
    justify: Option<String>,
    children: Vec<PreviewCardNode>,
    action: HookViewAction,
    ctx: &HookViewActionCtx,
) -> AnyElement {
    let mut row = view()
        .w_full()
        .flex_row()
        .gap(px(f32::from(gap.unwrap_or(8).min(64))));
    if wrap {
        row = row.flex_wrap();
    }
    row = match align.as_deref() {
        Some("start") => row.items_start(),
        Some("end") => row.items_end(),
        _ => row.items_center(),
    };
    row = match justify.as_deref() {
        Some("center") => row.justify_center(),
        Some("end") => row.justify_end(),
        Some("between") => row.justify_between(),
        _ => row.justify_start(),
    };
    for child in children {
        row = row.child(render_card_node(child, ctx));
    }
    attach_action_row(row, &action, ctx).into_element()
}

fn render_callout_node(
    tone: PreviewTone,
    title: Option<String>,
    body_text: String,
    children: Vec<PreviewCardNode>,
    action: HookViewAction,
    ctx: &HookViewActionCtx,
) -> AnyElement {
    let (bg, border, fg) = callout_colors(tone);
    let mut box_view = view()
        .w_full()
        .p_2p5()
        .gap_1p5()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border)
        .bg(bg);
    if let Some(t_str) = title.filter(|t| !t.trim().is_empty()) {
        box_view = box_view.child(
            text(t_str)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg),
        );
    }
    if !body_text.is_empty() {
        box_view = box_view.child(
            text(body_text)
                .selectable(true)
                .text_size(scaled_px(TEXT_XS))
                .text_color(fg_primary()),
        );
    }
    for child in children {
        box_view = box_view.child(render_card_node(child, ctx));
    }
    attach_action_row(box_view, &action, ctx).into_element()
}

fn render_text_node(
    content: String,
    tone: PreviewTone,
    is_mono: bool,
    bold: bool,
    size: Option<TextSizeTier>,
    action: HookViewAction,
    ctx: &HookViewActionCtx,
) -> AnyElement {
    let font_sz = match size.unwrap_or(TextSizeTier::Sm) {
        TextSizeTier::Xs => scaled_px(TEXT_XS),
        TextSizeTier::Sm => scaled_px(TEXT_SM),
        TextSizeTier::Base => scaled_px(TEXT_MD),
    };
    let weight = if bold {
        FontWeight::SEMIBOLD
    } else {
        FontWeight::NORMAL
    };
    let el = if is_mono {
        mono(content)
    } else {
        text(content)
    }
    .selectable(true)
    .text_size(font_sz)
    .font_weight(weight)
    .text_color(tone_fg(tone));
    attach_action_row(view().child(el), &action, ctx).into_element()
}

pub(crate) fn render_card_node(node: PreviewCardNode, ctx: &HookViewActionCtx) -> AnyElement {
    match node {
        PreviewCardNode::Section {
            title,
            children,
            action,
        } => render_section_node(title, children, action, ctx),
        PreviewCardNode::KeyValue {
            label,
            value,
            action,
        } => attach_action_row(
            view()
                .w_full()
                .flex_row()
                .items_start()
                .gap_2p5()
                .child(
                    text(label)
                        .min_w(px(88.0))
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg_secondary()),
                )
                .child(
                    mono(value)
                        .flex_1()
                        .min_w(px(0.0))
                        .selectable(true)
                        .text_size(scaled_px(TEXT_XS))
                        .text_color(fg_primary()),
                ),
            &action,
            ctx,
        )
        .into_element(),
        PreviewCardNode::Badge { tone, text, action } => render_badge_node(tone, text, action, ctx),
        PreviewCardNode::Code {
            language,
            code,
            action,
        } => render_code_node(language, code, action, ctx),
        PreviewCardNode::View {
            gap,
            padding,
            bordered,
            children,
            action,
        } => render_view_box_node(gap, padding, bordered, children, action, ctx),
        PreviewCardNode::Row {
            gap,
            wrap,
            align,
            justify,
            children,
            action,
        } => render_row_node(gap, wrap, align, justify, children, action, ctx),
        PreviewCardNode::Col {
            gap,
            children,
            action,
        } => {
            let mut col = view()
                .w_full()
                .flex_col()
                .gap(px(f32::from(gap.unwrap_or(8).min(64))));
            for child in children {
                col = col.child(render_card_node(child, ctx));
            }
            attach_action_row(col, &action, ctx).into_element()
        }
        PreviewCardNode::Text {
            text: content,
            tone,
            mono: is_mono,
            bold,
            size,
            action,
        } => render_text_node(content, tone, is_mono, bold, size, action, ctx),
        PreviewCardNode::Divider => ui_divider().into_element(),
        PreviewCardNode::Callout {
            tone,
            title,
            text: body_text,
            children,
            action,
        } => render_callout_node(tone, title, body_text, children, action, ctx),
        PreviewCardNode::Table { columns, rows } => render_table_node(columns, rows),
        PreviewCardNode::Collapsible {
            title,
            default_open,
            children,
        } => render_collapsible_node(title, default_open, children, ctx),
        PreviewCardNode::Tabs { items } => render_tabs_node(items, ctx),
    }
}

/// Renders structured hook `view` / `cards` in Inspector regions without host actions.
#[allow(dead_code)]
pub(crate) fn hook_preview_cards(cards: Signal<Vec<PreviewCardNode>>) -> impl IntoElement {
    hook_preview_cards_with_actions(cards, HookViewActionCtx::default())
}

/// Renders structured hook `view` / `cards` with interactive Inspector actions.
pub(crate) fn hook_preview_cards_with_actions(
    cards: Signal<Vec<PreviewCardNode>>,
    ctx: HookViewActionCtx,
) -> impl IntoElement {
    when(
        move || !cards.with(Vec::is_empty),
        move || {
            let ctx = ctx.clone();
            dynamic(move || {
                let items = cards.get();
                let mut stack = view().w_full().gap_2p5();
                for node in items {
                    stack = stack.child(render_card_node(node, &ctx));
                }
                stack
            })
        },
    )
}
