//! UI submodules for the zniff desktop and web capture console.

#[allow(unused_imports)]
pub(crate) use std::cell::{Cell, RefCell};
#[allow(unused_imports)]
pub(crate) use std::collections::{HashMap, HashSet};
#[allow(unused_imports)]
pub(crate) use std::net::{Ipv4Addr, SocketAddr};
#[allow(unused_imports)]
pub(crate) use std::rc::Rc;
#[allow(unused_imports)]
pub(crate) use std::sync::{Arc, Mutex};
#[cfg(not(target_arch = "wasm32"))]
#[allow(unused_imports)]
pub(crate) use std::time::Instant;
#[allow(unused_imports)]
pub(crate) use std::time::{Duration, SystemTime};

#[allow(unused_imports)]
pub(crate) use nsh_base::data_table::ColumnFixed;
#[allow(unused_imports)]
pub(crate) use nsh_base::{
    Anchor, DataTable, DataTableApi, DataTableCell, DataTableClasses, DataTableColumn,
    DataTableColumnMove, DataTableContextMenu, DataTableEvent, DataTableHeaderCell, DataTablePress,
    DataTableRange, DataTableRows, DataTableSelection, DiffLayout, ToastManager, ToastMotion,
    ToastOptions, ToastStackState, ToastTransitionStatus, Tree, TreeEntry, TreeEvent, TreeItem,
    data_table, diff_view, editor, input, menu, menu_item, popover, theme, toast, toast_stack,
    tooltip, tree,
};
#[allow(unused_imports)]
pub(crate) use nsh_core::EllipsizeMode;
#[allow(unused_imports)]
pub(crate) use nsh_core::prelude::*;

#[allow(unused_imports)]
pub(crate) use crate::ca::Ca;
#[allow(unused_imports)]
pub(crate) use crate::i18n::{
    Lang, PrefName, Side, StatusMsg, direction_name, lang_from_tag, localize_error, parse_pref, t,
    ws_kind,
};
#[allow(unused_imports)]
pub(crate) use crate::icon::{Glyph, glyph, glyph_px};
#[allow(unused_imports)]
pub(crate) use crate::proxy::{BreakDecision, BreakSide, Capture, Hub, ReplayOverride};
#[allow(unused_imports)]
pub(crate) use crate::session::Direction;
#[allow(unused_imports)]
pub(crate) use crate::{
    bypass, ca, clock, config, har, i18n, icon, present, proxy, rules, session, share, ws,
};
#[cfg(not(target_arch = "wasm32"))]
#[allow(unused_imports)]
pub(crate) use crate::{client_hello, decompress, mcp, relay, sysproxy, updater, web};

/// A desktop-only child. Native stays `parent.child(child)`. The web build
/// drops the child, so system proxy, certificates, and updates are not shown.
macro_rules! desktop_child {
    ($parent:expr, $child:expr) => {{
        #[cfg(not(target_arch = "wasm32"))]
        {
            $parent.child($child)
        }
        #[cfg(target_arch = "wasm32")]
        {
            $parent
        }
    }};
}

mod app_shell;
mod break_bar;
mod cli_state;
mod column_picker;
mod compose_box;
mod controls_actions;
mod controls_runtime;
mod detail_actions;
mod detail_body;
mod detail_headers_msgs;
mod detail_publish;
mod detail_summary;
mod detail_timing;
mod facet_drawer;
mod filter_panel;
mod filter_tokens;
mod flow_cells;
mod flow_table;
mod footer_qr;
mod json_tree;
mod lan_qr;
mod list_columns;
mod machine_settings;
mod mono_font;
mod more_menu;
mod more_menu_rows;
mod profile_dock;
mod profile_io;
mod profile_store;
mod quick_rules;
mod redact_preview;
mod row_projection;
mod seq_dep_waterfall;
mod seq_panel;
mod seq_swimlane;
mod session_actions;
mod theme;
mod toolbar;

pub use self::app_shell::App;
pub(crate) use self::break_bar::*;
pub use self::cli_state::initial_window_size;
#[cfg(not(target_arch = "wasm32"))]
pub use self::cli_state::maybe_run_cli_mode;
pub(crate) use self::cli_state::*;
pub(crate) use self::column_picker::*;
pub(crate) use self::compose_box::*;
pub(crate) use self::controls_actions::*;
pub(crate) use self::controls_runtime::*;
pub(crate) use self::detail_actions::*;
pub(crate) use self::detail_body::*;
pub(crate) use self::detail_headers_msgs::*;
pub(crate) use self::detail_publish::*;
pub(crate) use self::detail_summary::*;
pub(crate) use self::detail_timing::*;
pub(crate) use self::facet_drawer::*;
pub(crate) use self::filter_panel::*;
pub(crate) use self::filter_tokens::*;
pub(crate) use self::flow_cells::*;
pub(crate) use self::flow_table::*;
pub(crate) use self::footer_qr::*;
pub(crate) use self::json_tree::*;
pub(crate) use self::lan_qr::*;
pub(crate) use self::list_columns::*;
pub(crate) use self::machine_settings::*;
pub(crate) use self::mono_font::*;
pub(crate) use self::more_menu::*;
pub(crate) use self::more_menu_rows::*;
pub(crate) use self::profile_dock::*;
pub(crate) use self::profile_io::*;
pub(crate) use self::profile_store::*;
pub(crate) use self::quick_rules::*;
pub(crate) use self::redact_preview::*;
pub(crate) use self::row_projection::*;
pub(crate) use self::seq_dep_waterfall::*;
pub(crate) use self::seq_panel::*;
pub(crate) use self::seq_swimlane::*;
pub(crate) use self::session_actions::*;
pub(crate) use self::theme::*;
pub(crate) use self::toolbar::*;

#[cfg(test)]
mod tests;
