//! Unified split-tree workspace renderer driven by [`crate::layout_spec::LayoutSpec`].

#[allow(unused_imports)]
use super::*;

use crate::layout_spec::{LayoutNode, LayoutSize, LayoutSpec, ViewModule};

const MIN_CUSTOM_PANE_PX: f32 = 120.0;

/// Context flag telling `detail_pane` whether `summary` is mounted independently.
#[derive(Clone, Copy)]
pub(crate) struct StandaloneSummaryContext(pub(crate) Signal<bool>);

/// Context injected into `flow_list` when placed inside a custom `LayoutSpec` slot.
#[derive(Clone, Copy)]
pub(crate) struct FlowListSlotSizing {
    pub(crate) slot_width: Signal<f32>,
}

/// Marker context telling `facet_sidebar` to fill its slot and omit its built-in handle.
#[derive(Clone, Copy)]
pub(crate) struct SidebarSlotSizing;

/// Named UI builders registered by `app_workspace` for each [`ViewModule`].
#[derive(Clone)]
pub(crate) struct WorkspaceModules {
    pub(crate) sidebar: Rc<dyn Fn() -> AnyElement>,
    pub(crate) flow_list: Rc<dyn Fn() -> AnyElement>,
    pub(crate) detail: Rc<dyn Fn() -> AnyElement>,
    pub(crate) summary: Rc<dyn Fn() -> AnyElement>,
}

impl WorkspaceModules {
    /// Instantiates the view element for `module`.
    pub(crate) fn build(&self, module: ViewModule) -> AnyElement {
        match module {
            ViewModule::Sidebar => (self.sidebar)(),
            ViewModule::FlowList => (self.flow_list)(),
            ViewModule::Detail => (self.detail)(),
            ViewModule::Summary => (self.summary)(),
        }
    }
}

/// Shared host state needed by [`render_workspace_layout`] across built-in and custom trees.
#[derive(Clone)]
pub(crate) struct WorkspaceLayoutHost {
    pub(crate) workspace_ref: ElementRef,
    pub(crate) profiles: ProfileSignals,
    pub(crate) list_width: Signal<f32>,
    pub(crate) controls: Arc<Controls>,
    pub(crate) modules: WorkspaceModules,
    pub(crate) pane_overrides: Signal<std::collections::BTreeMap<String, u32>>,
    pub(crate) on_save_pane_size: Rc<dyn Fn(String, u32)>,
}

#[derive(Clone, Copy)]
struct SlotTrack {
    measured: Signal<f32>,
    drag_px: Signal<Option<f32>>,
    slot_width: Option<Signal<f32>>,
    is_sidebar: bool,
    spec_size: LayoutSize,
}

/// Standalone scrollable card pane for `ViewModule::Summary`.
pub(crate) fn summary_pane(
    stats: Signal<session::FilteredStats>,
    selected: Signal<Option<u64>>,
    controls: Arc<Controls>,
    rows: Signal<Vec<Arc<RowLine>>>,
    seq_builder: SeqBuilder,
    compose: Compose,
    shown_phase: Signal<Phase>,
) -> View {
    view()
        .flex_1()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .bg(bg_canvas())
        .child(
            scroll()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .bg(bg_canvas())
                .child(
                    uncull_nested_scroll(view()).w_full().p_3().child(
                        when(
                            move || stats.with(|s| s.total_count > 0),
                            move || {
                                let c = controls.clone();
                                filtered_summary_card(stats, selected, move || {
                                    begin_sequence_diagram(
                                        &c,
                                        rows,
                                        seq_builder,
                                        compose,
                                        selected,
                                        shown_phase,
                                    );
                                })
                            },
                        )
                        .otherwise(move || {
                            view()
                                .p_3p5()
                                .gap_1p5()
                                .rounded(px(RADIUS_LG))
                                .border_1()
                                .border_color(border_strong())
                                .bg(bg_card())
                                .child(
                                    text(tr(
                                        "当前列表暂无统计数据",
                                        "No filtered traffic summary yet",
                                    ))
                                    .text_size(px(TEXT_SM))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(fg_primary()),
                                )
                                .child(
                                    text(tr(
                                        "捕获或加载请求后，将在此处展示状态分布与生成时序图入口。",
                                        "Status metrics and the sequence diagram entry appear once flows are visible.",
                                    ))
                                    .text_size(px(TEXT_XS))
                                    .text_color(fg_muted()),
                                )
                        }),
                    ),
                ),
        )
}

/// Renders the active [`LayoutSpec`], rebuilding only when the spec itself changes.
pub(crate) fn render_workspace_layout(spec: Signal<LayoutSpec>, host: WorkspaceLayoutHost) -> View {
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .child(dynamic(move || {
            let current = spec.get();
            if current.is_builtin_split() {
                render_builtin_split(&host).into_element()
            } else if current.is_builtin_top_bottom() {
                render_builtin_top_bottom(&host).into_element()
            } else {
                render_custom_root(&current.root, &host).into_element()
            }
        }))
}

fn render_builtin_split(host: &WorkspaceLayoutHost) -> View {
    let sidebar_collapsed = host.profiles.sidebar_collapsed;
    let build_sidebar = Rc::clone(&host.modules.sidebar);
    view()
        .element_ref(&host.workspace_ref)
        .flex_1()
        .min_h(px(0.0))
        .flex_row()
        .child(when(
            move || !sidebar_effectively_collapsed(sidebar_collapsed.get(), window().width.0),
            move || (build_sidebar)(),
        ))
        .child((host.modules.flow_list)())
        .child(pane_splitter(
            host.list_width,
            host.profiles.list_height,
            host.profiles.layout_mode,
            host.profiles.sidebar_width,
            host.profiles.sidebar_collapsed,
            host.controls.clone(),
        ))
        .child((host.modules.detail)())
}

fn render_builtin_top_bottom(host: &WorkspaceLayoutHost) -> View {
    view()
        .element_ref(&host.workspace_ref)
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .child((host.modules.flow_list)())
        .child(pane_splitter(
            host.list_width,
            host.profiles.list_height,
            host.profiles.layout_mode,
            host.profiles.sidebar_width,
            host.profiles.sidebar_collapsed,
            host.controls.clone(),
        ))
        .child((host.modules.detail)())
}

fn render_custom_root(root: &LayoutNode, host: &WorkspaceLayoutHost) -> View {
    if root.contains_module(ViewModule::Sidebar) {
        provide_context(SidebarSlotSizing);
    }
    let flow_slot_width = root
        .contains_module(ViewModule::FlowList)
        .then(|| signal(0.0_f32));
    if let Some(slot_width) = flow_slot_width {
        provide_context(FlowListSlotSizing { slot_width });
    }

    let base = view()
        .element_ref(&host.workspace_ref)
        .flex_1()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .min_h(px(0.0));
    match root {
        LayoutNode::Row { children, .. } => populate_custom_container(
            base.flex_row(),
            Axis::Horizontal,
            "root",
            children,
            flow_slot_width,
            host,
        ),
        LayoutNode::Col { children, .. } => populate_custom_container(
            base.flex_col(),
            Axis::Vertical,
            "root",
            children,
            flow_slot_width,
            host,
        ),
        LayoutNode::Pane { module, .. } => {
            let content = host.modules.build(*module);
            let slot = if let (ViewModule::FlowList, Some(width_sig)) = (*module, flow_slot_width) {
                base.on_layout(move |bounds: &Bounds<Points>| {
                    let w = bounds.size.width.0;
                    if w > 0.0 && (w - width_sig.get_untracked()).abs() > 1.0 {
                        width_sig.set(w);
                    }
                })
            } else {
                base
            };
            slot.flex_col().child(content)
        }
    }
}

fn render_custom_child(
    node: &LayoutNode,
    parent_axis: Axis,
    node_path: &str,
    track: SlotTrack,
    flow_slot_width: Option<Signal<f32>>,
    host: &WorkspaceLayoutHost,
) -> View {
    let inner = match node {
        LayoutNode::Row { children, .. } => populate_custom_container(
            view().flex_row(),
            Axis::Horizontal,
            node_path,
            children,
            flow_slot_width,
            host,
        ),
        LayoutNode::Col { children, .. } => populate_custom_container(
            view().flex_col(),
            Axis::Vertical,
            node_path,
            children,
            flow_slot_width,
            host,
        ),
        LayoutNode::Pane { module, .. } => {
            let content = host.modules.build(*module);
            view()
                .flex_col()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .child(content)
        }
    };

    let measured = track.measured;
    let slot_width = track.slot_width;
    let sized = apply_slot_layout(inner, parent_axis, track.spec_size, track.drag_px);
    sized.on_layout(move |bounds: &Bounds<Points>| {
        let extent = match parent_axis {
            Axis::Horizontal => bounds.size.width.0,
            Axis::Vertical => bounds.size.height.0,
        };
        if extent > 0.0 && (extent - measured.get_untracked()).abs() > 1.0 {
            measured.set(extent);
        }
        if let Some(w_sig) = slot_width {
            let w = bounds.size.width.0;
            if w > 0.0 && (w - w_sig.get_untracked()).abs() > 1.0 {
                w_sig.set(w);
            }
        }
    })
}

fn populate_custom_container(
    mut container: View,
    axis: Axis,
    parent_path: &str,
    children: &[LayoutNode],
    flow_slot_width: Option<Signal<f32>>,
    host: &WorkspaceLayoutHost,
) -> View {
    let tracks: Vec<(String, SlotTrack)> = children
        .iter()
        .enumerate()
        .map(|(idx, child)| {
            let child_path = format!("{parent_path}[{idx}]");
            let init_drag = host
                .pane_overrides
                .with_untracked(|map| map.get(&child_path).copied().map(|v| v as f32));
            let track = SlotTrack {
                measured: signal(0.0_f32),
                drag_px: signal(init_drag),
                slot_width: matches!(
                    child,
                    LayoutNode::Pane {
                        module: ViewModule::FlowList,
                        ..
                    }
                )
                .then_some(flow_slot_width)
                .flatten(),
                is_sidebar: matches!(
                    child,
                    LayoutNode::Pane {
                        module: ViewModule::Sidebar,
                        ..
                    }
                ),
                spec_size: child.size(),
            };
            (child_path, track)
        })
        .collect();

    let pane_overrides = host.pane_overrides;
    let track_sync: Vec<(String, Signal<Option<f32>>)> = tracks
        .iter()
        .map(|(path, track)| (path.clone(), track.drag_px))
        .collect();
    effect(move || {
        pane_overrides.with(|map| {
            for (path, drag_px) in &track_sync {
                let expected = map.get(path).copied().map(|v| v as f32);
                if drag_px.get_untracked() != expected {
                    drag_px.set(expected);
                }
            }
        });
    });

    let sidebar_collapsed = host.profiles.sidebar_collapsed;
    for (idx, child) in children.iter().enumerate() {
        let (child_path, track) = (&tracks[idx].0, tracks[idx].1);
        if idx > 0 {
            let (prev_path, prev) = (&tracks[idx - 1].0, tracks[idx - 1].1);
            let leading_path = prev_path.clone();
            let trailing_path = child_path.clone();
            let on_save = Rc::clone(&host.on_save_pane_size);
            if prev.is_sidebar || track.is_sidebar {
                container = container.child(when(
                    move || {
                        !sidebar_effectively_collapsed(sidebar_collapsed.get(), window().width.0)
                    },
                    move || {
                        custom_splitter(
                            axis,
                            prev,
                            leading_path.clone(),
                            track,
                            trailing_path.clone(),
                            Rc::clone(&on_save),
                        )
                    },
                ));
            } else {
                container = container.child(custom_splitter(
                    axis,
                    prev,
                    leading_path,
                    track,
                    trailing_path,
                    on_save,
                ));
            }
        }

        if track.is_sidebar {
            let child_clone = child.clone();
            let child_path_owned = child_path.clone();
            let host_clone = host.clone();
            container = container.child(when(
                move || !sidebar_effectively_collapsed(sidebar_collapsed.get(), window().width.0),
                move || {
                    render_custom_child(
                        &child_clone,
                        axis,
                        &child_path_owned,
                        track,
                        flow_slot_width,
                        &host_clone,
                    )
                },
            ));
        } else {
            container = container.child(render_custom_child(
                child,
                axis,
                child_path,
                track,
                flow_slot_width,
                host,
            ));
        }
    }
    container
}

fn apply_slot_layout(
    slot: View,
    axis: Axis,
    spec_size: LayoutSize,
    drag_override: Signal<Option<f32>>,
) -> View {
    let cross_full = match axis {
        Axis::Horizontal => slot.h_full(),
        Axis::Vertical => slot.w_full(),
    };
    cross_full
        .min_w(px(0.0))
        .min_h(px(0.0))
        .overflow_hidden()
        .style_with(move |s| {
            if let Some(px_val) = drag_override.get() {
                let s = s.flex_grow(0.0).flex_shrink(0.0).basis(px(px_val));
                return match axis {
                    Axis::Horizontal => s.w(px(px_val)),
                    Axis::Vertical => s.h(px(px_val)),
                };
            }
            match spec_size {
                LayoutSize::Px(v) => {
                    let s = s.flex_grow(0.0).flex_shrink(0.0).basis(px(v));
                    match axis {
                        Axis::Horizontal => s.w(px(v)),
                        Axis::Vertical => s.h(px(v)),
                    }
                }
                LayoutSize::Percent(p) => {
                    let frac = (p / 100.0).clamp(0.01, 1.0);
                    let s = s.flex_grow(0.0).flex_shrink(0.0).basis(relative(frac));
                    match axis {
                        Axis::Horizontal => s.w(relative(frac)),
                        Axis::Vertical => s.h(relative(frac)),
                    }
                }
                LayoutSize::Fr(f) => s.flex_grow(f).flex_shrink(1.0).basis(px(0.0)),
                LayoutSize::Auto => s.flex_1(),
            }
        })
}

fn custom_splitter(
    axis: Axis,
    leading: SlotTrack,
    leading_path: String,
    trailing: SlotTrack,
    trailing_path: String,
    on_save: Rc<dyn Fn(String, u32)>,
) -> impl IntoElement {
    let resize_trailing = is_fixed_size(trailing.spec_size) && !is_fixed_size(leading.spec_size);
    let (target_drag, target_path) = if resize_trailing {
        (trailing.drag_px, trailing_path)
    } else {
        (leading.drag_px, leading_path)
    };
    let origin = signal(0.0_f32);
    let max_extent = signal(MIN_CUSTOM_PANE_PX);
    let state = signal(ResizeHandleState::Idle);
    let handle = resize_handle()
        .axis(axis)
        .accessibility_label(match axis {
            Axis::Horizontal => tr_opt("调整分栏宽度", "Resize pane width"),
            Axis::Vertical => tr_opt("调整分栏高度", "Resize pane height"),
        })
        .on_state_change(move |next: &ResizeHandleState| state.set(*next))
        .on_drag_start(move || {
            let lead_px = leading.measured.get_untracked().max(MIN_CUSTOM_PANE_PX);
            let trail_px = trailing.measured.get_untracked().max(MIN_CUSTOM_PANE_PX);
            let total = (lead_px + trail_px - MIN_CUSTOM_PANE_PX).max(MIN_CUSTOM_PANE_PX);
            max_extent.set(total);
            origin.set(if resize_trailing { trail_px } else { lead_px });
        })
        .on_drag_move(move |offset: &Points| {
            let delta = if resize_trailing { -offset.0 } else { offset.0 };
            let next = (origin.get() + delta).clamp(MIN_CUSTOM_PANE_PX, max_extent.get());
            target_drag.set(Some(next));
        })
        .on_drag_end(move |offset: &Points| {
            let delta = if resize_trailing { -offset.0 } else { offset.0 };
            let next = (origin.get() + delta).clamp(MIN_CUSTOM_PANE_PX, max_extent.get());
            let clamped = config::clamp_layout_pane_size(next.round() as u32);
            target_drag.set(Some(clamped as f32));
            (on_save)(target_path.clone(), clamped);
        })
        .child(splitter_indicator(axis, state, 3.0));

    let shell = view().relative().z_index(10).flex_shrink_0();
    match axis {
        Axis::Horizontal => shell.w(px(1.0)).self_stretch().child(handle),
        Axis::Vertical => shell.w_full().h(px(1.0)).child(handle),
    }
}

const fn is_fixed_size(size: LayoutSize) -> bool {
    matches!(size, LayoutSize::Px(_) | LayoutSize::Percent(_))
}
