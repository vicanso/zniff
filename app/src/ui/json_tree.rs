#[allow(unused_imports)]
use super::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub(crate) const JSON_ROOT_ID: &str = "$";
pub(crate) const MAX_JSON_TREE_NODES: usize = 20_000;
const TREE_ROW_H: f32 = 22.0;
const MAX_TREE_ROWS: usize = 13;
const TREE_CHROME: f32 = 12.0;
const MAX_PREVIEW_CHARS: usize = 320;
const TRUNCATED_PREFIX: &str = "__truncated:";

#[derive(Clone)]
pub(crate) struct JsonBody {
    pub(crate) flow_id: u64,
    pub(crate) body_hash: u64,
    pub(crate) value: Arc<serde_json::Value>,
}

impl PartialEq for JsonBody {
    fn eq(&self, other: &Self) -> bool {
        self.flow_id == other.flow_id
            && self.body_hash == other.body_hash
            && Arc::ptr_eq(&self.value, &other.value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JsonNodeKind {
    Object(usize),
    Array(usize),
    EmptyObject,
    EmptyArray,
    Str,
    Num,
    Literal,
    Truncated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsonNodeMeta {
    pub(crate) key: SharedString,
    pub(crate) is_index: bool,
    pub(crate) kind: JsonNodeKind,
    pub(crate) preview: SharedString,
}

pub(crate) struct JsonTreeView {
    pub(crate) items: Vec<TreeItem>,
    pub(crate) meta: HashMap<SharedString, JsonNodeMeta>,
    pub(crate) auto_expand_ids: Vec<SharedString>,
}

#[derive(Clone)]
struct TreeViewArc(Arc<JsonTreeView>);

impl PartialEq for TreeViewArc {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

struct BuildCtx {
    meta: HashMap<SharedString, JsonNodeMeta>,
    auto_expand_ids: Vec<SharedString>,
    remaining: usize,
}

fn body_content_hash(content_type: Option<&str>, body: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    content_type.hash(&mut hasher);
    body.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn update_json_body_signal(
    sig: Signal<Option<JsonBody>>,
    flow_id: u64,
    content_type: Option<&str>,
    body: &[u8],
    skip: bool,
) {
    if skip || body.is_empty() {
        if sig.with_untracked(Option::is_some) {
            sig.set(None);
        }
        return;
    }
    let body_hash = body_content_hash(content_type, body);
    let unchanged = sig.with_untracked(|prev| {
        prev.as_ref()
            .is_some_and(|p| p.flow_id == flow_id && p.body_hash == body_hash)
    });
    if unchanged {
        return;
    }
    let next = present::parse_json_body(content_type, body).map(|value| JsonBody {
        flow_id,
        body_hash,
        value: Arc::new(value),
    });
    sig.set(next);
}

fn escape_json_pointer(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    for ch in token.chars() {
        match ch {
            '~' => out.push_str("~0"),
            '/' => out.push_str("~1"),
            _ => out.push(ch),
        }
    }
    out
}

pub(crate) fn copy_json_node_value(root: &serde_json::Value, id: &str) -> Option<String> {
    if id.starts_with(TRUNCATED_PREFIX) {
        return None;
    }
    let ptr = id.strip_prefix(JSON_ROOT_ID)?;
    let target = root.pointer(ptr)?;
    match target {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Null => Some(String::from("null")),
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
            serde_json::to_string_pretty(target).ok()
        }
    }
}

fn format_string_preview(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(MAX_PREVIEW_CHARS) + 4);
    out.push('"');
    let mut truncated = false;
    for (count, ch) in s.chars().enumerate() {
        if count >= MAX_PREVIEW_CHARS {
            truncated = true;
            break;
        }
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    if truncated {
        out.push('…');
    }
    out.push('"');
    out
}

fn scalar_kind_and_preview(val: &serde_json::Value) -> (JsonNodeKind, SharedString) {
    match val {
        serde_json::Value::String(s) => (
            JsonNodeKind::Str,
            SharedString::from(format_string_preview(s)),
        ),
        serde_json::Value::Number(n) => (JsonNodeKind::Num, SharedString::from(n.to_string())),
        serde_json::Value::Bool(b) => (
            JsonNodeKind::Literal,
            SharedString::from(if *b { "true" } else { "false" }),
        ),
        serde_json::Value::Null => (JsonNodeKind::Literal, SharedString::from("null")),
        serde_json::Value::Object(_) => (JsonNodeKind::EmptyObject, SharedString::from("{}")),
        serde_json::Value::Array(_) => (JsonNodeKind::EmptyArray, SharedString::from("[]")),
    }
}

fn make_truncated_item(parent_id: &str, ctx: &mut BuildCtx) -> TreeItem {
    let id = SharedString::from(format!("{TRUNCATED_PREFIX}{parent_id}"));
    ctx.meta.insert(
        id.clone(),
        JsonNodeMeta {
            key: SharedString::default(),
            is_index: false,
            kind: JsonNodeKind::Truncated,
            preview: SharedString::default(),
        },
    );
    TreeItem::new(id, "…").disabled(true)
}

fn build_unfiltered_children(
    parent_id: &str,
    val: &serde_json::Value,
    ctx: &mut BuildCtx,
) -> Vec<TreeItem> {
    let mut children = Vec::new();
    match val {
        serde_json::Value::Object(map) => {
            children.reserve(map.len().min(64));
            for (k, child) in map {
                if ctx.remaining == 0 {
                    children.push(make_truncated_item(parent_id, ctx));
                    break;
                }
                let child_id = format!("{parent_id}/{}", escape_json_pointer(k));
                if let Some(item) = build_unfiltered_node(child_id, k.clone(), false, child, ctx) {
                    children.push(item);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            children.reserve(arr.len().min(64));
            for (idx, child) in arr.iter().enumerate() {
                if ctx.remaining == 0 {
                    children.push(make_truncated_item(parent_id, ctx));
                    break;
                }
                let child_id = format!("{parent_id}/{idx}");
                if let Some(item) =
                    build_unfiltered_node(child_id, format!("[{idx}]"), true, child, ctx)
                {
                    children.push(item);
                }
            }
        }
        _ => {}
    }
    children
}

fn build_unfiltered_node(
    id: String,
    key: String,
    is_index: bool,
    val: &serde_json::Value,
    ctx: &mut BuildCtx,
) -> Option<TreeItem> {
    if ctx.remaining == 0 {
        return None;
    }
    ctx.remaining -= 1;
    let id_shared = SharedString::from(id.clone());
    let key_shared = SharedString::from(key.clone());
    let (kind, len) = match val {
        serde_json::Value::Object(map) if !map.is_empty() => {
            (JsonNodeKind::Object(map.len()), Some(map.len()))
        }
        serde_json::Value::Array(arr) if !arr.is_empty() => {
            (JsonNodeKind::Array(arr.len()), Some(arr.len()))
        }
        _ => {
            let (kind, preview) = scalar_kind_and_preview(val);
            ctx.meta.insert(
                id_shared.clone(),
                JsonNodeMeta {
                    key: key_shared,
                    is_index,
                    kind,
                    preview: preview.clone(),
                },
            );
            return Some(TreeItem::new(id_shared, format!("{key}: {preview}")));
        }
    };
    let children = build_unfiltered_children(&id, val, ctx);
    ctx.meta.insert(
        id_shared.clone(),
        JsonNodeMeta {
            key: key_shared,
            is_index,
            kind,
            preview: SharedString::default(),
        },
    );
    let len = len.unwrap_or(0);
    Some(TreeItem::new(id_shared, format!("{key} ({len})")).children(children))
}

fn node_self_matches(key: &str, val: &serde_json::Value, q_lower: &str) -> bool {
    if key.to_lowercase().contains(q_lower) {
        return true;
    }
    match val {
        serde_json::Value::String(s) => s.to_lowercase().contains(q_lower),
        serde_json::Value::Number(n) => n.to_string().to_lowercase().contains(q_lower),
        serde_json::Value::Bool(b) => (if *b { "true" } else { "false" }).contains(q_lower),
        serde_json::Value::Null => "null".contains(q_lower),
        serde_json::Value::Object(map) if map.is_empty() => "{}".contains(q_lower),
        serde_json::Value::Array(arr) if arr.is_empty() => "[]".contains(q_lower),
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => false,
    }
}

fn child_has_match(key: &str, val: &serde_json::Value, q_lower: &str) -> bool {
    if node_self_matches(key, val, q_lower) {
        return true;
    }
    match val {
        serde_json::Value::Object(map) => map.iter().any(|(k, v)| child_has_match(k, v, q_lower)),
        serde_json::Value::Array(arr) => arr
            .iter()
            .enumerate()
            .any(|(i, v)| child_has_match(&format!("[{i}]"), v, q_lower)),
        _ => false,
    }
}

fn build_filtered_container(
    id: &str,
    val: &serde_json::Value,
    q_lower: &str,
    ctx: &mut BuildCtx,
) -> Vec<TreeItem> {
    let mut children = Vec::new();
    let mut child_truncated = false;
    let mut push_child =
        |child_id: String, child_key: String, is_index: bool, child: &serde_json::Value| {
            if let Some(item) =
                build_filtered_node(child_id, child_key.clone(), is_index, child, q_lower, ctx)
            {
                children.push(item);
            } else if ctx.remaining == 0
                && !child_truncated
                && child_has_match(&child_key, child, q_lower)
            {
                child_truncated = true;
            }
        };
    match val {
        serde_json::Value::Object(map) => {
            for (k, child) in map {
                push_child(
                    format!("{id}/{}", escape_json_pointer(k)),
                    k.clone(),
                    false,
                    child,
                );
            }
        }
        serde_json::Value::Array(arr) => {
            for (idx, child) in arr.iter().enumerate() {
                push_child(format!("{id}/{idx}"), format!("[{idx}]"), true, child);
            }
        }
        _ => {}
    }
    if child_truncated {
        children.push(make_truncated_item(id, ctx));
    }
    children
}

fn build_filtered_node(
    id: String,
    key: String,
    is_index: bool,
    val: &serde_json::Value,
    q_lower: &str,
    ctx: &mut BuildCtx,
) -> Option<TreeItem> {
    let self_matches = node_self_matches(&key, val, q_lower);
    let (total_len, kind) = match val {
        serde_json::Value::Object(map) if !map.is_empty() => {
            (map.len(), JsonNodeKind::Object(map.len()))
        }
        serde_json::Value::Array(arr) if !arr.is_empty() => {
            (arr.len(), JsonNodeKind::Array(arr.len()))
        }
        _ => {
            if !self_matches || ctx.remaining == 0 {
                return None;
            }
            ctx.remaining -= 1;
            let (kind, preview) = scalar_kind_and_preview(val);
            let id_shared = SharedString::from(id);
            ctx.meta.insert(
                id_shared.clone(),
                JsonNodeMeta {
                    key: SharedString::from(key.clone()),
                    is_index,
                    kind,
                    preview: preview.clone(),
                },
            );
            return Some(TreeItem::new(id_shared, format!("{key}: {preview}")));
        }
    };

    let mut children = build_filtered_container(&id, val, q_lower, ctx);
    let has_matching_descendant = !children.is_empty();
    if (!has_matching_descendant && !self_matches) || ctx.remaining == 0 {
        return None;
    }
    ctx.remaining -= 1;
    let id_shared = SharedString::from(id.clone());
    if has_matching_descendant {
        ctx.auto_expand_ids.push(id_shared.clone());
    } else {
        children = build_unfiltered_children(&id, val, ctx);
    }
    ctx.meta.insert(
        id_shared.clone(),
        JsonNodeMeta {
            key: SharedString::from(key.clone()),
            is_index,
            kind,
            preview: SharedString::default(),
        },
    );
    Some(TreeItem::new(id_shared, format!("{key} ({total_len})")).children(children))
}

pub(crate) fn build_json_tree_with_limit(
    value: &serde_json::Value,
    query: &str,
    limit: usize,
) -> JsonTreeView {
    let mut ctx = BuildCtx {
        meta: HashMap::new(),
        auto_expand_ids: Vec::new(),
        remaining: limit.max(1),
    };
    let q_lower = query.trim().to_lowercase();
    let item = if q_lower.is_empty() {
        let root_item = build_unfiltered_node(
            JSON_ROOT_ID.to_string(),
            JSON_ROOT_ID.to_string(),
            false,
            value,
            &mut ctx,
        );
        if root_item.as_ref().is_some_and(|it| !it.children.is_empty()) {
            ctx.auto_expand_ids.push(SharedString::from(JSON_ROOT_ID));
        }
        root_item
    } else {
        build_filtered_node(
            JSON_ROOT_ID.to_string(),
            JSON_ROOT_ID.to_string(),
            false,
            value,
            &q_lower,
            &mut ctx,
        )
    };
    JsonTreeView {
        items: item.into_iter().collect(),
        meta: ctx.meta,
        auto_expand_ids: ctx.auto_expand_ids,
    }
}

pub(crate) fn build_json_tree_view(value: &serde_json::Value, query: &str) -> JsonTreeView {
    build_json_tree_with_limit(value, query, MAX_JSON_TREE_NODES)
}

fn count_matches_in_value(key: &str, val: &serde_json::Value, q_lower: &str, hits: &mut usize) {
    if node_self_matches(key, val, q_lower) {
        *hits += 1;
    }
    match val {
        serde_json::Value::Object(map) => {
            for (k, child) in map {
                count_matches_in_value(k, child, q_lower, hits);
            }
        }
        serde_json::Value::Array(arr) => {
            for (idx, child) in arr.iter().enumerate() {
                let k = format!("[{idx}]");
                count_matches_in_value(&k, child, q_lower, hits);
            }
        }
        _ => {}
    }
}

pub(crate) fn json_tree_search_hits(jb: &JsonBody, query: &str) -> usize {
    let q_lower = query.trim().to_lowercase();
    if q_lower.is_empty() {
        return 0;
    }
    let mut hits = 0usize;
    count_matches_in_value(JSON_ROOT_ID, &jb.value, &q_lower, &mut hits);
    hits
}

pub(crate) fn visible_tree_row_count(items: &[TreeItem], expanded: &[SharedString]) -> usize {
    let mut count = 0usize;
    for item in items {
        count += 1;
        if count > MAX_TREE_ROWS {
            return count;
        }
        if !item.children.is_empty() && expanded.iter().any(|id| id == &item.id) {
            count += visible_tree_row_count(&item.children, expanded);
            if count > MAX_TREE_ROWS {
                return count;
            }
        }
    }
    count
}

pub(crate) fn tree_box_height(visible_rows: usize) -> f32 {
    let shown = visible_rows.clamp(1, MAX_TREE_ROWS) as f32;
    shown * TREE_ROW_H + TREE_CHROME
}

fn folder_summary(kind: JsonNodeKind, lang: Lang) -> String {
    match (kind, lang) {
        (JsonNodeKind::Object(n), Lang::Zh) => format!("{n} 个字段"),
        (JsonNodeKind::Object(1), Lang::En) => String::from("1 field"),
        (JsonNodeKind::Object(n), Lang::En) => format!("{n} fields"),
        (JsonNodeKind::Array(n), Lang::Zh) => format!("{n} 项"),
        (JsonNodeKind::Array(1), Lang::En) => String::from("1 item"),
        (JsonNodeKind::Array(n), Lang::En) => format!("{n} items"),
        _ => String::new(),
    }
}

fn value_color(kind: JsonNodeKind) -> AdaptiveColor {
    match kind {
        JsonNodeKind::Str => json_string_fg(),
        JsonNodeKind::Num => json_number_fg(),
        JsonNodeKind::Literal => json_literal_fg(),
        _ => fg_muted(),
    }
}

fn render_node_row(
    depth: u32,
    folder: bool,
    meta: JsonNodeMeta,
    langs: Signal<Option<Lang>>,
    is_selected: Memo<bool>,
) -> impl IntoElement {
    let key_color = if meta.is_index {
        fg_muted()
    } else {
        fg_primary()
    };
    let mut row = view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .pl(px(10.0 + depth as f32 * 16.0))
        .pr_3()
        .bg(move || {
            if is_selected.get() {
                accent_blue_soft()
            } else {
                bg_code()
            }
        })
        .hover(move |h| {
            h.bg(move || {
                if is_selected.get() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .group_focus_visible(|fv| fv.border_l(px(2.0)).border_l_color(accent_blue()));

    if folder {
        row = row.child(
            view()
                .w(px(16.0))
                .h(px(16.0))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .app_class("transition-transform group-aria-expanded:rotate-90")
                .child(glyph_px(11.0, || Glyph::ChevronRight, || color(fg_muted()))),
        );
    } else {
        row = row.child(view().w(px(16.0)).h(px(16.0)).flex_shrink_0());
    }

    match meta.kind {
        JsonNodeKind::Truncated => row.child(
            mono(tr("… 节点过多，已省略剩余项", "… Remaining nodes omitted"))
                .text_size(px(11.5))
                .text_color(fg_muted()),
        ),
        JsonNodeKind::Object(_) | JsonNodeKind::Array(_) => row
            .gap_1p5()
            .child(
                mono(meta.key)
                    .flex_shrink_0()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(key_color),
            )
            .child(
                mono(move || folder_summary(meta.kind, ui_lang(langs)))
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail)
                    .text_size(px(11.5))
                    .text_color(fg_muted()),
            ),
        kind => row
            .child(
                mono(meta.key)
                    .flex_shrink_0()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(key_color),
            )
            .child(
                mono(" : ")
                    .flex_shrink_0()
                    .text_size(px(12.0))
                    .text_color(fg_muted()),
            )
            .child(
                mono(meta.preview)
                    .flex_1()
                    .min_w(px(0.0))
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail)
                    .text_size(px(12.0))
                    .text_color(value_color(kind)),
            ),
    }
}

fn build_tree_memos(
    json: Signal<Option<JsonBody>>,
    search: Signal<String>,
    user_expanded: Signal<Vec<SharedString>>,
    search_expanded: Signal<Option<(String, Vec<SharedString>)>>,
    selected_node: Signal<Option<SharedString>>,
) -> (Memo<TreeViewArc>, Memo<bool>, Memo<Vec<SharedString>>) {
    let last_flow_id = stored(None::<u64>);
    effect(move || {
        let now_flow = json.with(|jb| jb.as_ref().map(|b| b.flow_id));
        untracked(|| {
            if last_flow_id.get() != now_flow {
                last_flow_id.replace(now_flow);
                user_expanded.set(vec![SharedString::from(JSON_ROOT_ID)]);
                search_expanded.set(None);
                selected_node.set(None);
            }
        });
    });

    let tree_view = computed(move || {
        let q = search.get();
        let built = json.with(|jb| match jb.as_ref() {
            Some(body) => build_json_tree_view(&body.value, &q),
            None => JsonTreeView {
                items: Vec::new(),
                meta: HashMap::new(),
                auto_expand_ids: Vec::new(),
            },
        });
        TreeViewArc(Arc::new(built))
    });

    let is_empty = computed(move || tree_view.with(|v| v.0.items.is_empty()));

    let effective_expanded = computed(move || {
        let q = search.get();
        let trimmed = q.trim();
        if trimmed.is_empty() {
            user_expanded.get()
        } else if let Some(saved) = search_expanded.with(|s| {
            s.as_ref()
                .filter(|(sq, _)| sq == trimmed)
                .map(|(_, ids)| ids.clone())
        }) {
            saved
        } else {
            tree_view.with(|v| v.0.auto_expand_ids.clone())
        }
    });

    (tree_view, is_empty, effective_expanded)
}

fn empty_tree_placeholder() -> impl IntoElement {
    view().size_full().items_center().justify_center().child(
        text(tr("无匹配节点", "No matching nodes"))
            .text_size(px(12.0))
            .text_color(fg_muted()),
    )
}

fn tree_container_height(
    is_empty: Memo<bool>,
    tree_view: Memo<TreeViewArc>,
    effective_expanded: Memo<Vec<SharedString>>,
) -> Points {
    if is_empty.get() {
        px(body_box_height(2))
    } else {
        let rows = tree_view
            .with(|v| effective_expanded.with(|exp| visible_tree_row_count(&v.0.items, exp)));
        px(tree_box_height(rows))
    }
}

pub(crate) fn json_tree_body(
    json: Signal<Option<JsonBody>>,
    search: Signal<String>,
    side: Side,
    controls: Arc<Controls>,
) -> impl IntoElement {
    json_tree_panel(json, search, move |payload| {
        set_clipboard(payload);
        set_status(&controls, StatusMsg::CopiedBody { side, hex: false });
    })
}

pub(crate) fn json_tree_panel(
    json: Signal<Option<JsonBody>>,
    search: Signal<String>,
    on_copy: impl Fn(String) + Clone + 'static,
) -> impl IntoElement {
    let langs = ui_langs();
    let user_expanded = signal(vec![SharedString::from(JSON_ROOT_ID)]);
    let search_expanded = signal(None::<(String, Vec<SharedString>)>);
    let selected_node = signal(None::<SharedString>);
    let (tree_view, is_empty, effective_expanded) =
        build_tree_memos(json, search, user_expanded, search_expanded, selected_node);

    guard_culled_pointer_events(view())
        .w_full()
        .h(move || tree_container_height(is_empty, tree_view, effective_expanded))
        .bg(bg_code())
        .overflow_hidden()
        .child(dynamic(move || {
            if is_empty.get() {
                return empty_tree_placeholder().into_element();
            }
            let on_copy = on_copy.clone();
            tree()
                .size_full()
                .accessibility_label("JSON Tree")
                .items(move || tree_view.with(|v| v.0.items.clone()))
                .expanded(move || Some(effective_expanded.get()))
                .selected(move || selected_node.get())
                .on_expanded_change(move |ids: &Vec<SharedString>| {
                    let q = search.get_untracked();
                    let trimmed = q.trim();
                    if trimmed.is_empty() {
                        user_expanded.set(ids.clone());
                    } else {
                        search_expanded.set(Some((trimmed.to_string(), ids.clone())));
                    }
                })
                .on_selected_change(move |id: &SharedString| {
                    selected_node.set(Some(id.clone()));
                })
                .on_action(move |id: &SharedString| {
                    let copied = json.with_untracked(|jb| {
                        jb.as_ref()
                            .and_then(|body| copy_json_node_value(&body.value, id.as_ref()))
                    });
                    if let Some(payload) = copied {
                        on_copy(payload);
                    }
                })
                .list_style(|s| s.py_1p5())
                .item_class("group")
                .item_style(|s| {
                    s.w_full()
                        .h(px(TREE_ROW_H))
                        .flex_row()
                        .items_center()
                        .cursor(Cursor::Pointer)
                })
                .render_item(move |entry: &TreeEntry| {
                    let node_id = entry.id.clone();
                    let sel_id = entry.id.clone();
                    let depth = entry.depth;
                    let folder = entry.folder;
                    let is_selected =
                        computed(move || selected_node.with(|s| s.as_ref() == Some(&sel_id)));
                    let node_meta =
                        computed(move || tree_view.with(|v| v.0.meta.get(&node_id).cloned()));
                    dynamic(move || {
                        node_meta
                            .get()
                            .map(|meta| render_node_row(depth, folder, meta, langs, is_selected))
                    })
                })
                .into_element()
        }))
}
