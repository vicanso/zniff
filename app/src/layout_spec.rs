//! Declarative workspace layout specification and validation IR.
//!
//! `LayoutSpec` serves as the platform-neutral intermediate representation shared
//! by built-in layouts (`Split`, `TopBottom`), custom JSON specs, and future
//! sandboxed TSX layout scripts.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::{BTreeMap, HashSet};
use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::config::LayoutMode;

/// Maximum nesting depth allowed in a layout tree (root is depth 1).
pub const MAX_LAYOUT_DEPTH: usize = 8;

/// Built-in identifier for the default horizontal Split layout.
pub const BUILTIN_SPLIT_ID: &str = "split";

/// Built-in identifier for the stacked Top/Bottom layout.
pub const BUILTIN_TOP_BOTTOM_ID: &str = "top_bottom";

/// Named UI modules that can be placed inside the main workspace layout tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewModule {
    Sidebar,
    FlowList,
    Detail,
    Summary,
}

impl ViewModule {
    /// Stable list of all registered workspace modules.
    pub const ALL: [ViewModule; 4] = [Self::Sidebar, Self::FlowList, Self::Detail, Self::Summary];

    /// Returns the canonical snake_case module identifier.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Sidebar => "sidebar",
            Self::FlowList => "flow_list",
            Self::Detail => "detail",
            Self::Summary => "summary",
        }
    }

    /// Parses a module identifier (trimmed, ASCII case-insensitive).
    pub fn parse_id(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.eq_ignore_ascii_case("sidebar") {
            Some(Self::Sidebar)
        } else if trimmed.eq_ignore_ascii_case("flow_list") {
            Some(Self::FlowList)
        } else if trimmed.eq_ignore_ascii_case("detail") {
            Some(Self::Detail)
        } else if trimmed.eq_ignore_ascii_case("summary") {
            Some(Self::Summary)
        } else {
            None
        }
    }

    /// Returns true if the module must always be present in a valid workspace layout.
    pub const fn is_required(self) -> bool {
        matches!(self, Self::FlowList)
    }
}

/// Dock position for the status footer strip.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FooterDock {
    #[default]
    Bottom,
    Top,
    Hidden,
}

/// Runtime surface target exposed to layout scripts and evaluators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutTarget {
    Desktop,
    Web,
}

/// Low-frequency environment inputs passed to layout script evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LayoutContext {
    pub width: f32,
    pub height: f32,
    pub target: LayoutTarget,
}

/// Quantizes `LayoutContext` dimensions into 20 px resize buckets for evaluation debouncing & caching.
pub fn layout_context_bucket(ctx: LayoutContext) -> (i32, i32) {
    (
        (ctx.width / 20.0).round() as i32,
        (ctx.height / 20.0).round() as i32,
    )
}

/// Structural shape of a [`LayoutSpec`] stripped of [`LayoutSize`] values.
///
/// Comparing `LayoutTopology` instead of full `LayoutSpec` lets the UI shell
/// update pane dimensions reactively in-place without unmounting heavy views
/// (`DataTable`, `Editor`) during window resizing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutTopology {
    BuiltinSplit,
    BuiltinTopBottom,
    CustomRow(Vec<LayoutTopology>),
    CustomCol(Vec<LayoutTopology>),
    CustomPane(ViewModule),
}

/// Primary-axis size constraint for a container or leaf pane node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayoutSize {
    Auto,
    Px(f32),
    Percent(f32),
    Fr(f32),
}

impl Default for LayoutSize {
    fn default() -> Self {
        Self::Fr(1.0)
    }
}

impl LayoutSize {
    /// Parses size expressions (`"auto"`, `"<n>px"`, `"<n>%"`, `"<n>fr"`, or `"<n>"`).
    pub fn parse_str(raw: &str) -> Result<Self, String> {
        let s = raw.trim();
        if s.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }
        if let Some(num) = strip_suffix_ci(s, "px") {
            let value = parse_finite_f32(num, raw)?;
            return Ok(Self::Px(value));
        }
        if let Some(num) = s.strip_suffix('%') {
            let value = parse_finite_f32(num, raw)?;
            return Ok(Self::Percent(value));
        }
        if let Some(num) = strip_suffix_ci(s, "fr") {
            let value = parse_finite_f32(num, raw)?;
            return Ok(Self::Fr(value));
        }
        if !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'+' | b'-' | b'e' | b'E'))
        {
            let value = parse_finite_f32(s, raw)?;
            return Ok(Self::Px(value));
        }
        Err(format!("invalid layout size `{raw}`"))
    }
}

fn strip_suffix_ci<'a>(s: &'a str, suffix: &str) -> Option<&'a str> {
    if s.len() < suffix.len() {
        return None;
    }
    let split = s.len() - suffix.len();
    if !s.is_char_boundary(split) {
        return None;
    }
    s[split..]
        .eq_ignore_ascii_case(suffix)
        .then_some(&s[..split])
}

fn parse_finite_f32(num: &str, full: &str) -> Result<f32, String> {
    let trimmed = num.trim();
    let v: f32 = trimmed
        .parse()
        .map_err(|_| format!("invalid layout size `{full}`"))?;
    if !v.is_finite() {
        return Err(format!("non-finite layout size `{full}`"));
    }
    Ok(v)
}

fn format_clean_f32(v: f32) -> String {
    if !v.is_finite() {
        return "0".to_string();
    }
    let rounded = v.round();
    if (v - rounded).abs() < 1e-4 {
        format!("{}", rounded as i64)
    } else {
        let s = format!("{v:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

impl fmt::Display for LayoutSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Auto => f.write_str("auto"),
            Self::Px(v) => write!(f, "{}px", format_clean_f32(v)),
            Self::Percent(v) => write!(f, "{}%", format_clean_f32(v)),
            Self::Fr(v) => write!(f, "{}fr", format_clean_f32(v)),
        }
    }
}

impl Serialize for LayoutSize {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for LayoutSize {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SizeVisitor;

        impl Visitor<'_> for SizeVisitor {
            type Value = LayoutSize;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a layout size string (e.g. \"220px\", \"50%\", \"1fr\", \"auto\") or number",
                )
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<LayoutSize, E> {
                LayoutSize::parse_str(v).map_err(E::custom)
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<LayoutSize, E> {
                Ok(LayoutSize::Px(v as f32))
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<LayoutSize, E> {
                Ok(LayoutSize::Px(v as f32))
            }

            fn visit_f64<E: de::Error>(self, v: f64) -> Result<LayoutSize, E> {
                if !v.is_finite() {
                    return Err(E::custom("layout size number must be finite"));
                }
                Ok(LayoutSize::Px(v as f32))
            }
        }

        deserializer.deserialize_any(SizeVisitor)
    }
}

/// Recursive split-tree node representing rows, columns, or named workspace panes.
///
/// Future Phase 3 custom TSX panels (e.g. `CustomPane`) extend this tagged enum cleanly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNode {
    Row {
        #[serde(default)]
        size: LayoutSize,
        children: Vec<LayoutNode>,
    },
    Col {
        #[serde(default)]
        size: LayoutSize,
        children: Vec<LayoutNode>,
    },
    Pane {
        #[serde(rename = "name")]
        module: ViewModule,
        #[serde(default)]
        size: LayoutSize,
    },
}

impl LayoutNode {
    /// Creates a horizontal split row container with default flex sizing.
    pub fn row(children: Vec<LayoutNode>) -> Self {
        Self::Row {
            size: LayoutSize::Fr(1.0),
            children,
        }
    }

    /// Creates a vertical split column container with default flex sizing.
    pub fn col(children: Vec<LayoutNode>) -> Self {
        Self::Col {
            size: LayoutSize::Fr(1.0),
            children,
        }
    }

    /// Creates a named module pane node with the given size constraint.
    pub fn pane(module: ViewModule, size: LayoutSize) -> Self {
        Self::Pane { module, size }
    }

    /// Returns this node's primary-axis size constraint.
    pub const fn size(&self) -> LayoutSize {
        match self {
            Self::Row { size, .. } | Self::Col { size, .. } | Self::Pane { size, .. } => *size,
        }
    }

    /// Returns a copy of this node with its primary-axis size updated.
    pub fn with_size(mut self, next: LayoutSize) -> Self {
        match &mut self {
            Self::Row { size, .. } | Self::Col { size, .. } | Self::Pane { size, .. } => {
                *size = next;
            }
        }
        self
    }

    /// Returns true if `module` appears anywhere inside this subtree.
    pub fn contains_module(&self, module: ViewModule) -> bool {
        match self {
            Self::Pane { module: cur, .. } => *cur == module,
            Self::Row { children, .. } | Self::Col { children, .. } => {
                children.iter().any(|child| child.contains_module(module))
            }
        }
    }

    /// Collects all module identifiers referenced in pre-order traversal.
    pub fn modules(&self) -> Vec<ViewModule> {
        let mut out = Vec::new();
        self.collect_modules(&mut out);
        out
    }

    fn collect_modules(&self, out: &mut Vec<ViewModule>) {
        match self {
            Self::Pane { module, .. } => out.push(*module),
            Self::Row { children, .. } | Self::Col { children, .. } => {
                for child in children {
                    child.collect_modules(out);
                }
            }
        }
    }

    /// Extracts the size-independent structural topology of this layout node.
    pub fn topology(&self) -> LayoutTopology {
        match self {
            Self::Pane { module, .. } => LayoutTopology::CustomPane(*module),
            Self::Row { children, .. } => {
                LayoutTopology::CustomRow(children.iter().map(Self::topology).collect())
            }
            Self::Col { children, .. } => {
                LayoutTopology::CustomCol(children.iter().map(Self::topology).collect())
            }
        }
    }

    /// Collects every child slot's canonical path (e.g. `"root[0]"`, `"root[1][0]"`) to its [`LayoutSize`].
    pub fn collect_slot_sizes(&self, parent_path: &str, out: &mut BTreeMap<String, LayoutSize>) {
        if let Self::Row { children, .. } | Self::Col { children, .. } = self {
            for (idx, child) in children.iter().enumerate() {
                let child_path = format!("{parent_path}[{idx}]");
                out.insert(child_path.clone(), child.size());
                child.collect_slot_sizes(&child_path, out);
            }
        }
    }
}

/// Validation or parse failure returned when inspecting a `LayoutSpec`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    ParseFailed(String),
    EmptyContainer { path: String },
    DuplicateModule { module: ViewModule },
    MissingRequiredModule { module: ViewModule },
    InvalidSize { path: String, detail: String },
    MaxDepthExceeded { max_depth: usize },
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParseFailed(msg) => write!(f, "failed to parse layout spec: {msg}"),
            Self::EmptyContainer { path } => write!(f, "empty container at `{path}`"),
            Self::DuplicateModule { module } => {
                write!(f, "module `{}` appears more than once", module.id())
            }
            Self::MissingRequiredModule { module } => {
                write!(f, "required module `{}` is missing", module.id())
            }
            Self::InvalidSize { path, detail } => {
                write!(f, "invalid size at `{path}`: {detail}")
            }
            Self::MaxDepthExceeded { max_depth } => {
                write!(f, "layout nesting depth exceeds limit ({max_depth})")
            }
        }
    }
}

impl std::error::Error for LayoutError {}

/// Complete workspace layout specification produced by presets or layout scripts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutSpec {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub footer: FooterDock,
    pub root: LayoutNode,
}

impl LayoutSpec {
    /// Returns the built-in horizontal Split workspace tree.
    pub fn builtin_split() -> Self {
        Self {
            name: BUILTIN_SPLIT_ID.to_string(),
            footer: FooterDock::Bottom,
            root: LayoutNode::row(vec![
                LayoutNode::pane(ViewModule::Sidebar, LayoutSize::Auto),
                LayoutNode::pane(ViewModule::FlowList, LayoutSize::Auto),
                LayoutNode::pane(ViewModule::Detail, LayoutSize::Fr(1.0)),
            ]),
        }
    }

    /// Returns the built-in stacked Top/Bottom workspace tree.
    pub fn builtin_top_bottom() -> Self {
        Self {
            name: BUILTIN_TOP_BOTTOM_ID.to_string(),
            footer: FooterDock::Bottom,
            root: LayoutNode::col(vec![
                LayoutNode::pane(ViewModule::FlowList, LayoutSize::Auto),
                LayoutNode::pane(ViewModule::Detail, LayoutSize::Fr(1.0)),
            ]),
        }
    }

    /// Maps the persisted `LayoutMode` enum to its canonical `LayoutSpec`.
    pub fn for_layout_mode(mode: LayoutMode) -> Self {
        match mode {
            LayoutMode::Split => Self::builtin_split(),
            LayoutMode::TopBottom => Self::builtin_top_bottom(),
        }
    }

    /// Returns true only when `root` uses the exact built-in Split structure and `Auto` sizing.
    pub fn is_builtin_split(&self) -> bool {
        matches!(
            &self.root,
            LayoutNode::Row { children, .. }
                if matches!(
                    children.as_slice(),
                    [
                        LayoutNode::Pane { module: ViewModule::Sidebar, size: LayoutSize::Auto },
                        LayoutNode::Pane { module: ViewModule::FlowList, size: LayoutSize::Auto },
                        LayoutNode::Pane { module: ViewModule::Detail, size: LayoutSize::Fr(f) },
                    ] if (*f - 1.0).abs() < f32::EPSILON
                )
        )
    }

    /// Returns true only when `root` uses the exact built-in Top/Bottom structure and `Auto` sizing.
    pub fn is_builtin_top_bottom(&self) -> bool {
        matches!(
            &self.root,
            LayoutNode::Col { children, .. }
                if matches!(
                    children.as_slice(),
                    [
                        LayoutNode::Pane { module: ViewModule::FlowList, size: LayoutSize::Auto },
                        LayoutNode::Pane { module: ViewModule::Detail, size: LayoutSize::Fr(f) },
                    ] if (*f - 1.0).abs() < f32::EPSILON
                )
        )
    }

    /// Returns true if `summary` is placed directly inside the workspace split tree.
    pub fn has_standalone_summary(&self) -> bool {
        self.root.contains_module(ViewModule::Summary)
    }

    /// Returns the size-independent workspace topology used to gate full tree rebuilds.
    pub fn topology(&self) -> LayoutTopology {
        if self.is_builtin_split() {
            LayoutTopology::BuiltinSplit
        } else if self.is_builtin_top_bottom() {
            LayoutTopology::BuiltinTopBottom
        } else {
            self.root.topology()
        }
    }

    /// Collects a flat path -> [`LayoutSize`] map for all split slots in this specification.
    pub fn slot_sizes(&self) -> BTreeMap<String, LayoutSize> {
        let mut out = BTreeMap::new();
        self.root.collect_slot_sizes("root", &mut out);
        out
    }

    /// Validates tree depth, container children, size ranges, uniqueness, and required modules.
    pub fn validate(&self) -> Result<(), LayoutError> {
        let mut seen = HashSet::new();
        validate_node(&self.root, 1, "root", &mut seen)?;
        if !seen.contains(&ViewModule::FlowList) {
            return Err(LayoutError::MissingRequiredModule {
                module: ViewModule::FlowList,
            });
        }
        Ok(())
    }

    /// Parses and validates a JSON layout specification.
    pub fn parse_json(raw: &str) -> Result<Self, LayoutError> {
        let spec: Self =
            serde_json::from_str(raw).map_err(|err| LayoutError::ParseFailed(err.to_string()))?;
        spec.validate()?;
        Ok(spec)
    }

    /// Parses and validates JSON, falling back safely to `builtin_split()` on error.
    pub fn parse_json_or_fallback(raw: &str) -> (Self, Option<LayoutError>) {
        match Self::parse_json(raw) {
            Ok(spec) => (spec, None),
            Err(err) => (Self::builtin_split(), Some(err)),
        }
    }

    /// Serializes this layout specification into formatted JSON.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

fn validate_node(
    node: &LayoutNode,
    depth: usize,
    path: &str,
    seen: &mut HashSet<ViewModule>,
) -> Result<(), LayoutError> {
    if depth > MAX_LAYOUT_DEPTH {
        return Err(LayoutError::MaxDepthExceeded {
            max_depth: MAX_LAYOUT_DEPTH,
        });
    }
    validate_size(node.size(), path)?;
    match node {
        LayoutNode::Pane { module, .. } => {
            if !seen.insert(*module) {
                return Err(LayoutError::DuplicateModule { module: *module });
            }
            Ok(())
        }
        LayoutNode::Row { children, .. } | LayoutNode::Col { children, .. } => {
            if children.is_empty() {
                return Err(LayoutError::EmptyContainer {
                    path: path.to_string(),
                });
            }
            for (idx, child) in children.iter().enumerate() {
                let child_path = format!("{path}[{idx}]");
                validate_node(child, depth + 1, &child_path, seen)?;
            }
            Ok(())
        }
    }
}

fn validate_size(size: LayoutSize, path: &str) -> Result<(), LayoutError> {
    match size {
        LayoutSize::Auto => Ok(()),
        LayoutSize::Px(v) if v.is_finite() && v > 0.0 => Ok(()),
        LayoutSize::Percent(v) if v.is_finite() && v > 0.0 && v <= 100.0 => Ok(()),
        LayoutSize::Fr(v) if v.is_finite() && v > 0.0 => Ok(()),
        other => Err(LayoutError::InvalidSize {
            path: path.to_string(),
            detail: format!("size `{other}` out of range"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_module_ids_and_required_flags() {
        assert_eq!(ViewModule::ALL.len(), 4);
        for module in ViewModule::ALL {
            assert_eq!(ViewModule::parse_id(module.id()), Some(module));
            assert_eq!(
                ViewModule::parse_id(&format!("  {}  ", module.id().to_ascii_uppercase())),
                Some(module)
            );
            assert_eq!(module.is_required(), module == ViewModule::FlowList);
        }
        assert_eq!(ViewModule::parse_id("unknown"), None);
    }

    #[test]
    fn builtins_validate_and_match_fast_paths() {
        let split = LayoutSpec::builtin_split();
        assert!(split.validate().is_ok());
        assert!(split.is_builtin_split());
        assert!(!split.is_builtin_top_bottom());
        assert!(!split.has_standalone_summary());

        let tb = LayoutSpec::builtin_top_bottom();
        assert!(tb.validate().is_ok());
        assert!(tb.is_builtin_top_bottom());
        assert!(!tb.is_builtin_split());

        let custom_sized_split = LayoutSpec {
            name: "custom".into(),
            footer: FooterDock::Top,
            root: LayoutNode::row(vec![
                LayoutNode::pane(ViewModule::Sidebar, LayoutSize::Px(220.0)),
                LayoutNode::pane(ViewModule::FlowList, LayoutSize::Fr(1.0)),
                LayoutNode::pane(ViewModule::Detail, LayoutSize::Percent(48.0)),
            ]),
        };
        assert!(custom_sized_split.validate().is_ok());
        assert!(!custom_sized_split.is_builtin_split());
    }

    #[test]
    fn nested_tsx_style_json_roundtrip_and_standalone_summary() {
        let json = r#"{
            "name": "Wide Focus",
            "footer": "top",
            "root": {
                "type": "row",
                "children": [
                    { "type": "pane", "name": "sidebar", "size": 220 },
                    {
                        "type": "col",
                        "size": "1fr",
                        "children": [
                            { "type": "pane", "name": "flow_list", "size": "1fr" },
                            { "type": "pane", "name": "summary", "size": "140px" }
                        ]
                    },
                    { "type": "pane", "name": "detail", "size": "48%" }
                ]
            }
        }"#;
        let parsed = LayoutSpec::parse_json(json).expect("valid nested layout");
        assert_eq!(parsed.footer, FooterDock::Top);
        assert!(parsed.has_standalone_summary());
        assert_eq!(
            parsed.root.modules(),
            vec![
                ViewModule::Sidebar,
                ViewModule::FlowList,
                ViewModule::Summary,
                ViewModule::Detail
            ]
        );
        let serialized = parsed.to_json_pretty().expect("serialize");
        let reparsed = LayoutSpec::parse_json(&serialized).expect("reparse");
        assert_eq!(parsed, reparsed);
    }

    #[test]
    fn validation_catches_all_error_variants() {
        assert!(matches!(
            LayoutSpec::parse_json("{not json"),
            Err(LayoutError::ParseFailed(_))
        ));

        let empty_row = LayoutSpec {
            name: String::new(),
            footer: FooterDock::Bottom,
            root: LayoutNode::row(vec![]),
        };
        assert_eq!(
            empty_row.validate(),
            Err(LayoutError::EmptyContainer {
                path: "root".into()
            })
        );

        let dup = LayoutSpec {
            name: String::new(),
            footer: FooterDock::Bottom,
            root: LayoutNode::row(vec![
                LayoutNode::pane(ViewModule::FlowList, LayoutSize::Fr(1.0)),
                LayoutNode::pane(ViewModule::FlowList, LayoutSize::Fr(1.0)),
            ]),
        };
        assert_eq!(
            dup.validate(),
            Err(LayoutError::DuplicateModule {
                module: ViewModule::FlowList
            })
        );

        let missing_flow = LayoutSpec {
            name: String::new(),
            footer: FooterDock::Hidden,
            root: LayoutNode::row(vec![LayoutNode::pane(
                ViewModule::Detail,
                LayoutSize::Fr(1.0),
            )]),
        };
        assert_eq!(
            missing_flow.validate(),
            Err(LayoutError::MissingRequiredModule {
                module: ViewModule::FlowList
            })
        );

        for bad_size in [
            LayoutSize::Px(0.0),
            LayoutSize::Px(-10.0),
            LayoutSize::Percent(0.0),
            LayoutSize::Percent(101.0),
            LayoutSize::Fr(0.0),
            LayoutSize::Fr(-1.0),
        ] {
            let bad = LayoutSpec {
                name: String::new(),
                footer: FooterDock::Bottom,
                root: LayoutNode::row(vec![LayoutNode::pane(ViewModule::FlowList, bad_size)]),
            };
            assert!(
                matches!(bad.validate(), Err(LayoutError::InvalidSize { .. })),
                "expected invalid size for {bad_size:?}"
            );
        }

        let mut deep = LayoutNode::pane(ViewModule::FlowList, LayoutSize::Fr(1.0));
        for _ in 0..MAX_LAYOUT_DEPTH {
            deep = LayoutNode::col(vec![deep]);
        }
        let too_deep = LayoutSpec {
            name: String::new(),
            footer: FooterDock::Bottom,
            root: deep,
        };
        assert_eq!(
            too_deep.validate(),
            Err(LayoutError::MaxDepthExceeded {
                max_depth: MAX_LAYOUT_DEPTH
            })
        );
    }

    #[test]
    fn parse_json_or_fallback_recovers_to_builtin_split() {
        let (spec, err) = LayoutSpec::parse_json_or_fallback(
            r#"{ "root": { "type": "row", "children": [ { "type": "pane", "name": "detail" } ] } }"#,
        );
        assert!(spec.is_builtin_split());
        assert_eq!(
            err,
            Some(LayoutError::MissingRequiredModule {
                module: ViewModule::FlowList
            })
        );
    }

    #[test]
    fn layout_context_and_with_size_roundtrip() {
        let resized_leaf =
            LayoutNode::pane(ViewModule::FlowList, LayoutSize::Auto).with_size(LayoutSize::Fr(2.0));
        assert_eq!(resized_leaf.size(), LayoutSize::Fr(2.0));
        let resized_col = LayoutNode::col(vec![resized_leaf]).with_size(LayoutSize::Percent(60.0));
        assert_eq!(resized_col.size(), LayoutSize::Percent(60.0));

        for target in [LayoutTarget::Desktop, LayoutTarget::Web] {
            let ctx = LayoutContext {
                width: 1280.0,
                height: 800.0,
                target,
            };
            let json = serde_json::to_string(&ctx).expect("serialize LayoutContext");
            let back: LayoutContext =
                serde_json::from_str(&json).expect("deserialize LayoutContext");
            assert_eq!(ctx, back);
        }
    }
}
