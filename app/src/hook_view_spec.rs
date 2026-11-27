//! Cross-platform declarative Inspector View IR, TSX JSX constructors,
//! validation limits, and lazy body helper prelude for all `zniff` hook stages.

use serde::{Deserialize, Serialize};

/// Maximum recursion depth allowed in one declarative TSX hook view tree.
pub const MAX_PREVIEW_CARD_DEPTH: usize = 8;

/// Maximum node count allowed in one hook view tree.
pub const MAX_PREVIEW_CARD_NODES: usize = 128;

/// Maximum aggregate UTF-8 text bytes across one hook view tree.
pub const MAX_PREVIEW_CARD_TEXT_BYTES: usize = 64 * 1024;

const MAX_TABLE_COLUMNS: usize = 16;
const MAX_TABLE_ROWS: usize = 200;
const MAX_COPY_ACTION_BYTES: usize = 16 * 1024;
const MAX_FILTER_ACTION_BYTES: usize = 512;

/// Semantic tone for declarative TSX badges, text, and callouts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewTone {
    #[default]
    Neutral,
    Ok,
    Warn,
    Danger,
    Accent,
}

/// Font size tier for declarative `<Text>` nodes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextSizeTier {
    Xs,
    #[default]
    Sm,
    Base,
}

/// Optional zero-JS-residency host action props attached to declarative nodes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookViewAction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select_flow_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_filter: Option<String>,
}

impl HookViewAction {
    pub fn is_empty(&self) -> bool {
        self.copy_value.is_none() && self.select_flow_id.is_none() && self.set_filter.is_none()
    }
}

/// One `<Tab label="...">` panel inside `<Tabs>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookViewTabItem {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub children: Vec<PreviewCardNode>,
}

const fn default_true() -> bool {
    true
}

/// Alias emphasizing that `PreviewCardNode` serves as the shared IR for all 4 hook stages.
#[allow(dead_code)]
pub type HookViewNode = PreviewCardNode;

/// Declarative Inspector UI IR returned via `view` (or legacy `cards`) by hook scripts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PreviewCardNode {
    Section {
        #[serde(default)]
        title: String,
        #[serde(default)]
        children: Vec<PreviewCardNode>,
        #[serde(default, flatten)]
        action: HookViewAction,
    },
    KeyValue {
        #[serde(default)]
        label: String,
        #[serde(default)]
        value: String,
        #[serde(default, flatten)]
        action: HookViewAction,
    },
    Badge {
        #[serde(default)]
        tone: PreviewTone,
        #[serde(default)]
        text: String,
        #[serde(default, flatten)]
        action: HookViewAction,
    },
    Code {
        #[serde(default)]
        language: Option<String>,
        #[serde(default)]
        code: String,
        #[serde(default, flatten)]
        action: HookViewAction,
    },
    View {
        #[serde(default)]
        gap: Option<u8>,
        #[serde(default)]
        padding: Option<u8>,
        #[serde(default)]
        bordered: bool,
        #[serde(default)]
        children: Vec<PreviewCardNode>,
        #[serde(default, flatten)]
        action: HookViewAction,
    },
    Row {
        #[serde(default)]
        gap: Option<u8>,
        #[serde(default)]
        wrap: bool,
        #[serde(default)]
        align: Option<String>,
        #[serde(default)]
        justify: Option<String>,
        #[serde(default)]
        children: Vec<PreviewCardNode>,
        #[serde(default, flatten)]
        action: HookViewAction,
    },
    Col {
        #[serde(default)]
        gap: Option<u8>,
        #[serde(default)]
        children: Vec<PreviewCardNode>,
        #[serde(default, flatten)]
        action: HookViewAction,
    },
    Text {
        #[serde(default)]
        text: String,
        #[serde(default)]
        tone: PreviewTone,
        #[serde(default)]
        mono: bool,
        #[serde(default)]
        bold: bool,
        #[serde(default)]
        size: Option<TextSizeTier>,
        #[serde(default, flatten)]
        action: HookViewAction,
    },
    Divider,
    Callout {
        #[serde(default)]
        tone: PreviewTone,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        text: String,
        #[serde(default)]
        children: Vec<PreviewCardNode>,
        #[serde(default, flatten)]
        action: HookViewAction,
    },
    Table {
        #[serde(default)]
        columns: Vec<String>,
        #[serde(default)]
        rows: Vec<Vec<String>>,
    },
    Collapsible {
        #[serde(default)]
        title: String,
        #[serde(
            default = "default_true",
            rename = "defaultOpen",
            alias = "default_open"
        )]
        default_open: bool,
        #[serde(default)]
        children: Vec<PreviewCardNode>,
    },
    Tabs {
        #[serde(default)]
        items: Vec<HookViewTabItem>,
    },
}

/// Validates structural depth, node count, table limits, action props, and text budget.
pub fn validate_preview_cards(nodes: &[PreviewCardNode]) -> Result<(), String> {
    let mut total_nodes = 0usize;
    let mut total_bytes = 0usize;
    for node in nodes {
        validate_card_node(node, 1, &mut total_nodes, &mut total_bytes)?;
    }
    Ok(())
}

/// Estimates aggregate serialized text/action bytes for byte-bounded LRU accounting.
pub fn estimate_view_nodes_bytes(nodes: &[PreviewCardNode]) -> usize {
    let mut bytes = 0usize;
    for node in nodes {
        bytes = bytes.saturating_add(estimate_node_bytes(node));
    }
    bytes.max(64)
}

fn estimate_action_bytes(action: &HookViewAction) -> usize {
    action.copy_value.as_ref().map_or(0, String::len)
        + action.set_filter.as_ref().map_or(0, String::len)
        + usize::from(action.select_flow_id.is_some()) * 8
}

fn estimate_node_bytes(node: &PreviewCardNode) -> usize {
    match node {
        PreviewCardNode::Section {
            title,
            children,
            action,
        } => 32 + title.len() + estimate_action_bytes(action) + estimate_view_nodes_bytes(children),
        PreviewCardNode::KeyValue {
            label,
            value,
            action,
        } => 32 + label.len() + value.len() + estimate_action_bytes(action),
        PreviewCardNode::Badge { text, action, .. } => {
            24 + text.len() + estimate_action_bytes(action)
        }
        PreviewCardNode::Code {
            language,
            code,
            action,
        } => {
            32 + language.as_ref().map_or(0, String::len)
                + code.len()
                + estimate_action_bytes(action)
        }
        PreviewCardNode::View {
            children, action, ..
        }
        | PreviewCardNode::Row {
            children, action, ..
        }
        | PreviewCardNode::Col {
            children, action, ..
        } => 24 + estimate_action_bytes(action) + estimate_view_nodes_bytes(children),
        PreviewCardNode::Text { text, action, .. } => {
            24 + text.len() + estimate_action_bytes(action)
        }
        PreviewCardNode::Divider => 16,
        PreviewCardNode::Callout {
            title,
            text,
            children,
            action,
            ..
        } => {
            32 + title.as_ref().map_or(0, String::len)
                + text.len()
                + estimate_action_bytes(action)
                + estimate_view_nodes_bytes(children)
        }
        PreviewCardNode::Table { columns, rows } => {
            let col_b: usize = columns.iter().map(String::len).sum();
            let row_b: usize = rows
                .iter()
                .map(|r| r.iter().map(String::len).sum::<usize>())
                .sum();
            48 + col_b + row_b
        }
        PreviewCardNode::Collapsible {
            title, children, ..
        } => 32 + title.len() + estimate_view_nodes_bytes(children),
        PreviewCardNode::Tabs { items } => {
            32 + items
                .iter()
                .map(|tab| 16 + tab.label.len() + estimate_view_nodes_bytes(&tab.children))
                .sum::<usize>()
        }
    }
}

fn validate_action(action: &HookViewAction, total_bytes: &mut usize) -> Result<(), String> {
    if let Some(copy) = &action.copy_value {
        if copy.len() > MAX_COPY_ACTION_BYTES {
            return Err(format!(
                "copyValue action exceeds {MAX_COPY_ACTION_BYTES} byte limit"
            ));
        }
        *total_bytes = total_bytes.saturating_add(copy.len());
    }
    if let Some(filter) = &action.set_filter {
        if filter.len() > MAX_FILTER_ACTION_BYTES {
            return Err(format!(
                "setFilter action exceeds {MAX_FILTER_ACTION_BYTES} byte limit"
            ));
        }
        *total_bytes = total_bytes.saturating_add(filter.len());
    }
    check_text_budget(*total_bytes)
}

fn validate_card_node(
    node: &PreviewCardNode,
    depth: usize,
    total_nodes: &mut usize,
    total_bytes: &mut usize,
) -> Result<(), String> {
    if depth > MAX_PREVIEW_CARD_DEPTH {
        return Err(format!(
            "preview view nesting exceeds max depth ({MAX_PREVIEW_CARD_DEPTH})"
        ));
    }
    *total_nodes += 1;
    if *total_nodes > MAX_PREVIEW_CARD_NODES {
        return Err(format!(
            "preview view exceeds max nodes ({MAX_PREVIEW_CARD_NODES})"
        ));
    }
    match node {
        PreviewCardNode::Section {
            title,
            children,
            action,
        } => {
            *total_bytes = total_bytes.saturating_add(title.len());
            validate_action(action, total_bytes)?;
            for child in children {
                validate_card_node(child, depth + 1, total_nodes, total_bytes)?;
            }
        }
        PreviewCardNode::KeyValue {
            label,
            value,
            action,
        } => {
            *total_bytes = total_bytes.saturating_add(label.len() + value.len());
            validate_action(action, total_bytes)?;
        }
        PreviewCardNode::Badge { text, action, .. }
        | PreviewCardNode::Text { text, action, .. } => {
            *total_bytes = total_bytes.saturating_add(text.len());
            validate_action(action, total_bytes)?;
        }
        PreviewCardNode::Code {
            language,
            code,
            action,
        } => {
            let lang_len = language.as_ref().map_or(0, String::len);
            *total_bytes = total_bytes.saturating_add(lang_len + code.len());
            validate_action(action, total_bytes)?;
        }
        PreviewCardNode::View {
            children, action, ..
        }
        | PreviewCardNode::Row {
            children, action, ..
        }
        | PreviewCardNode::Col {
            children, action, ..
        } => {
            validate_action(action, total_bytes)?;
            for child in children {
                validate_card_node(child, depth + 1, total_nodes, total_bytes)?;
            }
        }
        PreviewCardNode::Divider => {}
        PreviewCardNode::Callout {
            title,
            text,
            children,
            action,
            ..
        } => {
            *total_bytes =
                total_bytes.saturating_add(title.as_ref().map_or(0, String::len) + text.len());
            validate_action(action, total_bytes)?;
            for child in children {
                validate_card_node(child, depth + 1, total_nodes, total_bytes)?;
            }
        }
        PreviewCardNode::Table { columns, rows } => {
            if columns.len() > MAX_TABLE_COLUMNS {
                return Err(format!("table exceeds {MAX_TABLE_COLUMNS} columns"));
            }
            if rows.len() > MAX_TABLE_ROWS {
                return Err(format!("table exceeds {MAX_TABLE_ROWS} rows"));
            }
            for col in columns {
                *total_bytes = total_bytes.saturating_add(col.len());
            }
            for row in rows {
                if row.len() > MAX_TABLE_COLUMNS {
                    return Err(format!("table row exceeds {MAX_TABLE_COLUMNS} columns"));
                }
                for cell in row {
                    *total_bytes = total_bytes.saturating_add(cell.len());
                }
            }
            check_text_budget(*total_bytes)?;
        }
        PreviewCardNode::Collapsible {
            title, children, ..
        } => {
            *total_bytes = total_bytes.saturating_add(title.len());
            check_text_budget(*total_bytes)?;
            for child in children {
                validate_card_node(child, depth + 1, total_nodes, total_bytes)?;
            }
        }
        PreviewCardNode::Tabs { items } => {
            for tab in items {
                *total_bytes = total_bytes.saturating_add(tab.label.len());
                check_text_budget(*total_bytes)?;
                for child in &tab.children {
                    validate_card_node(child, depth + 1, total_nodes, total_bytes)?;
                }
            }
        }
    }
    Ok(())
}

fn check_text_budget(bytes: usize) -> Result<(), String> {
    if bytes > MAX_PREVIEW_CARD_TEXT_BYTES {
        Err(format!(
            "preview view text exceeds budget ({MAX_PREVIEW_CARD_TEXT_BYTES} bytes)"
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) const HOOK_VIEW_JS_PRELUDE: &str = r#"
function __zniff_opt_u8(v) {
  if (typeof v === "number" && isFinite(v)) {
    const n = Math.round(v);
    return n < 0 ? 0 : n > 255 ? 255 : n;
  }
  if (typeof v === "string" && /^\d+$/.test(v.trim())) {
    const n = Number(v.trim());
    return n > 255 ? 255 : n;
  }
  return undefined;
}
function __zniff_apply_action(node, props) {
  if (!props || typeof props !== "object") return node;
  if (props.copyValue !== undefined && props.copyValue !== null) {
    node.copyValue = String(props.copyValue);
  }
  if (typeof props.selectFlowId === "number" && isFinite(props.selectFlowId) && props.selectFlowId > 0) {
    node.selectFlowId = Math.floor(props.selectFlowId);
  }
  if (props.setFilter !== undefined && props.setFilter !== null) {
    node.setFilter = String(props.setFilter);
  }
  return node;
}
function Row(props) {
  const kids = [];
  __zniff_push_child(kids, props && props.children);
  const node = { type: "row", children: kids };
  if (props && props.size !== undefined && props.size !== null) node.size = props.size;
  if (props) {
    const g = __zniff_opt_u8(props.gap);
    if (g !== undefined) node.gap = g;
    if (props.wrap === true) node.wrap = true;
    if (typeof props.align === "string") node.align = props.align;
    if (typeof props.justify === "string") node.justify = props.justify;
    __zniff_apply_action(node, props);
  }
  return node;
}
function Col(props) {
  const kids = [];
  __zniff_push_child(kids, props && props.children);
  const node = { type: "col", children: kids };
  if (props && props.size !== undefined && props.size !== null) node.size = props.size;
  if (props) {
    const g = __zniff_opt_u8(props.gap);
    if (g !== undefined) node.gap = g;
    __zniff_apply_action(node, props);
  }
  return node;
}
function Section(props) {
  const kids = [];
  __zniff_push_child(kids, props && props.children);
  const node = {
    type: "section",
    title: props && props.title !== undefined && props.title !== null ? String(props.title) : "",
    children: kids,
  };
  return __zniff_apply_action(node, props);
}
function KeyValue(props) {
  const value = props && props.value !== undefined && props.value !== null
    ? String(props.value)
    : __zniff_text_children(props && props.children);
  const node = {
    type: "key_value",
    label: props && props.label !== undefined && props.label !== null ? String(props.label) : "",
    value,
  };
  return __zniff_apply_action(node, props);
}
function Badge(props) {
  const text = props && props.text !== undefined && props.text !== null
    ? String(props.text)
    : __zniff_text_children(props && props.children);
  const tone = props && typeof props.tone === "string" ? props.tone : "neutral";
  return __zniff_apply_action({ type: "badge", tone, text }, props);
}
function Code(props) {
  const code = props && props.code !== undefined && props.code !== null
    ? String(props.code)
    : __zniff_text_children(props && props.children);
  const node = { type: "code", code };
  if (props && typeof props.language === "string" && props.language.length > 0) {
    node.language = props.language;
  }
  return __zniff_apply_action(node, props);
}
function View(props) {
  const kids = [];
  __zniff_push_child(kids, props && props.children);
  const node = { type: "view", children: kids };
  if (props) {
    const g = __zniff_opt_u8(props.gap);
    if (g !== undefined) node.gap = g;
    const p = __zniff_opt_u8(props.padding);
    if (p !== undefined) node.padding = p;
    if (props.bordered === true) node.bordered = true;
    __zniff_apply_action(node, props);
  }
  return node;
}
function Text(props) {
  const text = props && props.text !== undefined && props.text !== null
    ? String(props.text)
    : __zniff_text_children(props && props.children);
  const node = { type: "text", text };
  if (props) {
    if (typeof props.tone === "string") node.tone = props.tone;
    if (props.mono === true) node.mono = true;
    if (props.bold === true) node.bold = true;
    if (typeof props.size === "string") node.size = props.size;
    __zniff_apply_action(node, props);
  }
  return node;
}
function Divider() {
  return { type: "divider" };
}
function Callout(props) {
  const rawKids = [];
  __zniff_push_child(rawKids, props && props.children);
  const children = [];
  let collected = "";
  for (let i = 0; i < rawKids.length; i++) {
    const k = rawKids[i];
    if (typeof k === "string" || typeof k === "number") {
      collected += String(k);
    } else if (k && typeof k === "object") {
      children.push(k);
    }
  }
  const text = props && props.text !== undefined && props.text !== null
    ? String(props.text)
    : collected;
  const node = {
    type: "callout",
    tone: props && typeof props.tone === "string" ? props.tone : "neutral",
    text,
    children,
  };
  if (props && props.title !== undefined && props.title !== null) {
    node.title = String(props.title);
  }
  return __zniff_apply_action(node, props);
}
function Table(props) {
  const columns = props && Array.isArray(props.columns)
    ? props.columns.map((c) => (c === null || c === undefined ? "" : String(c)))
    : [];
  const rows = props && Array.isArray(props.rows)
    ? props.rows.map((r) => (
        Array.isArray(r)
          ? r.map((c) => (c === null || c === undefined ? "" : String(c)))
          : []
      ))
    : [];
  return { type: "table", columns, rows };
}
function Collapsible(props) {
  const kids = [];
  __zniff_push_child(kids, props && props.children);
  return {
    type: "collapsible",
    title: props && props.title !== undefined && props.title !== null ? String(props.title) : "",
    defaultOpen: !(props && props.defaultOpen === false),
    children: kids,
  };
}
function Tab(props) {
  const kids = [];
  __zniff_push_child(kids, props && props.children);
  return {
    __zniff_tab: true,
    label: props && props.label !== undefined && props.label !== null ? String(props.label) : "",
    children: kids,
  };
}
function Tabs(props) {
  const raw = [];
  __zniff_push_child(raw, props && props.children);
  const items = [];
  for (let i = 0; i < raw.length; i++) {
    const item = raw[i];
    if (item && item.__zniff_tab === true) {
      items.push({ label: item.label || "", children: Array.isArray(item.children) ? item.children : [] });
    }
  }
  return { type: "tabs", items };
}
function __zniff_h(tag, props, ...children) {
  if (tag === __zniff_fragment) {
    const flat = [];
    __zniff_push_child(flat, children);
    return flat;
  }
  const merged = Object.assign({}, props || {});
  if (children.length > 0) merged.children = children;
  if (typeof tag === "function") return tag(merged);
  throw new Error(
    "Unsupported TSX tag `" +
      String(tag) +
      "`: expected Layout, Row, Col, Pane, Section, KeyValue, Badge, Code, View, Text, Divider, Callout, Table, Collapsible, Tabs, or Tab"
  );
}
function __zniff_enrich_body_target(obj) {
  if (!obj || typeof obj !== "object") return obj;
  let cachedText = null;
  const readText = () => {
    if (cachedText !== null) return cachedText;
    if (!obj.body || obj.body.byteLength === 0) {
      cachedText = "";
      return cachedText;
    }
    cachedText = globalThis.zniff.codec.utf8Decode(obj.body);
    return cachedText;
  };
  obj.text = readText;
  obj.json = () => {
    const raw = readText();
    if (!raw || !raw.trim()) return null;
    try { return JSON.parse(raw); } catch { return null; }
  };
  obj.form = () => {
    const raw = readText();
    if (!raw) return [];
    return globalThis.zniff.codec.formUrlDecode(raw);
  };
  return obj;
}
function __zniff_enrich_ctx(ctx) {
  if (!ctx || typeof ctx !== "object") return ctx;
  if (Array.isArray(ctx.query)) {
    for (let i = 0; i < ctx.query.length; i++) Object.freeze(ctx.query[i]);
    Object.freeze(ctx.query);
  }
  if (ctx.req && typeof ctx.req === "object") {
    __zniff_enrich_body_target(ctx.req);
    Object.freeze(ctx.req);
  }
  if (ctx.res && typeof ctx.res === "object") {
    __zniff_enrich_body_target(ctx.res);
    Object.freeze(ctx.res);
  }
  if (ctx.body !== undefined) {
    __zniff_enrich_body_target(ctx);
  }
  return ctx;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_expanded_inspector_nodes_and_limits() {
        let valid = vec![
            PreviewCardNode::Callout {
                tone: PreviewTone::Ok,
                title: Some("Verified".into()),
                text: "Signature OK".into(),
                children: vec![PreviewCardNode::Badge {
                    tone: PreviewTone::Accent,
                    text: "AES-256-GCM".into(),
                    action: HookViewAction {
                        copy_value: Some("secret".into()),
                        select_flow_id: Some(42),
                        set_filter: Some("host:api.vicanso.dev".into()),
                    },
                }],
                action: HookViewAction::default(),
            },
            PreviewCardNode::Table {
                columns: vec!["Key".into(), "Value".into()],
                rows: vec![vec!["env".into(), "prod".into()]],
            },
            PreviewCardNode::Tabs {
                items: vec![HookViewTabItem {
                    label: "Decoded".into(),
                    children: vec![PreviewCardNode::Divider],
                }],
            },
        ];
        assert!(validate_preview_cards(&valid).is_ok());

        let too_wide = vec![PreviewCardNode::Table {
            columns: (0..=MAX_TABLE_COLUMNS).map(|i| format!("c{i}")).collect(),
            rows: vec![],
        }];
        assert!(validate_preview_cards(&too_wide).is_err());

        let copy_too_large = vec![PreviewCardNode::Text {
            text: "x".into(),
            tone: PreviewTone::Neutral,
            mono: false,
            bold: false,
            size: None,
            action: HookViewAction {
                copy_value: Some("a".repeat(MAX_COPY_ACTION_BYTES + 1)),
                ..Default::default()
            },
        }];
        assert!(validate_preview_cards(&copy_too_large).is_err());
    }
}
