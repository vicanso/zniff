//! zniff: a desktop HTTP(S) capture app. The window is an nsh Rust view.
//! The proxy runs on its own tokio runtime in this process.

#![allow(clippy::too_many_arguments)]
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

mod bypass;
#[cfg(not(target_arch = "wasm32"))]
mod ca;
#[cfg(target_arch = "wasm32")]
#[path = "ca_web.rs"]
mod ca;
#[cfg(not(target_arch = "wasm32"))]
mod client_hello;
mod clock;
mod config;
#[cfg(not(target_arch = "wasm32"))]
mod decompress;
mod har;
mod i18n;
mod icon;
#[cfg(not(target_arch = "wasm32"))]
mod mcp;
mod present;
#[cfg(not(target_arch = "wasm32"))]
mod proxy;
#[cfg(target_arch = "wasm32")]
#[path = "hub_web.rs"]
mod proxy;
#[cfg(not(target_arch = "wasm32"))]
mod relay;
mod rules;
mod session;
mod share;
#[cfg(not(target_arch = "wasm32"))]
mod sysproxy;
#[cfg(not(target_arch = "wasm32"))]
mod updater;
#[cfg(not(target_arch = "wasm32"))]
mod web;
mod ws;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
#[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
use std::net::{Ipv4Addr, SocketAddr};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
use std::time::{Duration, SystemTime};

use nsh_base::data_table::ColumnFixed;
use nsh_base::{
    Anchor, DataTable, DataTableApi, DataTableCell, DataTableClasses, DataTableColumn,
    DataTableColumnMove, DataTableContextMenu, DataTableEvent, DataTableHeaderCell, DataTablePress,
    DataTableRange, DataTableRows, DataTableSelection, DiffLayout, ToastManager, ToastMotion,
    ToastOptions, ToastStackState, ToastTransitionStatus, data_table, diff_view, editor, input,
    menu, menu_item, popover, theme, toast, toast_stack, tooltip,
};
use nsh_core::EllipsizeMode;
use nsh_core::prelude::*;

use crate::ca::Ca;
use crate::i18n::{
    Lang, PrefName, Side, StatusMsg, direction_name, lang_from_tag, localize_error, parse_pref, t,
    ws_kind,
};
use crate::icon::{Glyph, glyph, glyph_px};
use crate::proxy::{BreakDecision, BreakSide, Capture, Hub, ReplayOverride};
use crate::session::Direction;

#[cfg(not(target_arch = "wasm32"))]
pub use ca::clear_ios_simulator_proxies;
#[cfg(not(target_arch = "wasm32"))]
pub use relay::shutdown_relay_push;
#[cfg(not(target_arch = "wasm32"))]
pub use sysproxy::restore_if_dirty;

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

pub fn initial_window_size() -> (i32, i32) {
    let cfg = saved_config();
    (cfg.window_width as i32, cfg.window_height as i32)
}

#[cfg(not(target_arch = "wasm32"))]
fn cli_info_output(args: &[String]) -> Option<String> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Some(format!(
            concat!(
                "zniff {}\n",
                "\n",
                "Usage:\n",
                "  zniff [--data-dir <dir>] [--proxy-auth <user:pass>]         Open desktop window\n",
                "  zniff --web [--web-bind <ip>] [--web-port <port>] [--bind <ip>] [-p|--port <port>]\n",
                "              [--data-dir <dir>] [--proxy-auth <user:pass>]\n",
                "  zniff --relay --token <secret> [--bind <ip>] [-p|--port <port>] [--data-dir <dir>]\n",
                "  zniff --mcp [-p|--port <port>] [--data-dir <dir>]\n",
                "\n",
                "Options:\n",
                "  --data-dir <dir>         Override CA & config directory (env: ZNIFF_DATA_DIR)\n",
                "  --proxy-auth <user:pass> Require Basic auth on capture port (env: ZNIFF_PROXY_AUTH)\n",
                "  -V, --version            Print version and exit\n",
                "  -h, --help               Print help and exit"
            ),
            env!("CARGO_PKG_VERSION"),
        ));
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        return Some(format!("zniff {}", env!("CARGO_PKG_VERSION")));
    }
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn apply_global_cli_flags(args: &[String]) -> std::result::Result<(), String> {
    let mut idx = 0usize;
    while idx < args.len() {
        let arg = args[idx].as_str();
        if let Some(rest) = arg.strip_prefix("--data-dir=") {
            let trimmed = rest.trim();
            if trimmed.is_empty() {
                return Err("缺少数据目录路径 (--data-dir)".to_string());
            }
            ca::set_data_dir_override(trimmed);
        } else if arg == "--data-dir" {
            idx += 1;
            let next = args
                .get(idx)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "缺少数据目录路径 (--data-dir)".to_string())?;
            ca::set_data_dir_override(next);
        } else if let Some(rest) = arg.strip_prefix("--proxy-auth=") {
            config::set_proxy_auth_override(rest);
        } else if arg == "--proxy-auth" {
            idx += 1;
            let next = args
                .get(idx)
                .ok_or_else(|| "缺少代理认证参数 (--proxy-auth <user:pass>)".to_string())?;
            config::set_proxy_auth_override(next);
        }
        idx += 1;
    }
    Ok(())
}

/// Headless modes exit before the desktop window. `--web` serves the operator
/// console and leaves the capture port closed. `--relay` and `--mcp` are separate.
#[cfg(not(target_arch = "wasm32"))]
pub fn maybe_run_cli_mode() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(info) = cli_info_output(&args) {
        println!("{info}");
        return Some(0);
    }
    if let Err(err) = apply_global_cli_flags(&args) {
        eprintln!("{err}");
        return Some(2);
    }
    let web = args.iter().any(|arg| arg == "--web");
    let relay_mode = args.iter().any(|arg| arg == "--relay");
    let mcp = args.iter().any(|arg| arg == "--mcp");
    if web && (relay_mode || mcp) {
        eprintln!("zniff --web 不能和 --relay 或 --mcp 一起用");
        return Some(2);
    }
    if web {
        let saved = saved_config();
        return Some(web::run_cli(&args, saved.listen_port));
    }
    if relay_mode {
        let saved = saved_config();
        return Some(relay::run_cli(&args, saved.listen_port, &saved.relay_token));
    }
    if !mcp {
        return None;
    }
    let mut port = saved_config().listen_port;
    let mut idx = 0usize;
    while idx < args.len() {
        let arg = args[idx].as_str();
        if let Some(rest) = arg.strip_prefix("--port=") {
            if let Some(parsed) = present::parse_port(rest) {
                port = parsed;
            }
        } else if arg == "--port" || arg == "-p" {
            if let Some(next) = args.get(idx + 1)
                && let Some(parsed) = present::parse_port(next)
            {
                port = parsed;
            }
            idx += 1;
        } else if arg == "--data-dir" || arg == "--proxy-auth" {
            idx += 1;
        }
        idx += 1;
    }
    Some(mcp::run_stdio_bridge(port))
}

struct Controls {
    hub: Arc<Hub>,
    capture: Mutex<Option<Capture>>,
    status: Mutex<StatusMsg>,
    running: Mutex<bool>,
    listen: Mutex<String>,
    listen_port: Mutex<u16>,
    ca_status: Mutex<ca::CaStatus>,
    /// User preference. Off until they turn on 「系统代理」.
    use_system_proxy: Mutex<bool>,
    /// True only after this session's `sysproxy::enable` returned `Applied`.
    proxy_applied: Mutex<bool>,
    /// Start, stop, and the proxy switch share this so they cannot interleave.
    ops: Mutex<()>,
    /// Next 「开始」binds `0.0.0.0` when this is on.
    listen_all: Mutex<bool>,
    update_state: Mutex<UpdateState>,
    redact_export: Mutex<bool>,
    redact_strict: Mutex<bool>,
    redact_fields: Mutex<String>,
    relay_url: Mutex<String>,
    relay_token: Mutex<String>,
    #[cfg(not(target_arch = "wasm32"))]
    relay_push: Mutex<Option<relay::PushHandle>>,
    toast_seq: std::sync::atomic::AtomicU64,
    toast_queue: Mutex<Vec<(u64, StatusMsg)>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
enum UpdatePhase {
    #[default]
    Idle,
    Checking,
    Downloading {
        version: String,
        received: u64,
        total: u64,
    },
    Installing {
        version: String,
    },
    ReadyToRestart {
        version: String,
    },
    OpenedInstaller {
        version: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct UpdateState {
    phase: UpdatePhase,
    nudge: Option<String>,
    ignored: Option<String>,
}

impl UpdateState {
    #[cfg(not(target_arch = "wasm32"))]
    fn from_disk() -> Self {
        Self {
            phase: UpdatePhase::Idle,
            nudge: updater::nudge(),
            ignored: updater::ignored(),
        }
    }

    fn is_busy(&self) -> bool {
        matches!(
            self.phase,
            UpdatePhase::Checking
                | UpdatePhase::Downloading { .. }
                | UpdatePhase::Installing { .. }
        )
    }

    fn has_toolbar_pill(&self) -> bool {
        match &self.phase {
            UpdatePhase::Downloading { .. }
            | UpdatePhase::Installing { .. }
            | UpdatePhase::ReadyToRestart { .. }
            | UpdatePhase::OpenedInstaller { .. } => true,
            UpdatePhase::Idle | UpdatePhase::Checking => self.nudge.is_some(),
        }
    }
}

#[derive(Clone, Copy)]
struct ProfileSignals {
    saved: Signal<Vec<config::SavedProfile>>,
    history: Signal<Vec<config::SavedProfile>>,
    name: Signal<String>,
    filter: Signal<String>,
    bypass: Signal<String>,
    rules: Signal<String>,
    upstream: Signal<String>,
    rules_on: Signal<bool>,
    breaks_on: Signal<bool>,
    show_tunnels: Signal<bool>,
    proxy_auth: Signal<String>,
    relay_url: Signal<String>,
    relay_accept: Signal<bool>,
    list_width: Signal<f32>,
    panel_open: Signal<bool>,
    deselect_rev: Signal<u64>,
}

#[derive(Clone, Copy)]
struct Compose {
    composing: Signal<bool>,
    /// True when the open editor is the paused response, not the request.
    response: Signal<bool>,
    method: Signal<String>,
    url: Signal<String>,
    headers: Signal<String>,
    body: Signal<String>,
    status: Signal<String>,
}

#[derive(Clone, PartialEq, Default)]
struct SeqCandidate {
    id: u64,
    method: String,
    host: String,
    path: String,
    status: Option<u16>,
    failed: bool,
    elapsed: String,
    comment: String,
    checked: bool,
}

#[derive(Clone, Copy)]
struct SeqBuilder {
    open: Signal<bool>,
    items: Signal<Vec<SeqCandidate>>,
    include_response: Signal<bool>,
    include_query: Signal<bool>,
    output: Signal<String>,
    diagram: Signal<present::SequenceDiagramData>,
    view_mode: Signal<u8>,
    source_flowchart: Signal<bool>,
    hover_bar: Signal<Option<usize>>,
    redact_open: Signal<bool>,
    redact_export: Signal<bool>,
    redact_strict: Signal<bool>,
    redact_fields: Signal<String>,
    redact_flow_id: Signal<Option<u64>>,
    /// Created with the app, not inside a panel. Virtualized rows can still read
    /// these for a frame after the panel closes, so they must outlive it.
    viewport_w: Signal<f32>,
    track_w: Signal<f32>,
    layout: Memo<(Vec<f32>, f32)>,
    flat_rows: Memo<Vec<DepFlatRow>>,
}

/// The app's UI set (`nsh.json` `"rust"`). The web package calls this entry
/// even though `rust_app!` also sets the root.
#[native_module("zniff/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

/// Header and rows keep a 2pt selection bar plus `px_3` (12pt). The filter uses the same inset.
const LIST_INSET: f32 = 14.0;
/// Most checked flows the sequence-diagram previews draw. Copy keeps them all.
const SEQ_PREVIEW_CAP: usize = 200;
const LIST_PAD_END: f32 = 12.0;
/// A set bit collapses that detail card. Zero leaves every card open for this window only.
const FOLD_QUERY: u8 = 1;
const FOLD_DIFF: u8 = 1 << 1;
const FOLD_MESSAGES: u8 = 1 << 2;
const FOLD_RES_BODY: u8 = 1 << 3;
const FOLD_RES_HEADERS: u8 = 1 << 4;
const FOLD_REQ_HEADERS: u8 = 1 << 5;
const FOLD_REQ_BODY: u8 = 1 << 6;
/// Toolbar controls share one box. The cert chip is a status, not a button.
const CONTROL_H: f32 = 30.0;
const CONTROL_RADIUS: f32 = 7.0;
const CONTROL_TEXT: f32 = 12.5;

fn ui_langs() -> Signal<Option<Lang>> {
    use_context().expect("language signal")
}

fn ui_lang(langs: Signal<Option<Lang>>) -> Lang {
    langs
        .get()
        .unwrap_or_else(|| lang_from_tag(locale().as_str()))
}

fn tr(zh: &'static str, en: &'static str) -> impl Fn() -> String + 'static {
    let langs = ui_langs();
    move || t(ui_lang(langs), zh, en).to_string()
}

fn bg_canvas() -> AdaptiveColor {
    AdaptiveColor::rgb(0xf8fafc, 0x0d1017)
}

fn bg_chrome() -> AdaptiveColor {
    AdaptiveColor::rgb(0xffffff, 0x131822)
}

fn bg_list() -> AdaptiveColor {
    AdaptiveColor::rgb(0xffffff, 0x10141d)
}

fn bg_card() -> AdaptiveColor {
    AdaptiveColor::rgb(0xffffff, 0x131822)
}

fn bg_code() -> AdaptiveColor {
    AdaptiveColor::rgb(0xf8fafc, 0x0f131b)
}

fn bg_control() -> AdaptiveColor {
    AdaptiveColor::rgb(0xf1f5f9, 0x1c2331)
}

fn bg_control_hover() -> AdaptiveColor {
    AdaptiveColor::rgb(0xe2e8f0, 0x252e40)
}

fn border_strong() -> AdaptiveColor {
    AdaptiveColor::rgb(0xe2e8f0, 0x222938)
}

fn border_subtle() -> AdaptiveColor {
    AdaptiveColor::rgb(0xf1f5f9, 0x19202e)
}

fn border_control() -> AdaptiveColor {
    AdaptiveColor::rgb(0xcbd5e1, 0x2b3448)
}

fn fg_primary() -> AdaptiveColor {
    AdaptiveColor::rgb(0x0f172a, 0xf1f5f9)
}

fn fg_secondary() -> AdaptiveColor {
    AdaptiveColor::rgb(0x334155, 0xcbd5e1)
}

fn fg_muted() -> AdaptiveColor {
    AdaptiveColor::rgb(0x526075, 0x8e9bb3)
}

fn accent_blue() -> AdaptiveColor {
    AdaptiveColor::rgb(0x2563eb, 0x2563eb)
}

fn accent_blue_hover() -> AdaptiveColor {
    AdaptiveColor::rgb(0x1d4ed8, 0x1d4ed8)
}

fn accent_blue_soft() -> AdaptiveColor {
    AdaptiveColor::rgb(0xdbeafe, 0x172554)
}

fn accent_blue_soft_hover() -> AdaptiveColor {
    AdaptiveColor::rgb(0xbfdbfe, 0x1e3a8a)
}

fn accent_blue_border() -> AdaptiveColor {
    AdaptiveColor::rgb(0x93c5fd, 0x1e40af)
}

fn accent_blue_text() -> AdaptiveColor {
    AdaptiveColor::rgb(0x1d4ed8, 0x93c5fd)
}

/// The baseline row's leading bar. Same red as the diff view's old side.
fn baseline_mark() -> AdaptiveColor {
    AdaptiveColor::rgb(0xdc2626, 0xf87171)
}

/// The current row in a diff. Same green as the diff view's new side.
fn diff_current_mark() -> AdaptiveColor {
    AdaptiveColor::rgb(0x16a34a, 0x4ade80)
}

fn status_ok_fg() -> AdaptiveColor {
    AdaptiveColor::rgb(0x047857, 0x34d399)
}

fn status_ok_bg() -> AdaptiveColor {
    AdaptiveColor::rgb(0xd1fae5, 0x0a2e26)
}

fn status_ok_bg_hover() -> AdaptiveColor {
    AdaptiveColor::rgb(0xc6f6e2, 0x0f3d33)
}

fn status_warn_fg() -> AdaptiveColor {
    AdaptiveColor::rgb(0xb45309, 0xfbbf24)
}

fn status_warn_bg() -> AdaptiveColor {
    AdaptiveColor::rgb(0xfef3c7, 0x33240b)
}

fn status_err_fg() -> AdaptiveColor {
    AdaptiveColor::rgb(0x9f1239, 0xfb7185)
}

fn status_err_bg() -> AdaptiveColor {
    AdaptiveColor::rgb(0xffe4e6, 0x361320)
}

fn status_err_bg_hover() -> AdaptiveColor {
    AdaptiveColor::rgb(0xfecdd3, 0x4c1d2e)
}

fn danger_button_bg() -> AdaptiveColor {
    AdaptiveColor::rgb(0xdc2626, 0xe11d48)
}

fn danger_button_hover() -> AdaptiveColor {
    AdaptiveColor::rgb(0xb91c1c, 0xbe123c)
}

fn method_fg(method: &str) -> AdaptiveColor {
    match method {
        "GET" => AdaptiveColor::rgb(0x0369a1, 0x38bdf8),
        "POST" => AdaptiveColor::rgb(0x047857, 0x34d399),
        "PUT" | "PATCH" => AdaptiveColor::rgb(0xb45309, 0xfbbf24),
        "DELETE" => AdaptiveColor::rgb(0x9f1239, 0xfb7185),
        "CONNECT" => AdaptiveColor::rgb(0x6d28d9, 0xa78bfa),
        _ => fg_secondary(),
    }
}

fn method_bg(method: &str) -> AdaptiveColor {
    match method {
        "GET" => AdaptiveColor::rgb(0xe0f2fe, 0x0c2a42),
        "POST" => AdaptiveColor::rgb(0xd1fae5, 0x0a2e26),
        "PUT" | "PATCH" => AdaptiveColor::rgb(0xfef3c7, 0x33240b),
        "DELETE" => AdaptiveColor::rgb(0xffe4e6, 0x361320),
        "CONNECT" => AdaptiveColor::rgb(0xede9fe, 0x241842),
        _ => bg_control(),
    }
}

fn http_status_fg(status: Option<u16>, failed: bool) -> AdaptiveColor {
    match status {
        _ if failed => status_err_fg(),
        Some(code) if code >= 400 => status_err_fg(),
        Some(code) if (300..400).contains(&code) => status_warn_fg(),
        Some(code) if (200..300).contains(&code) => status_ok_fg(),
        _ => fg_muted(),
    }
}

fn http_status_bg(status: Option<u16>, failed: bool) -> AdaptiveColor {
    match status {
        _ if failed => status_err_bg(),
        Some(code) if code >= 400 => status_err_bg(),
        Some(code) if (300..400).contains(&code) => status_warn_bg(),
        Some(code) if (200..300).contains(&code) => status_ok_bg(),
        _ => bg_control(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LanAddr {
    name: String,
    ip: Ipv4Addr,
}

#[derive(Clone, PartialEq)]
struct RowLine {
    id: u64,
    method: String,
    status: Option<u16>,
    time: String,
    gap_nanos: u128,
    /// Local `HH:MM:SS.mmm`, shown when the gap cell is hovered.
    clock: String,
    /// Request start until the full response. `—` while it is still open.
    elapsed: String,
    elapsed_nanos: Option<u128>,
    elapsed_slow: bool,
    host: String,
    client_ip: String,
    path: String,
    url: String,
    comment: String,
    protocol: String,
    size: String,
    size_bytes: u64,
    failed: bool,
    pinned: bool,
    /// First 8 hex characters of the share that pushed this row. Empty for local capture.
    relay_short: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ListSort {
    column: ListColumn,
    desc: bool,
}

impl ListSort {
    fn toggle(current: Option<Self>, column: ListColumn) -> Option<Self> {
        match current {
            Some(Self {
                column: cur,
                desc: false,
            }) if cur == column => Some(Self { column, desc: true }),
            Some(Self {
                column: cur,
                desc: true,
            }) if cur == column => None,
            _ => Some(Self {
                column,
                desc: false,
            }),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ListColumns {
    id: bool,
    method: bool,
    path: bool,
    status: bool,
    gap: bool,
    elapsed: bool,
    size: bool,
    protocol: bool,
    host: bool,
    client_ip: bool,
    full_url: bool,
    ellipsize: config::ListEllipsize,
    order: [ListColumn; 10],
    order_rev: u64,
}

impl PartialEq for ListColumns {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.method == other.method
            && self.path == other.path
            && self.status == other.status
            && self.gap == other.gap
            && self.elapsed == other.elapsed
            && self.size == other.size
            && self.protocol == other.protocol
            && self.host == other.host
            && self.client_ip == other.client_ip
            && self.full_url == other.full_url
            && self.ellipsize == other.ellipsize
            && self.order == other.order
    }
}

impl Eq for ListColumns {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ListColumn {
    Id,
    Method,
    Path,
    Status,
    Gap,
    Elapsed,
    Size,
    Protocol,
    Host,
    ClientIp,
}

impl ListColumn {
    const ORDERED: [Self; 10] = [
        Self::Id,
        Self::Method,
        Self::Path,
        Self::Status,
        Self::Gap,
        Self::Elapsed,
        Self::Size,
        Self::Protocol,
        Self::Host,
        Self::ClientIp,
    ];

    fn from_key(key: &str) -> Option<Self> {
        match key.trim() {
            "id" => Some(Self::Id),
            "method" => Some(Self::Method),
            "path" => Some(Self::Path),
            "status" => Some(Self::Status),
            "gap" => Some(Self::Gap),
            "elapsed" => Some(Self::Elapsed),
            "size" => Some(Self::Size),
            "protocol" => Some(Self::Protocol),
            "host" => Some(Self::Host),
            "client_ip" => Some(Self::ClientIp),
            _ => None,
        }
    }

    fn is_fixed(self) -> bool {
        matches!(self, Self::Id | Self::Method | Self::Path)
    }

    fn order_from_keys(keys: &[String]) -> [Self; 10] {
        let resolved = config::resolve_column_order(keys);
        let mut out = Self::ORDERED;
        for (slot, key) in out.iter_mut().zip(resolved.iter()) {
            if let Some(col) = Self::from_key(key) {
                *slot = col;
            }
        }
        out
    }

    fn key(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Method => "method",
            Self::Path => "path",
            Self::Status => "status",
            Self::Gap => "gap",
            Self::Elapsed => "elapsed",
            Self::Size => "size",
            Self::Protocol => "protocol",
            Self::Host => "host",
            Self::ClientIp => "client_ip",
        }
    }

    fn labels(self) -> (&'static str, &'static str) {
        match self {
            Self::Id => ("序号", "#"),
            Self::Method => ("方法", "Method"),
            Self::Path => ("路径", "Path"),
            Self::Status => ("状态", "Status"),
            Self::Gap => ("间隔", "Gap"),
            Self::Elapsed => ("耗时", "Elapsed"),
            Self::Size => ("大小", "Size"),
            Self::Protocol => ("协议", "Protocol"),
            Self::Host => ("主机", "Host"),
            Self::ClientIp => ("来源 IP", "Client IP"),
        }
    }
}

impl ListColumns {
    fn from_config(config: &config::Config) -> Self {
        Self {
            id: config.show_id,
            method: config.show_method,
            path: config.show_path,
            status: config.show_status,
            gap: config.show_gap,
            elapsed: config.show_elapsed,
            size: config.show_size,
            protocol: config.show_protocol,
            host: config.show_host,
            client_ip: config.show_client_ip,
            full_url: config.show_full_url,
            ellipsize: config.ellipsize,
            order: ListColumn::order_from_keys(&config.column_order),
            order_rev: 0,
        }
    }

    fn on(self, column: ListColumn) -> bool {
        match column {
            ListColumn::Id => self.id,
            ListColumn::Method => self.method,
            ListColumn::Path => self.path,
            ListColumn::Status => self.status,
            ListColumn::Gap => self.gap,
            ListColumn::Elapsed => self.elapsed,
            ListColumn::Size => self.size,
            ListColumn::Protocol => self.protocol,
            ListColumn::Host => self.host,
            ListColumn::ClientIp => self.client_ip,
        }
    }

    fn first_visible(self) -> Option<ListColumn> {
        self.order.into_iter().find(|&col| self.on(col))
    }

    fn last_visible(self) -> Option<ListColumn> {
        self.order.into_iter().rev().find(|&col| self.on(col))
    }

    fn visible_count(self) -> usize {
        self.order.into_iter().filter(|&col| self.on(col)).count()
    }

    fn toggle(self, column: ListColumn) -> Self {
        let mut next = self;
        match column {
            ListColumn::Id => next.id = !next.id,
            ListColumn::Method => next.method = !next.method,
            ListColumn::Path => next.path = !next.path,
            ListColumn::Status => next.status = !next.status,
            ListColumn::Gap => next.gap = !next.gap,
            ListColumn::Elapsed => next.elapsed = !next.elapsed,
            ListColumn::Size => next.size = !next.size,
            ListColumn::Protocol => next.protocol = !next.protocol,
            ListColumn::Host => next.host = !next.host,
            ListColumn::ClientIp => next.client_ip = !next.client_ip,
        }
        if next.visible_count() == 0 {
            return self;
        }
        next
    }

    fn can_move_slot(slot: usize, delta: isize) -> bool {
        let Some(target) = slot
            .checked_add_signed(delta)
            .filter(|&t| t < ListColumn::ORDERED.len())
        else {
            return false;
        };
        (slot < 3) == (target < 3)
    }

    #[cfg(test)]
    fn can_move(self, column: ListColumn, delta: isize) -> bool {
        let Some(slot) = self.order.iter().position(|&col| col == column) else {
            return false;
        };
        Self::can_move_slot(slot, delta)
    }

    fn move_by(mut self, column: ListColumn, delta: isize) -> Self {
        let Some(slot) = self.order.iter().position(|&col| col == column) else {
            return self;
        };
        if !Self::can_move_slot(slot, delta) {
            return self;
        }
        let target = (slot as isize + delta) as usize;
        self.order.swap(slot, target);
        self.order_rev = self.order_rev.wrapping_add(1);
        self
    }

    fn apply_visible_order(mut self, visible_keys: &[SharedString]) -> Self {
        let before = self;
        for fixed_zone in [true, false] {
            let picked: Vec<ListColumn> = visible_keys
                .iter()
                .filter_map(|key| ListColumn::from_key(key.as_ref()))
                .filter(|&col| col.is_fixed() == fixed_zone && before.on(col))
                .collect();
            let current: Vec<ListColumn> = self
                .order
                .into_iter()
                .filter(|&col| col.is_fixed() == fixed_zone && before.on(col))
                .collect();
            if picked.len() != current.len() || !current.iter().all(|col| picked.contains(col)) {
                continue;
            }
            let mut iter = picked.into_iter();
            for slot in &mut self.order {
                if slot.is_fixed() == fixed_zone
                    && before.on(*slot)
                    && let Some(col) = iter.next()
                {
                    *slot = col;
                }
            }
        }
        self
    }

    fn has_custom_order(self) -> bool {
        self.order != ListColumn::ORDERED
    }

    fn reset_order(mut self) -> Self {
        if self.order == ListColumn::ORDERED {
            return self;
        }
        self.order = ListColumn::ORDERED;
        self.order_rev = self.order_rev.wrapping_add(1);
        self
    }

    fn with_full_url(self, full_url: bool) -> Self {
        Self { full_url, ..self }
    }

    fn with_ellipsize(self, ellipsize: config::ListEllipsize) -> Self {
        Self { ellipsize, ..self }
    }

    fn ellipsize_mode(self) -> EllipsizeMode {
        match self.ellipsize {
            config::ListEllipsize::Head => EllipsizeMode::Head,
            config::ListEllipsize::Middle => EllipsizeMode::Middle,
            config::ListEllipsize::Tail => EllipsizeMode::Tail,
        }
    }

    fn order_keys(self) -> Vec<String> {
        let keys: Vec<String> = self
            .order
            .into_iter()
            .map(|col| col.key().to_string())
            .collect();
        config::normalize_column_order(&keys)
    }

    fn write(self, config: &mut config::Config) {
        config.show_id = self.id;
        config.show_method = self.method;
        config.show_path = self.path;
        config.show_status = self.status;
        config.show_gap = self.gap;
        config.show_elapsed = self.elapsed;
        config.show_size = self.size;
        config.show_protocol = self.protocol;
        config.show_host = self.host;
        config.show_client_ip = self.client_ip;
        config.show_full_url = self.full_url;
        config.ellipsize = self.ellipsize;
        config.column_order = self.order_keys();
    }
}

/// Columns the table shows, in order. Id, Method, and Path (when shown) stay in
/// the fixed left group. The rest scroll under them.
/// Widths are the last saved drag, not a live value: the table keeps a drag
/// until this list changes, and a changed width here would fight the drag.
fn flow_table_columns(shown: ListColumns, widths: &HashMap<String, u32>) -> Vec<DataTableColumn> {
    shown
        .order
        .into_iter()
        .filter(|&col| shown.on(col))
        .map(|col| {
            let key = col.key();
            let fixed = col.is_fixed();
            let limit = config::column_width_limit(key).expect("known column");
            let width = widths.get(key).copied().unwrap_or(limit.default);
            let mut column = DataTableColumn::default();
            column.key = key.into();
            column.width = Some(px(width as f32));
            column.min_width = Some(px(limit.min as f32));
            column.max_width = Some(px(limit.max as f32));
            column.fixed = ColumnFixed::from(fixed);
            column.resizable = Some(true);
            column.movable = Some(true);
            column.selectable = Some(false);
            column
        })
        .collect()
}

/// Column titles follow the same `tr` pair as the rest of the window.
/// Called while the `.columns` prop runs, so a language change rebuilds them.
fn flow_column_title(key: &str, full_url: bool) -> String {
    match key {
        "method" => tr("方法", "Method")(),
        "id" => tr("序号", "#")(),
        "path" => {
            if full_url {
                "URL".to_string()
            } else {
                tr("路径", "Path")()
            }
        }
        "status" => tr("状态", "Status")(),
        "protocol" => tr("协议", "Protocol")(),
        "gap" => tr("间隔", "Gap")(),
        "elapsed" => tr("耗时", "Elapsed")(),
        "host" => tr("主机", "Host")(),
        "client_ip" => tr("来源 IP", "Client IP")(),
        "size" => tr("大小", "Size")(),
        _ => String::new(),
    }
}

fn flow_table_classes() -> DataTableClasses {
    let mut classes = DataTableClasses::default();
    classes.root = Some("bg-white dark:bg-[#10141d]".into());
    classes.header = Some(
        "bg-[#f1f5f9] dark:bg-[#151b26] border-b border-b-[#e2e8f0] dark:border-b-[#222938]".into(),
    );
    classes.header_cell =
        Some("bg-[#f1f5f9] dark:bg-[#151b26] cursor-grab active:cursor-grabbing".into());
    classes.header_cell_dragging = Some("opacity-60 bg-[#e2e8f0] dark:bg-[#222938]".into());
    classes.move_indicator = Some("bg-[#2563eb] dark:bg-[#3b82f6]".into());
    classes.row = Some(
        "bg-white dark:bg-[#10141d] border-b border-b-[#f1f5f9] dark:border-b-[#19202e] cursor-pointer"
            .into(),
    );
    classes.row_odd = Some("bg-[#fafbfd] dark:bg-[#121722]".into());
    classes.row_hover = Some("bg-[#f1f5f9] dark:bg-[#1a2232]".into());
    classes.row_selected = Some("bg-[#eff6ff] dark:bg-[#172542]".into());
    // The table draws a 1px rule after the pinned columns. Painting it made
    // a line only beside the first scrolling column. The drag target stays
    // and shows a thicker 3px bar across the full 5px hit band on hover/drag.
    classes.resize_handle = Some(
        "w-[3px] bg-transparent hover:bg-[#2563eb] group-hover:bg-[#2563eb] group-active:bg-[#2563eb] dark:hover:bg-[#3b82f6] dark:group-hover:bg-[#3b82f6] dark:group-active:bg-[#3b82f6]"
            .into(),
    );
    classes
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Empty,
    Gone,
    Ready,
}

#[derive(Clone, PartialEq)]
struct HeaderLine {
    key: u32,
    name: String,
    value: String,
    hint: Option<String>,
}

#[derive(Clone, Copy)]
struct Shown {
    phase: Signal<Phase>,
    method: Signal<String>,
    url: Signal<String>,
    protocol: Signal<String>,
    rules: Signal<String>,
    status: Signal<Option<u16>>,
    error: Signal<String>,
    request_headers: Signal<Vec<HeaderLine>>,
    response_headers: Signal<Vec<HeaderLine>>,
    request_body: Signal<String>,
    response_body: Signal<String>,
    request_hex: Signal<Vec<u8>>,
    response_hex: Signal<Vec<u8>>,
    /// Detail-pane viewport width. The hex line uses it after the pane is shown.
    pane_width: Signal<f32>,
    request_language: Signal<String>,
    response_language: Signal<String>,
    request_doc: Signal<Option<nsh_base::text_view::Document>>,
    response_doc: Signal<Option<nsh_base::text_view::Document>>,
    req_doc_preview: Signal<bool>,
    res_doc_preview: Signal<bool>,
    request_form: Signal<Vec<HeaderLine>>,
    request_note: Signal<String>,
    response_note: Signal<String>,
    metrics: Signal<String>,
    timing_bar: Signal<present::TimingBreakdown>,
    query: Signal<Vec<HeaderLine>>,
    preview: Signal<String>,
    preview_w: Signal<f32>,
    preview_h: Signal<f32>,
    image_caption: Signal<String>,
    pretty: Signal<bool>,
    messages: Signal<Vec<HeaderLine>>,
    diff: Signal<String>,
    diff_old: Signal<String>,
    diff_new: Signal<String>,
    pinned: Signal<bool>,
    comment: Signal<String>,
    editing_comment: Signal<bool>,
    repeating: Signal<bool>,
    repeat_count: Signal<String>,
    repeat_interval: Signal<String>,
    repeat_concurrency: Signal<String>,
    req_search: Signal<String>,
    res_search: Signal<String>,
    msg_search: Signal<String>,
    msg_dir: Signal<u8>,
    msg_kind: Signal<u8>,
    /// Eight hex characters of the share that pushed the selected row. Empty locally.
    relay: Signal<String>,
    request_offset: Signal<usize>,
    response_offset: Signal<usize>,
    request_window: Signal<TextWindow>,
    response_window: Signal<TextWindow>,
}

#[derive(Clone, Copy)]
enum CopyPart {
    Url,
    Curl,
    Request,
    Response,
}

/// The root view: a flow list and the selected flow.
#[component]
pub fn App() -> impl IntoElement {
    #[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
    {
        let mut bar = nsh_core::desktop::TitleBarOptions::default();
        bar.style = nsh_core::desktop::TitleBarStyle::Hidden;
        bar.height = Some(px(42.0));
        let mut attributes = nsh_core::desktop::WindowAttributes::default();
        attributes.title_bar = Some(bar);
        nsh_core::element::desktop::main_window().set(attributes);
    }
    let langs = signal(load_lang_pref());
    provide_context(langs);
    let status = signal(StatusMsg::Idle);
    let running = signal(false);
    let ca_status = signal(ca::CaStatus::NotInstalled);
    let use_system_proxy = signal(false);
    let memory = signal(String::from("0 B"));
    let dropped = signal(0_u64);
    let saved_filter = saved_config();
    let filter = signal(saved_filter.filter.clone());
    let list_filter = signal(String::new());
    let effective_filter = signal(saved_filter.filter);
    effect(move || {
        effective_filter.set(session::combine_filters(&filter.get(), &list_filter.get()));
    });
    let saved_profiles = signal(saved_filter.saved_profiles);
    let profile_history = signal(saved_filter.profile_history);
    let selected = signal(None::<u64>);
    let rows = signal(Vec::<Arc<RowLine>>::new());
    let filtered_stats = signal(session::FilteredStats::default());
    let facets = signal(session::SessionFacets::default());
    let facets_open = signal(false);
    let flow_total = signal(0_usize);
    let listen = signal(String::new());
    let saved = saved_config();
    let shown = Shown {
        phase: signal(Phase::Empty),
        method: signal(String::new()),
        url: signal(String::new()),
        protocol: signal(String::new()),
        rules: signal(String::new()),
        status: signal(None),
        error: signal(String::new()),
        request_headers: signal(Vec::new()),
        response_headers: signal(Vec::new()),
        request_body: signal(String::new()),
        response_body: signal(String::new()),
        request_hex: signal(Vec::new()),
        response_hex: signal(Vec::new()),
        pane_width: signal(0.0),
        request_language: signal(String::new()),
        response_language: signal(String::new()),
        request_doc: signal(None),
        response_doc: signal(None),
        req_doc_preview: signal(false),
        res_doc_preview: signal(false),
        request_form: signal(Vec::new()),
        request_note: signal(String::new()),
        response_note: signal(String::new()),
        metrics: signal(String::new()),
        timing_bar: signal(present::TimingBreakdown::default()),
        query: signal(Vec::new()),
        preview: signal(String::new()),
        preview_w: signal(0.0),
        preview_h: signal(0.0),
        image_caption: signal(String::new()),
        pretty: signal(saved.default_pretty),
        messages: signal(Vec::new()),
        diff: signal(String::new()),
        diff_old: signal(String::new()),
        diff_new: signal(String::new()),
        pinned: signal(false),
        comment: signal(String::new()),
        editing_comment: signal(false),
        repeating: signal(false),
        repeat_count: signal(present::REPEAT_COUNT_DEFAULT.to_string()),
        repeat_interval: signal(present::REPEAT_INTERVAL_DEFAULT_MS.to_string()),
        repeat_concurrency: signal(present::REPEAT_CONCURRENCY_DEFAULT.to_string()),
        req_search: signal(String::new()),
        res_search: signal(String::new()),
        msg_search: signal(String::new()),
        msg_dir: signal(0),
        msg_kind: signal(0),
        relay: signal(String::new()),
        request_offset: signal(0),
        response_offset: signal(0),
        request_window: signal(TextWindow::default()),
        response_window: signal(TextWindow::default()),
    };
    let port_text = signal(String::new());
    let listen_all = signal(false);
    let paused = signal(None::<(u64, BreakSide)>);
    let paused_count = signal(0_usize);
    let paused_queue = signal(Vec::<(u64, BreakSide)>::new());
    let baseline = signal(None::<u64>);
    let compose = Compose {
        composing: signal(false),
        response: signal(false),
        method: signal(String::new()),
        url: signal(String::new()),
        headers: signal(String::new()),
        body: signal(String::new()),
        status: signal(String::new()),
    };
    let seq_diagram = signal(present::SequenceDiagramData::default());
    let seq_viewport_w = signal(0.0_f32);
    let seq_builder = SeqBuilder {
        open: signal(false),
        items: signal(Vec::new()),
        include_response: signal(true),
        include_query: signal(false),
        output: signal(String::new()),
        diagram: seq_diagram,
        view_mode: signal(0),
        source_flowchart: signal(false),
        hover_bar: signal(None),
        redact_open: signal(false),
        redact_export: signal(saved.redact_export),
        redact_strict: signal(saved.redact_strict),
        redact_fields: signal(saved.redact_fields.clone()),
        redact_flow_id: signal(None),
        viewport_w: seq_viewport_w,
        track_w: signal(0.0_f32),
        layout: computed(move || seq_diagram.with(|d| swimlane_layout(d, seq_viewport_w.get()))),
        flat_rows: computed(move || seq_diagram.with(build_dep_flat_rows)),
    };
    effect(move || {
        let has_selected = selected.get().is_some();
        let is_composing = compose.composing.get();
        if has_selected || is_composing {
            close_sequence_diagram(seq_builder);
        }
    });
    let list_width = signal(saved.list_width as f32);
    let columns = signal(ListColumns::from_config(&saved));
    let sort = signal(None::<ListSort>);
    let initial_profile_name = if !saved.active_profile.trim().is_empty() {
        saved.active_profile.clone()
    } else {
        matched_saved_profile(
            &saved_profiles.get(),
            &saved.filter,
            &saved.bypass_hosts,
            &saved.rules,
            &saved.upstream_proxy,
        )
        .unwrap_or_default()
    };
    let bypass_text = signal(saved.bypass_hosts);
    let rules_text = signal(saved.rules);
    let upstream_text = signal(saved.upstream_proxy);
    let proxy_auth_sig = signal(config::runtime_proxy_auth(&saved.proxy_auth));
    let relay_url_sig = signal(saved.relay_url);
    let relay_accept_sig = signal(saved.relay_accept);
    let profile_signals = ProfileSignals {
        saved: saved_profiles,
        history: profile_history,
        name: signal(initial_profile_name),
        filter,
        bypass: bypass_text,
        rules: rules_text,
        upstream: upstream_text,
        rules_on: signal(saved.rules_enabled),
        breaks_on: signal(saved.breaks_enabled),
        show_tunnels: signal(saved.show_tunnel_failures),
        proxy_auth: proxy_auth_sig,
        relay_url: relay_url_sig,
        relay_accept: relay_accept_sig,
        list_width,
        panel_open: signal(false),
        deselect_rev: signal(0),
    };
    let lan_url = signal(String::new());
    let lan_choice = signal(String::new());
    let lan_addrs = signal(Vec::<LanAddr>::new());
    let qr_path = signal(String::new());
    let controls = open_controls();
    if let Some(err) = config::take_last_load_error() {
        set_status(&controls, StatusMsg::ConfigBroken { err });
    }
    let update_state = signal(lock_update_state(&controls));
    status.set(lock_status(&controls));
    ca_status.set(lock_ca_status(&controls));
    use_system_proxy.set(lock_bool(&controls.use_system_proxy));
    port_text.set(lock_port(&controls.listen_port).to_string());
    listen_all.set(lock_bool(&controls.listen_all));
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::spawn(updater::sweep_installer_mounts);
        if updater::auto_check_due(clock::now()) {
            auto_check_update(controls.clone());
        }
        if saved_config().capturing {
            let controls_boot = controls.clone();
            std::thread::spawn(move || start_capture(&controls_boot));
        }
    }
    #[cfg(target_arch = "wasm32")]
    start_web_poll(
        controls.clone(),
        port_text,
        listen_all,
        profile_signals,
        effective_filter,
        seq_builder,
        selected,
        baseline,
    );

    let controls_tick = controls.clone();
    let filter_tick = filter;
    let effective_filter_tick = effective_filter;
    let profile_tick = profile_signals;
    let filter_pending = stored(filter.get());
    let filter_since = stored(None::<SystemTime>);
    let history_pending = stored((
        filter.get(),
        bypass_text.get(),
        rules_text.get(),
        upstream_text.get(),
        saved.rules_enabled,
        saved.breaks_enabled,
        saved.show_tunnel_failures,
    ));
    let history_since = stored(Some(clock::now()));
    let win_pending = stored((saved.window_width, saved.window_height));
    let win_since = stored(None::<SystemTime>);
    let selected_tick = selected;
    let rows_tick = rows;
    let stats_tick = filtered_stats;
    let facets_tick = facets;
    let facets_open_tick = facets_open;
    let total_tick = flow_total;
    let shown_tick = shown;
    let seen_gen = stored(0_u64);
    let seen_filter = stored(String::new());
    let seen_setup_filter = stored(String::new());
    let seen_selected = stored(None::<u64>);
    // One follow-up after the selection frame. Layout can put the previous
    // offset back once; the same flow updating does not scroll.
    let detail_scroll = element_ref();
    let detail_scroll_tick = detail_scroll;
    let detail_top_left = stored(false);
    let seen_detail_stamp = stored((
        true,
        None::<u64>,
        None::<u64>,
        None::<u64>,
        None::<u64>,
        shown_tick.pretty.get(),
        ui_lang(langs),
    ));
    let seen_listen = stored(String::new());
    let seen_lang = stored(ui_lang(langs));
    let baseline_tick = baseline;
    let sort_tick = sort;
    let seen_sort = stored(None::<ListSort>);
    let cols_tick = columns;
    let seen_full_url = stored(columns.get().full_url);
    let seen_paused = stored(None::<(u64, BreakSide)>);
    let compose_tick = compose;
    let share_id = signal(String::new());
    let share_id_tick = share_id;
    let seen_share = stored(String::new());
    let update_tick = update_state;
    let seen_update = stored(update_state.get());
    let update_probe_at = stored(clock::now());
    let spill_gc_at = stored(clock::now());
    let seen_facets_open = stored(false);
    let detail_hold = stored(SystemTime::UNIX_EPOCH);
    let row_cache_tick = Rc::new(RefCell::new(RowCache {
        lang: ui_lang(langs),
        rows: HashMap::new(),
    }));
    let toast_manager = stored(ToastManager::<u64, StatusMsg>::new(ToastMotion::sonner()));
    let toast_shown = signal(Vec::<(u64, StatusMsg, ToastTransitionStatus)>::new());
    let toast_state = ToastStackState::new();
    let toast_overlay_gen = signal(0_u64);
    let toast_manager_tick = toast_manager;
    let toast_shown_tick = toast_shown;
    let toast_state_tick = toast_state;
    let toast_gen_tick = toast_overlay_gen;
    // The hex text's own box is only as wide as the current line, so measuring
    // it always says 16 bytes fit. The pane viewport is the width the card
    // actually has. Publishing it from this tick rebuilds the text; a signal
    // write inside `on_layout` can miss that text.
    let pane_px = Rc::new(Cell::new(0.0_f32));
    let pane_tick = pane_px.clone();
    let seen_rules_rev = stored(controls_tick.hub.rules_revision());
    set_interval(Duration::from_millis(16), move || {
        let pending_toasts = {
            let mut q = controls_tick
                .toast_queue
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            std::mem::take(&mut *q)
        };
        if !pending_toasts.is_empty() {
            if toast_shown_tick.with(Vec::is_empty)
                && let Some((last_id, _)) = pending_toasts.last()
            {
                toast_gen_tick.set(*last_id);
            }
            let now = nsh_core::time::now();
            let next_toasts = toast_manager_tick.with(|m| {
                for (tid, msg) in pending_toasts {
                    m.push(
                        tid,
                        msg,
                        ToastOptions::timeout(Some(Duration::from_millis(2800))),
                        now,
                    );
                }
                m.visible(3)
                    .map(|(id, data, status)| (*id, data.clone(), status))
                    .collect::<Vec<_>>()
            });
            toast_shown_tick.set(next_toasts);
        }
        let now_instant = nsh_core::time::now();
        let next_after_advance = toast_manager_tick.with(|m| {
            if m.advance(now_instant, toast_state_tick.is_expanded())
                .changed
            {
                Some(
                    m.visible(3)
                        .map(|(id, data, status)| (*id, data.clone(), status))
                        .collect::<Vec<_>>(),
                )
            } else {
                None
            }
        });
        if let Some(next_toasts) = next_after_advance {
            toast_shown_tick.set(next_toasts);
        }
        let width = pane_tick.get();
        let published = shown_tick.pane_width.get_untracked();
        if width > 0.0 && (width - published).abs() > 1.0 {
            shown_tick.pane_width.set(width);
        }
        let share_now = controls_tick.hub.share_id().unwrap_or_default();
        if share_now != seen_share.get() {
            seen_share.replace(share_now.clone());
            share_id_tick.set(share_now);
        }
        let update_now = lock_update_state(&controls_tick);
        if update_now != seen_update.get() {
            seen_update.replace(update_now.clone());
            update_tick.set(update_now);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if clock::elapsed(update_probe_at.get()) >= Duration::from_secs(3600) {
            update_probe_at.replace(clock::now());
            if updater::auto_check_due(clock::now()) {
                auto_check_update(controls_tick.clone());
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = update_probe_at.get();
        }
        let status_now = lock_status(&controls_tick);
        if status.with(|cur| cur != &status_now) {
            status.set(status_now);
        }
        let running_now = *controls_tick
            .running
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if running.get_untracked() != running_now {
            running.set(running_now);
        }
        let listen_now = lock_string(&controls_tick.listen);
        if listen.with(|cur| cur != &listen_now) {
            listen.set(listen_now.clone());
        }
        let ca_now = lock_ca_status(&controls_tick);
        if ca_status.get_untracked() != ca_now {
            ca_status.set(ca_now);
        }
        let sys_proxy_now = lock_bool(&controls_tick.use_system_proxy);
        if use_system_proxy.get_untracked() != sys_proxy_now {
            use_system_proxy.set(sys_proxy_now);
        }
        let rules_rev_now = controls_tick.hub.rules_revision();
        if rules_rev_now != seen_rules_rev.get() {
            seen_rules_rev.replace(rules_rev_now);
            let hub_rules = controls_tick.hub.rules_text();
            if hub_rules != profile_tick.rules.get_untracked() {
                profile_tick.rules.set(hub_rules);
            }
        }
        let paused_now = controls_tick.hub.paused();
        let paused_len = controls_tick.hub.paused_queue_len();
        let paused_queue_now = controls_tick.hub.paused_queue();
        if paused.get_untracked() != paused_now {
            paused.set(paused_now);
        }
        if paused_count.get_untracked() != paused_len {
            paused_count.set(paused_len);
        }
        if paused_queue.with(|cur| cur != &paused_queue_now) {
            paused_queue.set(paused_queue_now);
        }
        if paused_now != seen_paused.get() {
            let prev_paused = seen_paused.replace(paused_now);
            if let Some((paused_id, side)) = paused_now {
                if selected_tick.get().is_none() {
                    selected_tick.set(Some(paused_id));
                }
                if compose_tick.composing.get()
                    && selected_tick.get() == Some(paused_id)
                    && compose_tick.response.get() != matches!(side, BreakSide::Response)
                {
                    begin_compose(&controls_tick, Some(paused_id), compose_tick);
                }
            } else if let Some((prev_id, prev_side)) = prev_paused
                && compose_tick.composing.get()
                && (compose_tick.response.get()
                    || (prev_side == BreakSide::Request && selected_tick.get() == Some(prev_id)))
            {
                reset_compose(compose_tick);
            }
        }
        if listen_now != seen_listen.get() {
            seen_listen.replace(listen_now.clone());
            publish_lan(
                &listen_now,
                &lan_choice.get_untracked(),
                lan_addrs,
                lan_choice,
                lan_url,
                qr_path,
            );
        }
        let generation = controls_tick.hub.generation();
        let filter_now = filter_tick.get();
        let effective_filter_now = effective_filter_tick.get();
        if filter_now != filter_pending.get() {
            filter_pending.replace(filter_now.clone());
            filter_since.replace(Some(clock::now()));
        } else if filter_since
            .get()
            .is_some_and(|at| clock::elapsed(at) >= Duration::from_millis(500))
        {
            remember_filter(&filter_now);
            filter_since.replace(None);
        }
        let history_now = (
            filter_now.clone(),
            profile_tick.bypass.get(),
            profile_tick.rules.get(),
            profile_tick.upstream.get(),
            profile_tick.rules_on.get(),
            profile_tick.breaks_on.get(),
            profile_tick.show_tunnels.get(),
        );
        if history_now != history_pending.get() {
            history_pending.replace(history_now);
            history_since.replace(Some(clock::now()));
        } else if history_since
            .get()
            .is_some_and(|at| clock::elapsed(at) >= Duration::from_millis(1200))
        {
            record_profile_history(
                &profile_tick.name.get(),
                &history_now.0,
                &history_now.1,
                &history_now.2,
                &history_now.3,
                history_now.4,
                history_now.5,
                history_now.6,
                profile_tick.history,
            );
            history_since.replace(None);
        }
        let win_size = window();
        if win_size.width.0.is_finite()
            && win_size.height.0.is_finite()
            && win_size.width.0 > 0.0
            && win_size.height.0 > 0.0
        {
            let win_now = (
                config::clamp_window_width(win_size.width.0.round() as u32),
                config::clamp_window_height(win_size.height.0.round() as u32),
            );
            if win_now != win_pending.get() {
                win_pending.replace(win_now);
                win_since.replace(Some(clock::now()));
            } else if win_since
                .get()
                .is_some_and(|at| clock::elapsed(at) >= Duration::from_millis(500))
            {
                remember_window_size(win_now.0, win_now.1);
                win_since.replace(None);
            }
        }
        release_sequence_data(seq_builder);
        let spill_expired = if clock::elapsed(spill_gc_at.get()) >= Duration::from_secs(15) {
            let now = clock::now();
            spill_gc_at.replace(now);
            controls_tick
                .hub
                .with_session(|session| session.gc_spill(now))
        } else {
            false
        };
        let selected_now = selected_tick.get();
        let lang_now = ui_lang(langs);
        let baseline_now = baseline_tick.get();
        let pretty_now = shown_tick.pretty.get();
        let sort_now = sort_tick.get();
        let full_url_now = cols_tick.get().full_url;
        let facets_open_now = facets_open_tick.get();
        let facets_opened = facets_open_now && !seen_facets_open.get();
        seen_facets_open.replace(facets_open_now);
        let list_changed = generation != seen_gen.get()
            || effective_filter_now != seen_filter.get()
            || filter_now != seen_setup_filter.get()
            || lang_now != seen_lang.get()
            || sort_now != seen_sort.get()
            || full_url_now != seen_full_url.get()
            || spill_expired
            || facets_opened;
        let want_stats = selected_now.is_none();
        let stats_only = !list_changed && want_stats && seen_selected.get().is_some();
        if list_changed {
            seen_gen.replace(generation);
            seen_filter.replace(effective_filter_now.clone());
            seen_setup_filter.replace(filter_now.clone());
            seen_lang.replace(lang_now);
            seen_sort.replace(sort_now);
            seen_full_url.replace(full_url_now);
            let (matched, _) = controls_tick.hub.matching_ids(&effective_filter_now);
            let facet_ids = if facets_open_now && filter_now != effective_filter_now {
                Some(controls_tick.hub.matching_ids(&filter_now).0)
            } else {
                None
            };
            let (lines, stats, next_facets, next_total, next_mem, next_dropped) = {
                let mut cache = row_cache_tick.borrow_mut();
                controls_tick.hub.with_session(|session| {
                    let lines = project_rows(
                        session,
                        &matched,
                        lang_now,
                        sort_now,
                        full_url_now,
                        &mut cache,
                    );
                    let stats = want_stats.then(|| {
                        session::FilteredStats::from_flows(
                            matched.iter().filter_map(|(id, _)| session.get(*id)),
                        )
                    });
                    let next_facets = facets_open_now.then(|| {
                        let ids = facet_ids.as_ref().unwrap_or(&matched);
                        session::SessionFacets::from_flows(
                            ids.iter().filter_map(|(id, _)| session.get(*id)),
                        )
                    });
                    (
                        lines,
                        stats,
                        next_facets,
                        session.len(),
                        format_bytes(session.memory_bytes()),
                        session.dropped(),
                    )
                })
            };
            if total_tick.get_untracked() != next_total {
                total_tick.set(next_total);
            }
            if let Some(next_facets) = next_facets
                && facets_tick.with(|cur| cur != &next_facets)
            {
                facets_tick.set(next_facets);
            }
            let had_rows = rows_tick.with(|cur| !cur.is_empty());
            if !rows_tick.with(|cur| same_row_arcs(cur, &lines)) {
                rows_tick.set(lines);
            }
            if had_rows && next_total == 0 {
                // The session was just emptied: the list copies are freed only now,
                // after the clear's own trim, so hand those pages back as well.
                crate::session::trim_memory();
            }
            if let Some(stats) = stats
                && stats_tick.with(|cur| cur != &stats)
            {
                stats_tick.set(stats);
            }
            if memory.with(|cur| cur != &next_mem) {
                memory.set(next_mem);
            }
            if dropped.get_untracked() != next_dropped {
                dropped.set(next_dropped);
            }
        } else if stats_only {
            let (matched, _) = controls_tick.hub.matching_ids(&effective_filter_now);
            let stats = controls_tick.hub.with_session(|session| {
                session::FilteredStats::from_flows(
                    matched.iter().filter_map(|(id, _)| session.get(*id)),
                )
            });
            if stats_tick.with(|cur| cur != &stats) {
                stats_tick.set(stats);
            }
        }
        if selected_now != seen_selected.get() {
            seen_selected.replace(selected_now);
            shown_tick.req_doc_preview.set(false);
            shown_tick.res_doc_preview.set(false);
            shown_tick.editing_comment.set(false);
            shown_tick.repeating.set(false);
            shown_tick.msg_search.set(String::new());
            shown_tick.msg_dir.set(0);
            shown_tick.msg_kind.set(0);
            detail_scroll_tick.scroll_to_offset(px(0.0), false);
            detail_top_left.replace(true);
        } else if detail_top_left.get() {
            detail_scroll_tick.scroll_to_offset(px(0.0), false);
            detail_top_left.replace(false);
        }
        let (selected_rev, selected_done, baseline_rev) =
            if selected_now.is_some() || baseline_now.is_some() {
                controls_tick.hub.with_session(|session| {
                    let selected_flow = selected_now.and_then(|id| session.get(id));
                    (
                        selected_flow.map(|flow| flow.rev),
                        selected_flow
                            .map(|flow| flow.finished_at.is_some())
                            .unwrap_or(true),
                        baseline_now.and_then(|id| session.get(id).map(|flow| flow.rev)),
                    )
                })
            } else {
                (None, true, None)
            };
        let detail_stamp = (
            false,
            selected_now,
            selected_rev,
            baseline_now,
            baseline_rev,
            pretty_now,
            lang_now,
        );
        if detail_stamp != seen_detail_stamp.get() || spill_expired {
            let prev = seen_detail_stamp.get();
            let view_changed = prev.1 != selected_now
                || prev.3 != baseline_now
                || prev.5 != pretty_now
                || prev.6 != lang_now;
            let streaming = selected_now.is_some() && !selected_done;
            let due = clock::elapsed(detail_hold.get()) >= Duration::from_millis(200);
            if !streaming || view_changed || spill_expired || due {
                seen_detail_stamp.replace(detail_stamp);
                detail_hold.replace(clock::now());
                if prev.1 != selected_now || prev.5 != pretty_now {
                    reset_body_offsets(shown_tick);
                }
                publish_detail(
                    &shown_tick,
                    &controls_tick.hub,
                    selected_now,
                    lang_now,
                    baseline_now,
                );
            }
        }
    });

    let controls_start = controls.clone();
    let controls_ca = controls.clone();
    let controls_remove = controls.clone();
    let controls_proxy = controls.clone();
    let controls_clear = controls.clone();
    let controls_clear_unpinned = controls.clone();
    let controls_clear_key = controls_clear.clone();
    let controls_replay_key = controls.clone();
    let controls_export = controls.clone();
    let controls_env = controls.clone();
    let selected_clear = selected;
    let baseline_clear = baseline;
    let compose_clear = compose;
    let selected_clear_unpinned = selected;
    let baseline_clear_unpinned = baseline;
    let compose_clear_unpinned = compose;
    let controls_detail = controls.clone();
    let controls_break = controls.clone();
    let controls_list = controls.clone();
    let controls_split = controls.clone();
    let controls_import = controls.clone();
    let controls_drop = controls.clone();
    let folded = signal(0_u8);
    let drop_langs = langs;
    let filter_focus = element_ref();
    let rows_step = rows;
    let selected_step = selected;
    let compose_step = compose;
    on_key_with(
        "up",
        KeyOptions {
            repeat: true,
            ..KeyOptions::default()
        },
        move |_| step_selected(rows_step, selected_step, compose_step, -1),
    );
    on_key_with(
        "down",
        KeyOptions {
            repeat: true,
            ..KeyOptions::default()
        },
        move |_| step_selected(rows_step, selected_step, compose_step, 1),
    );
    let selected_clear_key = selected;
    let baseline_clear_key = baseline;
    let compose_clear_key = compose;
    let seq_clear_key = seq_builder;
    let shown_clear_key = shown;
    on_key("mod+k", move |_| {
        clear_captured(
            &controls_clear_key,
            selected_clear_key,
            baseline_clear_key,
            compose_clear_key,
            seq_clear_key,
            shown_clear_key,
        );
    });
    on_key("mod+f", move |_| filter_focus.focus());
    let selected_replay = selected;
    let controls_submit_key = controls_replay_key.clone();
    on_key("mod+r", move |_| {
        replay_flow(
            &controls_replay_key,
            selected_replay.get(),
            ReplayOverride::default(),
        );
    });
    let compose_submit_key = compose;
    let selected_submit_key = selected;
    let paused_submit_key = paused;
    on_key_with(
        "mod+enter",
        KeyOptions {
            when: nsh_core::keymap::When::Always,
            ..KeyOptions::default()
        },
        move |_| {
            if compose_submit_key.composing.get() {
                submit_compose(
                    &controls_submit_key,
                    compose_submit_key,
                    selected_submit_key,
                    paused_submit_key,
                );
            }
        },
    );
    let selected_esc = selected;
    let compose_esc = compose;
    let seq_esc = seq_builder;
    let shown_esc = shown;
    on_key("escape", move |_| {
        if compose_esc.composing.get() {
            reset_compose(compose_esc);
        } else if seq_esc.open.get() {
            close_sequence_diagram(seq_esc);
        } else if seq_esc.redact_open.get() {
            seq_esc.redact_open.set(false);
        } else if shown_esc.repeating.get() {
            shown_esc.repeating.set(false);
        } else if shown_esc.editing_comment.get() {
            shown_esc.editing_comment.set(false);
        } else if selected_esc.get().is_some() {
            selected_esc.set(None);
        }
    });

    view()
        .flex_1()
        .bg(bg_canvas())
        .text_color(fg_primary())
        .on_file_drop(move |event: &FileDropEvent| {
            import_dropped(&controls_drop, event, ui_lang(drop_langs), profile_signals);
        })
        .child(toolbar(
            status,
            running,
            ca_status,
            use_system_proxy,
            move || install_ca(controls_ca.clone()),
            move || remove_ca(controls_remove.clone()),
            move || toggle_system_proxy(controls_proxy.clone()),
            move || {
                clear_captured(
                    &controls_clear,
                    selected_clear,
                    baseline_clear,
                    compose_clear,
                    seq_builder,
                    shown,
                );
            },
            move || {
                clear_unpinned_captured(
                    &controls_clear_unpinned,
                    selected_clear_unpinned,
                    baseline_clear_unpinned,
                    compose_clear_unpinned,
                    seq_builder,
                    shown,
                );
            },
            move || export_har_file(controls_export.clone(), &effective_filter.get()),
            move || copy_proxy_env(&controls_env),
            move || toggle_capture(controls_start.clone()),
            controls_import,
            rows,
            flow_total,
            effective_filter,
            shown.pretty,
            shown.phase,
            share_id,
            profile_signals,
            compose,
            seq_builder,
            selected,
            update_state,
        ))
        .child(when(
            move || paused.get().is_some(),
            move || {
                break_bar(
                    paused,
                    paused_count,
                    paused_queue,
                    compose,
                    selected,
                    controls_break.clone(),
                )
            },
        ))
        .child(
            view()
                .flex_1()
                .flex_row()
                .child(flow_list(
                    rows,
                    facets,
                    facets_open,
                    selected,
                    paused,
                    compose,
                    filter,
                    list_filter,
                    effective_filter,
                    rules_text,
                    list_width,
                    columns,
                    sort,
                    controls_list,
                    shown,
                    baseline,
                    filter_focus,
                ))
                .child(column_splitter(list_width, controls_split))
                .child(detail_pane(
                    shown,
                    controls_detail,
                    selected,
                    paused,
                    compose,
                    seq_builder,
                    rows,
                    list_filter,
                    effective_filter,
                    rules_text,
                    baseline,
                    folded,
                    pane_px,
                    filtered_stats,
                    detail_scroll,
                )),
        )
        .child(footer(
            listen, running, memory, dropped, port_text, lan_url, lan_choice, lan_addrs, qr_path,
            listen_all, controls,
        ))
        .child(each(
            move || {
                if toast_shown.with(Vec::is_empty) {
                    Vec::new()
                } else {
                    vec![toast_overlay_gen.get()]
                }
            },
            |id| *id,
            move |_, _| {
                overlay()
                    .inset_0()
                    .flex_col()
                    .items_center()
                    .pt(px(54.0))
                    .pointer_events(PointerEvents::None)
                    .child(
                        toast_stack()
                            .placement(Anchor::TopCenter)
                            .state(toast_state)
                            .w(move || px((window().width.0 - 24.0).clamp(220.0, 380.0)))
                            .max_w(move || px((window().width.0 - 24.0).max(200.0)))
                            .pointer_events(PointerEvents::Auto)
                            .child(each(
                                toast_shown,
                                |(id, _, _)| *id,
                                move |item, _| {
                                    let id = item.with(|(id, _, _)| *id);
                                    let dismiss_manager = toast_manager;
                                    let dismiss_shown = toast_shown;
                                    let dismiss_one = move || {
                                        let now = nsh_core::time::now();
                                        let next = dismiss_manager.with(|m| {
                                            m.dismiss(&id, now);
                                            m.visible(3)
                                                .map(|(id, data, status)| {
                                                    (*id, data.clone(), status)
                                                })
                                                .collect::<Vec<_>>()
                                        });
                                        dismiss_shown.set(next);
                                    };
                                    toast()
                                        .transition_status(move || item.with(|(_, _, s)| *s))
                                        .on_dismiss_request(dismiss_one)
                                        .rounded(px(10.0))
                                        .border_1()
                                        .border_color(border_strong())
                                        .bg(bg_card())
                                        .shadow_lg()
                                        .transition_all()
                                        .duration(180.0)
                                        .starting_class("opacity-0 -translate-y-2")
                                        .ending_class("opacity-0 scale-95")
                                        .child(
                                            view()
                                                .w_full()
                                                .px_3p5()
                                                .py_3()
                                                .flex_row()
                                                .items_center()
                                                .gap_3()
                                                .cursor(Cursor::Pointer)
                                                .on_press(dismiss_one)
                                                .child(
                                                    view()
                                                        .w(px(32.0))
                                                        .h(px(32.0))
                                                        .rounded(px(8.0))
                                                        .flex_shrink_0()
                                                        .flex_row()
                                                        .items_center()
                                                        .justify_center()
                                                        .bg(move || {
                                                            if item.with(|(_, msg, _)| {
                                                                msg.toast_kind() == Some(true)
                                                            }) {
                                                                status_warn_bg()
                                                            } else {
                                                                status_ok_bg()
                                                            }
                                                        })
                                                        .child(glyph_px(
                                                            16.0,
                                                            move || {
                                                                if item.with(|(_, msg, _)| {
                                                                    msg.toast_kind() == Some(true)
                                                                }) {
                                                                    Glyph::Alert
                                                                } else {
                                                                    Glyph::Check
                                                                }
                                                            },
                                                            move || {
                                                                if item.with(|(_, msg, _)| {
                                                                    msg.toast_kind() == Some(true)
                                                                }) {
                                                                    color(status_warn_fg())
                                                                } else {
                                                                    color(status_ok_fg())
                                                                }
                                                            },
                                                        )),
                                                )
                                                .child(
                                                    view()
                                                        .flex_1()
                                                        .min_w(px(0.0))
                                                        .flex_col()
                                                        .gap(px(2.0))
                                                        .child(
                                                            text(move || {
                                                                let lang = ui_lang(ui_langs());
                                                                item.with(|(_, msg, _)| {
                                                                    msg.toast_title(lang)
                                                                        .to_string()
                                                                })
                                                            })
                                                            .text_size(px(13.0))
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_color(fg_primary()),
                                                        )
                                                        .child(
                                                            text(move || {
                                                                let lang = ui_lang(ui_langs());
                                                                item.with(|(_, msg, _)| {
                                                                    msg.text(lang)
                                                                })
                                                            })
                                                            .number_of_lines(3)
                                                            .text_overflow(EllipsizeMode::Middle)
                                                            .text_size(px(12.0))
                                                            .text_color(fg_secondary()),
                                                        ),
                                                )
                                                .child(
                                                    view()
                                                        .w(px(20.0))
                                                        .h(px(20.0))
                                                        .rounded(px(5.0))
                                                        .flex_shrink_0()
                                                        .flex_row()
                                                        .items_center()
                                                        .justify_center()
                                                        .hover(|s| s.bg(bg_control_hover()))
                                                        .active(|s| s.opacity(0.75))
                                                        .label(tr("关闭", "Dismiss"))
                                                        .child(glyph_px(
                                                            10.5,
                                                            || Glyph::Close,
                                                            || color(fg_muted()),
                                                        )),
                                                ),
                                        )
                                },
                            )),
                    )
            },
        ))
}

#[allow(clippy::too_many_arguments)]
fn toolbar(
    status: Signal<StatusMsg>,
    running: Signal<bool>,
    ca_status: Signal<ca::CaStatus>,
    use_system_proxy: Signal<bool>,
    install: impl Fn() + 'static,
    remove: impl Fn() + 'static,
    toggle_proxy: impl Fn() + 'static,
    clear: impl Fn() + 'static,
    clear_unpinned: impl Fn() + 'static,
    export_har: impl Fn() + 'static,
    copy_env: impl Fn() + 'static,
    toggle: impl Fn() + 'static,
    import_controls: Arc<Controls>,
    rows: Signal<Vec<Arc<RowLine>>>,
    flow_total: Signal<usize>,
    filter: Signal<String>,
    pretty: Signal<bool>,
    shown_phase: Signal<Phase>,
    share_id: Signal<String>,
    profiles: ProfileSignals,
    compose: Compose,
    seq_builder: SeqBuilder,
    selected: Signal<Option<u64>>,
    update_state: Signal<UpdateState>,
) -> impl IntoElement {
    let install = Rc::new(install);
    let clear = Rc::new(clear);
    let clear_unpinned = Rc::new(clear_unpinned);
    let remove = Rc::new(remove);
    let filter_controls = import_controls.clone();
    let deselect_controls = import_controls.clone();
    #[cfg(not(target_arch = "wasm32"))]
    let update_controls = import_controls.clone();
    const TOOLBAR_H: f32 = 42.0;
    let bar = view()
        .window_drag(true)
        .flex_row()
        .items_center()
        .gap_2()
        .pl(move || {
            #[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
            {
                let left = nsh_core::element::desktop::title_bar().left.0;
                if left > 0.0 {
                    return px(left + 6.0);
                }
            }
            px(LIST_INSET)
        })
        .pr_3()
        .h(px(TOOLBAR_H))
        .bg(bg_chrome())
        .border_b_1()
        .border_b_color(border_strong())
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .pr_1()
                .child(
                    view()
                        .w(px(20.0))
                        .h(px(20.0))
                        .rounded(px(5.5))
                        .bg(accent_blue())
                        .items_center()
                        .justify_center()
                        .child(
                            mono("Z")
                                .text_size(px(12.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(white()),
                        ),
                )
                .child(
                    text("zniff")
                        .text_size(px(14.5))
                        .font_weight(FontWeight::BOLD)
                        .text_color(fg_primary()),
                ),
        )
        .child(view().w(px(1.0)).h(px(16.0)).bg(border_strong()))
        .child({
            let langs = ui_langs();
            let startup_error = move || match status.get() {
                msg @ (StatusMsg::BindFailed { .. }
                | StatusMsg::CaUnavailable(_)
                | StatusMsg::StopRestoreFailed(_)
                | StatusMsg::ProxyApplyFailed { .. }
                | StatusMsg::ProxyRestoreFailed(_)) => Some(msg.text(ui_lang(langs))),
                _ => None,
            };
            let active_name = move || {
                toolbar_active_profile_name(
                    &profiles.saved.get(),
                    &profiles.name.get(),
                    &profiles.filter.get(),
                    &profiles.bypass.get(),
                    &profiles.rules.get(),
                    &profiles.upstream.get(),
                )
            };
            let condition_info = move || {
                toolbar_filter_conditions(
                    &filter.get(),
                    &profiles.bypass.get(),
                    &profiles.rules.get(),
                    &profiles.upstream.get(),
                    profiles.rules_on.get(),
                    ui_lang(langs),
                )
            };
            view()
                .flex_1()
                .flex_row()
                .items_center()
                .gap_2()
                .px_1()
                .min_w(px(0.0))
                .child(
                    view()
                        .w(px(7.0))
                        .h(px(7.0))
                        .rounded_full()
                        .flex_shrink_0()
                        .bg(move || {
                            if startup_error().is_some() {
                                status_warn_fg()
                            } else if running.get() {
                                status_ok_fg()
                            } else {
                                fg_muted()
                            }
                        }),
                )
                .child(when(
                    move || startup_error().is_none() && active_name().is_some(),
                    move || {
                        let deselect_controls = deselect_controls.clone();
                        view()
                            .window_drag(false)
                            .h(px(22.0))
                            .max_w(px(184.0))
                            .min_w(px(0.0))
                            .rounded(px(5.0))
                            .overflow_hidden()
                            .flex_row()
                            .items_center()
                            .border_1()
                            .border_color(accent_blue_border())
                            .bg(accent_blue_soft())
                            .child(
                                view()
                                    .h_full()
                                    .max_w(px(162.0))
                                    .min_w(px(0.0))
                                    .pl_2()
                                    .pr_1p5()
                                    .flex_row()
                                    .items_center()
                                    .cursor(Cursor::Pointer)
                                    .hover(|style| style.bg(accent_blue_soft_hover()))
                                    .active(|style| style.opacity(0.8))
                                    .tooltip(label_tip(
                                        "打开过滤与规则配置",
                                        "Open filters & rules config",
                                    ))
                                    .on_press(move || {
                                        profiles.panel_open.set(true);
                                    })
                                    .child(
                                        text(move || active_name().unwrap_or_default())
                                            .max_w(px(148.0))
                                            .min_w(px(0.0))
                                            .number_of_lines(1)
                                            .text_overflow(EllipsizeMode::Tail)
                                            .text_size(px(11.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(accent_blue_text()),
                                    ),
                            )
                            .child(
                                view()
                                    .w(px(1.0))
                                    .h(px(12.0))
                                    .flex_shrink_0()
                                    .bg(accent_blue_border()),
                            )
                            .child(
                                view()
                                    .w(px(20.0))
                                    .h(px(20.0))
                                    .flex_shrink_0()
                                    .flex_row()
                                    .items_center()
                                    .justify_center()
                                    .cursor(Cursor::Pointer)
                                    .hover(|style| style.bg(accent_blue_soft_hover()))
                                    .active(|style| style.opacity(0.75))
                                    .label(tr("取消选择配置", "Deselect setup"))
                                    .tooltip(label_tip(
                                        "取消选择当前配置",
                                        "Deselect current setup",
                                    ))
                                    .on_press(move || {
                                        deselect_active_profile(profiles, &deselect_controls);
                                    })
                                    .child(glyph_px(
                                        10.0,
                                        || Glyph::Close,
                                        || color(accent_blue_text()),
                                    )),
                            )
                    },
                ))
                .child(
                    when(
                        move || startup_error().is_none() && condition_info().1,
                        move || {
                            mono(move || condition_info().0)
                                .flex_1()
                                .min_w(px(0.0))
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Tail)
                                .text_size(px(12.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg_secondary())
                        },
                    )
                    .otherwise(move || {
                        text(move || {
                            if let Some(err) = startup_error() {
                                return err;
                            }
                            condition_info().0
                        })
                        .flex_1()
                        .min_w(px(0.0))
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if startup_error().is_some() {
                                status_warn_fg()
                            } else {
                                fg_muted()
                            }
                        })
                    }),
                )
        });
    #[cfg(not(target_arch = "wasm32"))]
    let bar = bar.child(toolbar_update_pill(update_state, update_controls));
    bar.child(flow_count_badge(rows, flow_total, filter, move || {
        reset_compose(compose);
        close_sequence_diagram(seq_builder);
        selected.set(None);
    }))
    .child(toolbar_text_button(tr("统计", "Summary"), move || {
        reset_compose(compose);
        close_sequence_diagram(seq_builder);
        selected.set(None);
    }))
    .child(toolbar_text_button(tr("清空", "Clear"), {
        let clear = Rc::clone(&clear);
        move || clear()
    }))
    .child(saved_filter_menu(profiles, filter_controls))
    .child(toolbar_menu(
        ca_status,
        use_system_proxy,
        install,
        remove,
        toggle_proxy,
        export_har,
        copy_env,
        {
            let clear = Rc::clone(&clear);
            move || clear()
        },
        {
            let clear_unpinned = Rc::clone(&clear_unpinned);
            move || clear_unpinned()
        },
        import_controls,
        rows,
        flow_total,
        filter,
        pretty,
        shown_phase,
        share_id,
        compose,
        seq_builder,
        selected,
        update_state,
    ))
    .child(primary_button(
        {
            let langs = ui_langs();
            move || -> SharedString {
                if running.get() {
                    t(ui_lang(langs), "停止", "Stop").into()
                } else {
                    t(ui_lang(langs), "开始", "Start").into()
                }
            }
        },
        move || running.get(),
        toggle,
    ))
}

fn toolbar_text_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .window_drag(false)
        .h(px(CONTROL_H))
        .px_3()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(border_control())
        .bg(bg_control())
        .hover(|style| style.bg(bg_control_hover()))
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.8))
        .on_press(on_press)
        .child(control_text(label).text_color(fg_primary()))
}

const SAVED_RULE_CAP: usize = 16;

fn saved_filter_menu(profiles: ProfileSignals, controls: Arc<Controls>) -> impl IntoElement {
    let filter = profiles.filter;
    let saved = profiles.saved;
    let bypass_text = profiles.bypass;
    let rules_text = profiles.rules;
    let upstream_text = profiles.upstream;
    let rules_on = profiles.rules_on;
    let breaks_on = profiles.breaks_on;
    let active_rules_count = move || crate::rules::parse_rules(&rules_text.get()).len();
    let open = profiles.panel_open;
    let is_tinted = move || open.get() || !filter.get().trim().is_empty();
    popover()
        .open(move || Some(open.get()))
        .on_open_change(move |event: &nsh_base::PopoverEvent| {
            if let nsh_base::PopoverEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(10.0))
        .trigger(
            view()
                .window_drag(false)
                .h(px(CONTROL_H))
                .px_3()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_1p5()
                .rounded(px(CONTROL_RADIUS))
                .border_1()
                .border_color(move || {
                    if capture_muted(rules_on, breaks_on) {
                        status_warn_fg()
                    } else if open.get()
                        || !filter.get().trim().is_empty()
                        || !saved.with(Vec::is_empty)
                        || config_set(bypass_text, rules_text, upstream_text)
                    {
                        accent_blue_border()
                    } else {
                        border_control()
                    }
                })
                .bg(move || {
                    if is_tinted() {
                        accent_blue_soft()
                    } else {
                        bg_control()
                    }
                })
                .hover(|style| {
                    style.bg(move || {
                        if is_tinted() {
                            accent_blue_soft_hover()
                        } else {
                            bg_control_hover()
                        }
                    })
                })
                .active(|style| style.opacity(0.8))
                .cursor(Cursor::Pointer)
                .tooltip(label_tip("过滤与规则配置", "Filters & Rules Config"))
                .child(when(
                    move || {
                        !filter.get().trim().is_empty()
                            || config_set(bypass_text, rules_text, upstream_text)
                            || capture_muted(rules_on, breaks_on)
                    },
                    move || {
                        view().w(px(6.0)).h(px(6.0)).rounded_full().bg(move || {
                            if capture_muted(rules_on, breaks_on) {
                                status_warn_fg()
                            } else if !filter.get().trim().is_empty() {
                                accent_blue()
                            } else {
                                status_ok_fg()
                            }
                        })
                    },
                ))
                .child(glyph_px(
                    13.5,
                    || Glyph::Sliders,
                    move || {
                        color(if capture_muted(rules_on, breaks_on) {
                            status_warn_fg()
                        } else if is_tinted() || config_set(bypass_text, rules_text, upstream_text)
                        {
                            accent_blue_text()
                        } else {
                            fg_secondary()
                        })
                    },
                ))
                .child(control_text(tr("过滤", "Filters")).text_color(move || {
                    if capture_muted(rules_on, breaks_on) {
                        status_warn_fg()
                    } else if is_tinted() {
                        accent_blue_text()
                    } else {
                        fg_primary()
                    }
                }))
                .child(when(
                    move || active_rules_count() > 0,
                    move || {
                        view()
                            .h(px(16.0))
                            .px_1()
                            .rounded(px(4.0))
                            .flex_row()
                            .items_center()
                            .justify_center()
                            .bg(accent_blue_soft())
                            .child(
                                mono(move || active_rules_count().to_string())
                                    .tabular_nums()
                                    .text_size(px(10.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(accent_blue_text()),
                            )
                    },
                )),
        )
        .child(saved_filter_panel(profiles, controls))
}

fn saved_filter_panel(profiles: ProfileSignals, controls: Arc<Controls>) -> impl IntoElement {
    let filter = profiles.filter;
    let bypass_text = profiles.bypass;
    let rules_text = profiles.rules;
    let upstream_text = profiles.upstream;
    let rules_on = profiles.rules_on;
    let breaks_on = profiles.breaks_on;
    let show_tunnels = profiles.show_tunnels;
    let list_width = profiles.list_width;
    let field = filter;
    let reset_filter = filter;
    let host_field = signal(extract_host_or(&filter.get_untracked()));
    let path_field = signal(extract_path_or(&filter.get_untracked()));
    let exclude_field = signal(extract_exclude_or(&filter.get_untracked()));
    effect(move || {
        let current = filter.get();
        if extract_host_or(&set_host_or("", &host_field.get_untracked()))
            != extract_host_or(&current)
        {
            host_field.set(extract_host_or(&current));
        }
        if extract_path_or(&set_path_or("", &path_field.get_untracked()))
            != extract_path_or(&current)
        {
            path_field.set(extract_path_or(&current));
        }
        if extract_exclude_or(&set_exclude_or("", &exclude_field.get_untracked()))
            != extract_exclude_or(&current)
        {
            exclude_field.set(extract_exclude_or(&current));
        }
    });

    let bypass_controls = controls.clone();
    let rules_controls = controls.clone();
    let rules_save_controls = controls.clone();
    let rules_clear_controls = controls.clone();
    let upstream_controls = controls.clone();
    let proxy_auth_text = profiles.proxy_auth;
    let proxy_auth_controls = controls.clone();
    let relay_url_text = profiles.relay_url;
    let relay_token_text = signal(lock_string(&controls.relay_token));
    let relay_accept_on = profiles.relay_accept;
    let relay_url_controls = controls.clone();
    let relay_token_controls = controls.clone();
    let relay_accept_controls = controls.clone();
    let tunnel_controls = controls;
    let draft_host = signal(String::new());
    let draft_path = signal(String::new());
    let draft_op = signal(String::new());
    let draft_from = signal(String::new());
    let draft_to = signal(String::new());
    let draft_modes = QuickRuleModeSignals::new();
    let draft_kind = signal(0_u8);
    let import_open = signal(false);

    view()
        .w(move || {
            px(adaptive_filter_panel_width(
                window().width.0,
                list_width.get(),
            ))
        })
        .max_w(move || px((window().width.0 - 16.0).max(280.0)))
        .max_h(move || px(adaptive_filter_panel_max_height(window().height.0)))
        .flex_col()
        .overflow_hidden()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
        .child(
            scroll()
                .w_full()
                .max_h(move || {
                    px(adaptive_filter_body_max_height(
                        window().height.0,
                        import_open.get(),
                    ))
                })
                .child(
                    view()
                        .w_full()
                        .p_3()
                        .gap_3()
                        .flex_row()
                        .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_2()
                        .child(
                            view()
                                .h(px(28.0))
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .child(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            text(tr("流量过滤", "Traffic Filter"))
                                                .text_size(px(13.0))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(fg_primary()),
                                        )
                                        .child(
                                            text(tr(
                                                "多项用 | 或逗号分隔",
                                                "Separate OR items with | or comma",
                                            ))
                                            .text_size(px(11.0))
                                            .text_color(fg_muted()),
                                        ),
                                )
                                .child(view().flex_row().items_center().gap_1p5().child(when(
                                    move || !reset_filter.get().trim().is_empty(),
                                    move || {
                                        view()
                                            .h(px(24.0))
                                            .px_2()
                                            .rounded(px(6.0))
                                            .border_1()
                                            .border_color(border_control())
                                            .bg(bg_control())
                                            .hover(|style| style.bg(bg_control_hover()))
                                            .active(|style| style.opacity(0.8))
                                            .flex_row()
                                            .items_center()
                                            .cursor(Cursor::Pointer)
                                            .on_press(move || reset_filter.set(String::new()))
                                            .child(
                                                text(tr("清空条件", "Reset"))
                                                    .text_size(px(11.5))
                                                    .text_color(fg_secondary()),
                                            )
                                    },
                                ))),
                        )
                        .child(config_field(
                            "过滤表达式",
                            "Filter Expression",
                            focused_input(
                                field,
                                tr(
                                    "host:a.com path:/api type:json，支持 | 表示 OR",
                                    "host:a.com path:/api type:json, use | for OR",
                                ),
                                28.0,
                                false,
                                move |text| field.set(text.to_string()),
                                None,
                            ),
                        ))
                        .child(config_field(
                            "包含域名",
                            "Include Domains",
                            side_input(
                                host_field,
                                tr("api.example.com | *.cdn.com", "api.example.com | *.cdn.com"),
                                28.0,
                                false,
                                move |text| {
                                    host_field.set(text.to_string());
                                    filter.set(set_host_or(&filter.get(), text));
                                },
                            ),
                        ))
                        .child(config_field(
                            "包含 Path 前缀",
                            "Include Path Prefixes",
                            side_input(
                                path_field,
                                tr("/api/v1 | /auth | /graphql", "/api/v1 | /auth | /graphql"),
                                28.0,
                                false,
                                move |text| {
                                    path_field.set(text.to_string());
                                    filter.set(set_path_or(&filter.get(), text));
                                },
                            ),
                        ))
                        .child(config_field(
                            "排除域名或路径",
                            "Exclude Host / Path",
                            side_input(
                                exclude_field,
                                tr("ads.example.com | /health", "ads.example.com | /health"),
                                28.0,
                                false,
                                move |text| {
                                    exclude_field.set(text.to_string());
                                    filter.set(set_exclude_or(&filter.get(), text));
                                },
                            ),
                        ))
                        .child(
                            view()
                                .gap_1()
                                .w_full()
                                .child(
                                    text(tr("快捷筛选", "Quick Filters"))
                                        .text_size(px(11.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(fg_muted()),
                                )
                                .child(
                                    view()
                                        .w_full()
                                        .flex_row()
                                        .items_center()
                                        .gap_1p5()
                                        .child(filter_chip(
                                            filter,
                                            "JSON",
                                            "JSON",
                                            "type:json",
                                            JSON_ALIASES,
                                        ))
                                        .child(filter_chip(
                                            filter,
                                            "报错",
                                            "Errors",
                                            "失败",
                                            FAIL_ALIASES,
                                        ))
                                        .child(filter_chip(
                                            filter,
                                            "星标",
                                            "Starred",
                                            "pin:",
                                            PIN_ALIASES,
                                        ))
                                        .child(filter_chip(
                                            filter,
                                            "慢请求",
                                            "Slow",
                                            "slow:",
                                            SLOW_ALIASES,
                                        )),
                                ),
                        ),
                )
                .child(view().w(px(1.0)).self_stretch().bg(border_subtle()))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_2()
                        .child(
                            view()
                                .h(px(28.0))
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .child(
                                    text(tr("规则与代理配置", "Rules & Proxy Config"))
                                        .text_size(px(13.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(fg_primary()),
                                )
                                .child(when(
                                    move || {
                                        !crate::rules::parse_rules(&rules_text.get()).is_empty()
                                    },
                                    move || {
                                        view()
                                            .h(px(20.0))
                                            .px_2()
                                            .rounded(px(99.0))
                                            .flex_row()
                                            .items_center()
                                            .bg(accent_blue_soft())
                                            .child(
                                                text(move || {
                                                    let count = crate::rules::parse_rules(
                                                        &rules_text.get(),
                                                    )
                                                    .len();
                                                    let lang = ui_lang(ui_langs());
                                                    match lang {
                                                        Lang::Zh => format!("{count} 条规则生效"),
                                                        Lang::En => format!("{count} active"),
                                                    }
                                                })
                                                .text_size(px(11.0))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(accent_blue_text()),
                                            )
                                    },
                                )),
                        )
                        .child(
                            view()
                                .w_full()
                                .flex_row()
                                .gap_1p5()
                                .child(enable_switch(rules_on, "启用规则", "Rules", {
                                    let controls = rules_controls.clone();
                                    move |enabled| {
                                        controls.hub.set_rules_enabled(enabled);
                                    }
                                }))
                                .child(enable_switch(breaks_on, "启用断点", "Breaks", {
                                    let controls = rules_controls.clone();
                                    move |enabled| {
                                        controls.hub.set_breaks_enabled(enabled);
                                    }
                                }))
                                .child(enable_switch(
                                    show_tunnels,
                                    "隧道失败",
                                    "Tunnel errors",
                                    move |on| {
                                        tunnel_controls.hub.set_show_tunnel_failures(on);
                                        if let Err(err) =
                                            save_config(|config| config.show_tunnel_failures = on)
                                        {
                                            set_status(
                                                &tunnel_controls,
                                                StatusMsg::PrefUnsaved {
                                                    name: PrefName::Tunnel,
                                                    err: err.to_string(),
                                                },
                                            );
                                        }
                                    },
                                )),
                        )
                        .child(config_field(
                            "直通名单 (不解密)",
                            "TLS Bypass (No MITM)",
                            side_input(
                                bypass_text,
                                tr(
                                    "example.com | *.pin.dev 或 only:api.example.com",
                                    "example.com | *.pin.dev or only:api.example.com",
                                ),
                                28.0,
                                false,
                                move |text| {
                                    bypass_text.set(text.to_string());
                                    bypass_controls.hub.set_bypass(text);
                                    if let Err(err) =
                                        save_config(|config| config.bypass_hosts = text.to_string())
                                    {
                                        set_status(
                                            &bypass_controls,
                                            StatusMsg::PrefUnsaved {
                                                name: PrefName::Bypass,
                                                err: err.to_string(),
                                            },
                                        );
                                    }
                                },
                            ),
                        ))
                        .child(
                            view()
                                .gap_1()
                                .w_full()
                                .child(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            text(tr("转发与改写规则", "Rules"))
                                                .text_size(px(11.5))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(fg_muted()),
                                        )
                                        .child(
                                            view()
                                                .flex_row()
                                                .items_center()
                                                .gap_1p5()
                                                .child(when(
                                                    move || {
                                                        !crate::rules::invalid_rule_lines(
                                                            &rules_text.get(),
                                                        )
                                                        .is_empty()
                                                    },
                                                    move || {
                                                        view()
                                                            .h(px(20.0))
                                                            .px_1p5()
                                                            .rounded(px(4.0))
                                                            .flex_row()
                                                            .items_center()
                                                            .bg(status_warn_bg())
                                                            .child(
                                                                text(move || {
                                                                    let bad = crate::rules::invalid_rule_lines(
                                                                        &rules_text.get(),
                                                                    );
                                                                    let lines = bad
                                                                        .iter()
                                                                        .map(usize::to_string)
                                                                        .collect::<Vec<_>>()
                                                                        .join(", ");
                                                                    let lang = ui_lang(ui_langs());
                                                                    match lang {
                                                                        Lang::Zh => {
                                                                            format!("第 {lines} 行语法有误")
                                                                        }
                                                                        Lang::En => {
                                                                            format!("Syntax error on line {lines}")
                                                                        }
                                                                    }
                                                                })
                                                                .text_size(px(10.5))
                                                                .font_weight(FontWeight::MEDIUM)
                                                                .text_color(status_warn_fg()),
                                                            )
                                                    },
                                                ))
                                                .child(when(
                                                    move || !rules_text.get().trim().is_empty(),
                                                    move || {
                                                        let clear_controls =
                                                            rules_clear_controls.clone();
                                                        view()
                                                            .h(px(24.0))
                                                            .px_2()
                                                            .flex_shrink_0()
                                                            .rounded(px(6.0))
                                                            .border_1()
                                                            .border_color(border_control())
                                                            .bg(bg_control())
                                                            .hover(|style| {
                                                                style.bg(bg_control_hover())
                                                            })
                                                            .active(|style| style.opacity(0.8))
                                                            .flex_row()
                                                            .items_center()
                                                            .cursor(Cursor::Pointer)
                                                            .on_press(move || {
                                                                rules_text.set(String::new());
                                                                clear_controls.hub.set_rules("");
                                                                let _ = save_config(|config| {
                                                                    config.rules.clear()
                                                                });
                                                            })
                                                            .child(
                                                                text(tr("清空", "Clear"))
                                                                    .text_size(px(11.5))
                                                                    .text_color(fg_secondary()),
                                                            )
                                                    },
                                                )),
                                        ),
                                )
                                .child(rules_editor(
                                    rules_text,
                                    rules_controls.clone(),
                                    import_open,
                                )),
                        )
                        .child(replace_row(
                            rules_text,
                            rules_controls,
                            draft_host,
                            draft_path,
                            draft_op,
                            draft_from,
                            draft_to,
                            draft_modes,
                            draft_kind,
                        ))
                        .child(config_field(
                            "上游代理",
                            "Upstream Proxy",
                            side_input(
                                upstream_text,
                                tr(
                                    "[user:pass@]host:port，空着直连",
                                    "[user:pass@]host:port, empty = direct",
                                ),
                                28.0,
                                false,
                                move |text| {
                                    upstream_text.set(text.to_string());
                                    upstream_controls.hub.set_upstream(text);
                                    if let Err(err) = save_config(|config| {
                                        config.upstream_proxy = text.to_string()
                                    }) {
                                        set_status(
                                            &upstream_controls,
                                            StatusMsg::PrefUnsaved {
                                                name: PrefName::Upstream,
                                                err: err.to_string(),
                                            },
                                        );
                                    }
                                },
                            ),
                        ))
                        .child(config_field(
                            "代理认证",
                            "Proxy auth",
                            side_input(
                                proxy_auth_text,
                                tr(
                                    "user:pass，空着不校验",
                                    "user:pass, empty disables auth",
                                ),
                                28.0,
                                false,
                                move |text| {
                                    proxy_auth_text.set(text.to_string());
                                    let stored = text.trim().to_string();
                                    proxy_auth_controls.hub.set_proxy_auth(&stored);
                                    if let Err(err) =
                                        save_config(|config| config.proxy_auth = stored)
                                    {
                                        set_status(
                                            &proxy_auth_controls,
                                            StatusMsg::PrefUnsaved {
                                                name: PrefName::ProxyAuth,
                                                err: err.to_string(),
                                            },
                                        );
                                    }
                                },
                            ),
                        ))
                        .child(config_field(
                            "中转地址",
                            "Relay address",
                            side_input(
                                relay_url_text,
                                tr(
                                    "http://主机:端口，空着只在本机分享",
                                    "http://host:port, empty shares locally",
                                ),
                                28.0,
                                false,
                                move |text| {
                                    relay_url_text.set(text.to_string());
                                    let stored = text.trim().to_string();
                                    *relay_url_controls
                                        .relay_url
                                        .lock()
                                        .unwrap_or_else(|err| err.into_inner()) = stored.clone();
                                    if let Err(err) =
                                        save_config(|config| config.relay_url = stored)
                                    {
                                        set_status(
                                            &relay_url_controls,
                                            StatusMsg::PrefUnsaved {
                                                name: PrefName::Relay,
                                                err: err.to_string(),
                                            },
                                        );
                                    }
                                },
                            ),
                        ))
                        .child(config_field(
                            "中转密钥",
                            "Relay token",
                            side_input(
                                relay_token_text,
                                tr(
                                    "上传和接收用同一个密钥",
                                    "Same secret for upload and receive",
                                ),
                                28.0,
                                false,
                                move |text| {
                                    relay_token_text.set(text.to_string());
                                    let stored = text.trim().to_string();
                                    *relay_token_controls
                                        .relay_token
                                        .lock()
                                        .unwrap_or_else(|err| err.into_inner()) = stored.clone();
                                    relay_token_controls.hub.set_relay_token(&stored);
                                    if let Err(err) =
                                        save_config(|config| config.relay_token = stored)
                                    {
                                        set_status(
                                            &relay_token_controls,
                                            StatusMsg::PrefUnsaved {
                                                name: PrefName::Relay,
                                                err: err.to_string(),
                                            },
                                        );
                                    }
                                },
                            ),
                        ))
                        .child(
                            view().w_full().flex_row().child(enable_switch(
                                relay_accept_on,
                                "接收中继",
                                "Accept relay",
                                move |on| {
                                    relay_accept_controls.hub.set_relay_accept(on);
                                    if on && !relay_accept_controls.hub.relay_token_ready() {
                                        set_status(
                                            &relay_accept_controls,
                                            StatusMsg::RelayAcceptNeedToken,
                                        );
                                    }
                                    if let Err(err) =
                                        save_config(|config| config.relay_accept = on)
                                    {
                                        set_status(
                                            &relay_accept_controls,
                                            StatusMsg::PrefUnsaved {
                                                name: PrefName::Relay,
                                                err: err.to_string(),
                                            },
                                        );
                                    }
                                },
                            )),
                        ),
                ),
                ),
        )
        .child(profile_dock(
            profiles,
            draft_host,
            draft_path,
            draft_op,
            draft_from,
            draft_to,
            draft_modes,
            draft_kind,
            import_open,
            rules_save_controls,
        ))
}

fn matched_saved_profile(
    saved: &[config::SavedProfile],
    filter: &str,
    bypass: &str,
    rules: &str,
    upstream: &str,
) -> Option<String> {
    let filter = filter.trim();
    let bypass = bypass.trim();
    let rules = rules.trim();
    let upstream = upstream.trim();
    if filter.is_empty() && bypass.is_empty() && rules.is_empty() && upstream.is_empty() {
        return None;
    }
    saved
        .iter()
        .find(|preset| {
            preset.filter.trim() == filter
                && preset.bypass_hosts.trim() == bypass
                && preset.rules.trim() == rules
                && preset.upstream_proxy.trim() == upstream
        })
        .map(|preset| preset.name.clone())
}

fn toolbar_active_profile_name(
    saved: &[config::SavedProfile],
    typed_name: &str,
    filter: &str,
    bypass: &str,
    rules: &str,
    upstream: &str,
) -> Option<String> {
    let typed = typed_name.trim();
    if !typed.is_empty() {
        return Some(typed.to_string());
    }
    matched_saved_profile(saved, filter, bypass, rules, upstream)
}

fn redact_upstream_summary(upstream: &str) -> String {
    let trimmed = upstream.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let without_scheme = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("HTTP://"))
        .unwrap_or(trimmed);
    let authority = without_scheme
        .split_once('/')
        .map(|(head, _)| head)
        .unwrap_or(without_scheme);
    if let Some((userinfo, host_port)) = authority.rsplit_once('@') {
        let user = userinfo
            .split_once(':')
            .map(|(u, _)| u)
            .unwrap_or(userinfo)
            .trim();
        if user.is_empty() {
            return host_port.to_string();
        }
        return format!("{user}@{host_port}");
    }
    authority.to_string()
}

fn toolbar_filter_conditions(
    filter: &str,
    bypass: &str,
    rules: &str,
    upstream: &str,
    rules_enabled: bool,
    lang: Lang,
) -> (String, bool) {
    let filter = filter.trim();
    if !filter.is_empty() {
        return (filter.to_string(), true);
    }
    let mut parts = Vec::new();
    if rules_enabled {
        let preview = rule_preset_preview(rules);
        if !preview.is_empty() {
            parts.push(preview);
        }
    }
    let bypass = bypass.trim();
    if !bypass.is_empty() {
        parts.push(format!("bypass:{bypass}"));
    }
    let upstream = redact_upstream_summary(upstream);
    if !upstream.is_empty() {
        parts.push(format!("upstream:{upstream}"));
    }
    if !parts.is_empty() {
        return (parts.join(" · "), true);
    }
    (
        t(
            lang,
            "全部流量 · 未设置过滤条件",
            "All traffic · No filter set",
        )
        .to_string(),
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn profile_dock(
    profiles: ProfileSignals,
    draft_host: Signal<String>,
    draft_path: Signal<String>,
    draft_op: Signal<String>,
    draft_from: Signal<String>,
    draft_to: Signal<String>,
    draft_modes: QuickRuleModeSignals,
    draft_kind: Signal<u8>,
    import_open: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let rule_name = profiles.name;
    let filter = profiles.filter;
    let bypass_text = profiles.bypass;
    let rules_text = profiles.rules;
    let upstream_text = profiles.upstream;
    let rules_on = profiles.rules_on;
    let breaks_on = profiles.breaks_on;
    let show_tunnels = profiles.show_tunnels;
    let saved = profiles.saved;
    let history = profiles.history;
    let preset_controls = controls.clone();
    let history_controls = controls.clone();
    let blank_controls = controls.clone();
    let export_controls = controls.clone();
    let import_controls = controls.clone();
    let import_panel_controls = controls.clone();
    let save_controls = controls;
    let dropdown_open = signal(false);
    let history_open = signal(false);
    let import_text = signal(String::new());
    let matched_name = move || {
        saved.with(|items| {
            matched_saved_profile(
                items,
                &filter.get(),
                &bypass_text.get(),
                &rules_text.get(),
                &upstream_text.get(),
            )
        })
    };
    // The dropdown follows the name field. Adding a rule changes the live
    // text, so a content match would drop the current setup. Seed the name
    // once from that match, then leave it until 新建配置.
    let shown_name = move || {
        let typed = rule_name.get();
        let typed = typed.trim();
        if !typed.is_empty() {
            return Some(typed.to_string());
        }
        matched_name()
    };
    let adopted = signal(false);
    effect(move || {
        if adopted.get() {
            return;
        }
        let typed = rule_name.get();
        if !typed.trim().is_empty() {
            adopted.set(true);
            return;
        }
        if let Some(name) = matched_name() {
            rule_name.set(name);
            adopted.set(true);
        }
    });
    let deselect_rev = profiles.deselect_rev;
    let seen_deselect_rev = signal(deselect_rev.get_untracked());
    effect(move || {
        let rev = deselect_rev.get();
        if rev == seen_deselect_rev.get_untracked() {
            return;
        }
        seen_deselect_rev.set(rev);
        adopted.set(true);
        dropdown_open.set(false);
        history_open.set(false);
        clear_rule_draft(
            draft_host,
            draft_path,
            draft_op,
            draft_from,
            draft_to,
            draft_modes,
            draft_kind,
        );
    });

    view()
        .w_full()
        .flex_shrink_0()
        .px_3()
        .py_2p5()
        .gap_2()
        .border_t_1()
        .border_t_color(border_subtle())
        .child(when(
            move || import_open.get(),
            move || {
                let confirm_controls = import_panel_controls.clone();
                let ready = move || !import_text.get().trim().is_empty();
                view()
                    .w_full()
                    .p_2p5()
                    .gap_2()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(border_subtle())
                    .bg(bg_canvas())
                    .child(
                        view()
                            .w_full()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                text(tr(
                                    "粘贴 TOML 配置导入（支持 [[profiles]] 或单套配置）",
                                    "Paste TOML setup to import (supports [[profiles]] or single setup)",
                                ))
                                .flex_1()
                                .min_w(px(0.0))
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Tail)
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(fg_muted()),
                            )
                            .child(
                                view()
                                    .flex_shrink_0()
                                    .flex_row()
                                    .items_center()
                                    .gap_1p5()
                                    .child(when(
                                        move || !import_text.get().is_empty(),
                                        move || {
                                            view()
                                                .h(px(24.0))
                                                .px_2()
                                                .rounded(px(6.0))
                                                .border_1()
                                                .border_color(border_control())
                                                .bg(bg_control())
                                                .hover(|style| style.bg(bg_control_hover()))
                                                .active(|style| style.opacity(0.8))
                                                .flex_row()
                                                .items_center()
                                                .cursor(Cursor::Pointer)
                                                .on_press(move || import_text.set(String::new()))
                                                .child(
                                                    text(tr("清空", "Clear"))
                                                        .text_size(px(11.0))
                                                        .text_color(fg_secondary()),
                                                )
                                        },
                                    ))
                                    .child(
                                        view()
                                            .h(px(24.0))
                                            .px_2p5()
                                            .rounded(px(6.0))
                                            .border_1()
                                            .border_color(border_control())
                                            .bg(bg_control())
                                            .hover(|style| style.bg(bg_control_hover()))
                                            .active(|style| style.opacity(0.8))
                                            .flex_row()
                                            .items_center()
                                            .cursor(Cursor::Pointer)
                                            .on_press(move || {
                                                import_text.set(String::new());
                                                import_open.set(false);
                                            })
                                            .child(
                                                text(tr("取消", "Cancel"))
                                                    .text_size(px(11.0))
                                                    .text_color(fg_secondary()),
                                            ),
                                    )
                                    .child(
                                        view()
                                            .h(px(24.0))
                                            .px_2p5()
                                            .rounded(px(6.0))
                                            .border_1()
                                            .border_color(move || {
                                                if ready() {
                                                    accent_blue()
                                                } else {
                                                    border_control()
                                                }
                                            })
                                            .bg(move || {
                                                if ready() {
                                                    accent_blue()
                                                } else {
                                                    bg_control()
                                                }
                                            })
                                            .hover(|style| {
                                                style.bg(move || {
                                                    if ready() {
                                                        accent_blue_hover()
                                                    } else {
                                                        bg_control()
                                                    }
                                                })
                                            })
                                            .active(|style| {
                                                style.opacity(move || {
                                                    if ready() { 0.85_f32 } else { 1.0_f32 }
                                                })
                                            })
                                            .flex_row()
                                            .items_center()
                                            .cursor(Cursor::Pointer)
                                            .disabled(move || !ready())
                                            .disabled_style(|style| style.cursor(Cursor::Default))
                                            .on_press(move || {
                                                if !ready() {
                                                    return;
                                                }
                                                if import_profiles_text(
                                                    &confirm_controls,
                                                    &import_text.get(),
                                                    profiles,
                                                ) {
                                                    clear_rule_draft(
                                                        draft_host,
                                                        draft_path,
                                                        draft_op,
                                                        draft_from,
                                                        draft_to,
                                                        draft_modes,
                                                        draft_kind,
                                                    );
                                                    import_text.set(String::new());
                                                    import_open.set(false);
                                                }
                                            })
                                            .child(
                                                text(tr("确认导入", "Confirm Import"))
                                                    .text_size(px(11.0))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(move || {
                                                        if ready() {
                                                            AdaptiveColor::rgb(0xffffff, 0xffffff)
                                                        } else {
                                                            fg_muted()
                                                        }
                                                    }),
                                            ),
                                    ),
                            ),
                    )
                    .child(
                        view()
                            .w_full()
                            .h(px(116.0))
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(border_control())
                            .bg(bg_code())
                            .overflow_hidden()
                            .child(
                                editor()
                                    .value(move || SharedString::from(import_text.get()))
                                    .auto_focus(true)
                                    .language("toml")
                                    .line_numbers(true)
                                    .folding(false)
                                    .soft_wrap(true)
                                    .font_size(Points(12.0))
                                    .line_height(Points(18.0))
                                    .scroll_beyond_last_line(false)
                                    .placeholder(move || {
                                        SharedString::from(tr(
                                            "在此粘贴导出的 [[profiles]] TOML 内容...",
                                            "Paste exported [[profiles]] TOML content here...",
                                        )())
                                    })
                                    .on_change(move |text: &SharedString| {
                                        import_text.set(text.to_string());
                                    })
                                    .size_full(),
                            ),
                    )
            },
        ))
        .child(
            view()
                .w_full()
                .h(px(30.0))
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(
                    menu()
                        .open(move || Some(dropdown_open.get() && !saved.with(Vec::is_empty)))
                        .on_open_change(move |event: &nsh_base::MenuEvent| {
                            if let nsh_base::MenuEvent::OpenChange(next, _) = event {
                                dropdown_open.set(*next && !saved.with(Vec::is_empty));
                            }
                        })
                        .placement(Placement::Top)
                        .align(Align::Start)
                        .offset(px(6.0))
                        .margin(px(8.0))
                        .trigger(
                            view()
                                .min_w(px(132.0))
                                .max_w(px(176.0))
                                .h(px(30.0))
                                .px_2p5()
                                .flex_shrink_0()
                                .rounded(px(7.0))
                                .border_1()
                                .border_color(move || {
                                    if dropdown_open.get() || shown_name().is_some() {
                                        accent_blue_border()
                                    } else {
                                        border_control()
                                    }
                                })
                                .bg(move || {
                                    if dropdown_open.get() || shown_name().is_some() {
                                        accent_blue_soft()
                                    } else {
                                        bg_control()
                                    }
                                })
                                .hover(|style| {
                                    style.bg(move || {
                                        if saved.with(Vec::is_empty) {
                                            bg_control()
                                        } else if dropdown_open.get() || shown_name().is_some() {
                                            accent_blue_soft_hover()
                                        } else {
                                            bg_control_hover()
                                        }
                                    })
                                })
                                .active(|style| {
                                    style.opacity(move || {
                                        if saved.with(Vec::is_empty) {
                                            1.0_f32
                                        } else {
                                            0.8_f32
                                        }
                                    })
                                })
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .cursor(Cursor::Pointer)
                                .disabled(move || saved.with(Vec::is_empty))
                                .disabled_style(|style| style.cursor(Cursor::Default))
                                .child(
                                    view()
                                        .w(px(6.0))
                                        .h(px(6.0))
                                        .rounded_full()
                                        .flex_shrink_0()
                                        .bg(move || {
                                            if shown_name().is_some() {
                                                accent_blue()
                                            } else {
                                                fg_muted()
                                            }
                                        }),
                                )
                                .child(
                                    text({
                                        let langs = ui_langs();
                                        move || -> SharedString {
                                            let lang = ui_lang(langs);
                                            if let Some(name) = shown_name() {
                                                name.into()
                                            } else if saved.with(Vec::is_empty) {
                                                t(lang, "暂无已保存配置", "No saved setups").into()
                                            } else {
                                                t(lang, "选择已保存配置", "Saved setups").into()
                                            }
                                        }
                                    })
                                    .flex_auto()
                                    .min_w(px(0.0))
                                    .number_of_lines(1)
                                    .text_overflow(EllipsizeMode::Tail)
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(move || {
                                        if shown_name().is_some() {
                                            accent_blue_text()
                                        } else if saved.with(Vec::is_empty) {
                                            fg_muted()
                                        } else {
                                            fg_secondary()
                                        }
                                    }),
                                )
                                .child(when(
                                    move || !saved.with(Vec::is_empty),
                                    move || {
                                        view()
                                            .h(px(16.0))
                                            .px_1()
                                            .rounded(px(4.0))
                                            .flex_shrink_0()
                                            .flex_row()
                                            .items_center()
                                            .bg(bg_card())
                                            .child(
                                                mono(move || saved.with(Vec::len).to_string())
                                                    .tabular_nums()
                                                    .text_size(px(10.5))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(fg_muted()),
                                            )
                                    },
                                ))
                                .child(glyph_px(12.0, || Glyph::Chevron, || color(fg_muted()))),
                        )
                        .child(
                            view()
                                .w(px(360.0))
                                .max_w(move || px((window().width.0 - 24.0).clamp(220.0, 360.0)))
                                .max_h(move || px((window().height.0 - 96.0).clamp(140.0, 220.0)))
                                .overflow_hidden()
                                .rounded(px(10.0))
                                .border_1()
                                .border_color(border_strong())
                                .bg(bg_card())
                                .shadow_md()
                                .child(
                                    scroll()
                                        .w_full()
                                        .max_h(move || {
                                            px((window().height.0 - 98.0).clamp(138.0, 218.0))
                                        })
                                        .child(
                                            view()
                                                .w_full()
                                                .p_1()
                                                .gap_0p5()
                                                .child(when(
                                                    move || shown_name().is_some(),
                                                    {
                                                        let clear_controls =
                                                            preset_controls.clone();
                                                        move || {
                                                            let clear_controls =
                                                                clear_controls.clone();
                                                            view()
                                                                .w_full()
                                                                .gap_1()
                                                                .child(
                                                                    menu_item()
                                                                        .w_full()
                                                                        .h(px(28.0))
                                                                        .px_2()
                                                                        .rounded(px(6.0))
                                                                        .flex_row()
                                                                        .items_center()
                                                                        .gap_2()
                                                                        .hover(|style| {
                                                                            style.bg(
                                                                                bg_control_hover(),
                                                                            )
                                                                        })
                                                                        .active(|style| {
                                                                            style.opacity(0.8)
                                                                        })
                                                                        .cursor(Cursor::Pointer)
                                                                        .on_select(move || {
                                                                            adopted.set(true);
                                                                            deselect_active_profile(
                                                                                profiles,
                                                                                &clear_controls,
                                                                            );
                                                                            dropdown_open
                                                                                .set(false);
                                                                        })
                                                                        .child(glyph_px(
                                                                            11.0,
                                                                            || Glyph::Close,
                                                                            || color(fg_muted()),
                                                                        ))
                                                                        .child(
                                                                            text(tr(
                                                                                "取消选择当前配置",
                                                                                "Deselect current setup",
                                                                            ))
                                                                            .flex_1()
                                                                            .min_w(px(0.0))
                                                                            .number_of_lines(1)
                                                                            .text_overflow(
                                                                                EllipsizeMode::Tail,
                                                                            )
                                                                            .text_size(px(11.5))
                                                                            .font_weight(
                                                                                FontWeight::MEDIUM,
                                                                            )
                                                                            .text_color(
                                                                                fg_secondary(),
                                                                            ),
                                                                        ),
                                                                )
                                                                .child(
                                                                    view()
                                                                        .w_full()
                                                                        .h(px(1.0))
                                                                        .my_0p5()
                                                                        .bg(border_subtle()),
                                                                )
                                                        }
                                                    },
                                                ))
                                                .child(each(
                                                saved,
                                                |item| {
                                                    format!(
                                                        "{}\t{}\t{}\t{}\t{}",
                                                        item.name,
                                                        item.filter,
                                                        item.bypass_hosts,
                                                        item.rules,
                                                        item.upstream_proxy
                                                    )
                                                },
                                                move |item, _| {
                                                    let apply = item;
                                                    let remove = item;
                                                    let mark = item;
                                                    let display_name = item;
                                                    let display_rules = item;
                                                    let preset_controls = preset_controls.clone();
                                                    let filter_apply = filter;
                                                    let bypass_apply = bypass_text;
                                                    let rules_apply = rules_text;
                                                    let upstream_apply = upstream_text;
                                                    let rules_on_apply = rules_on;
                                                    let breaks_on_apply = breaks_on;
                                                    let tunnels_apply = show_tunnels;
                                                    let is_active = move || {
                                                        let typed = rule_name.get();
                                                        let typed = typed.trim();
                                                        if !typed.is_empty() {
                                                            return mark.with(|preset| {
                                                                preset
                                                                    .name
                                                                    .eq_ignore_ascii_case(typed)
                                                            });
                                                        }
                                                        let f = filter.get();
                                                        let b = bypass_text.get();
                                                        let r = rules_text.get();
                                                        let u = upstream_text.get();
                                                        if f.trim().is_empty()
                                                            && b.trim().is_empty()
                                                            && r.trim().is_empty()
                                                            && u.trim().is_empty()
                                                        {
                                                            return false;
                                                        }
                                                        mark.with(|preset| {
                                                            preset.filter.trim() == f.trim()
                                                                && preset.bypass_hosts.trim()
                                                                    == b.trim()
                                                                && preset.rules.trim() == r.trim()
                                                                && preset.upstream_proxy.trim()
                                                                    == u.trim()
                                                        })
                                                    };
                                                    view()
                                                        .h(px(30.0))
                                                        .gap_1()
                                                        .flex_row()
                                                        .items_center()
                                                        .child(
                                                            menu_item()
                                                                .flex_1()
                                                                .min_w(px(0.0))
                                                                .h_full()
                                                                .px_2()
                                                                .rounded(px(6.0))
                                                                .overflow_hidden()
                                                                .flex_row()
                                                                .items_center()
                                                                .gap_2()
                                                                .border_1()
                                                                .border_color(move || {
                                                                    if is_active() {
                                                                        accent_blue_border()
                                                                    } else {
                                                                        AdaptiveColor::TRANSPARENT
                                                                    }
                                                                })
                                                                .bg(move || {
                                                                    if is_active() {
                                                                        accent_blue_soft()
                                                                    } else {
                                                                        AdaptiveColor::TRANSPARENT
                                                                    }
                                                                })
                                                                .hover(|style| {
                                                                    style.bg(move || {
                                                                        if is_active() {
                                                                            accent_blue_soft_hover()
                                                                        } else {
                                                                            bg_control_hover()
                                                                        }
                                                                    })
                                                                })
                                                                .active(|style| style.opacity(0.8))
                                                                .cursor(Cursor::Pointer)
                                                                .on_select(move || {
                                                                    if is_active() {
                                                                        adopted.set(true);
                                                                        deselect_active_profile(
                                                                            profiles,
                                                                            &preset_controls,
                                                                        );
                                                                        dropdown_open.set(false);
                                                                        return;
                                                                    }
                                                                    let preset = apply.with(
                                                                        |preset| preset.clone(),
                                                                    );
                                                                    record_profile_history(
                                                                        &rule_name.get(),
                                                                        &filter_apply.get(),
                                                                        &bypass_apply.get(),
                                                                        &rules_apply.get(),
                                                                        &upstream_apply.get(),
                                                                        rules_on_apply.get(),
                                                                        breaks_on_apply.get(),
                                                                        tunnels_apply.get(),
                                                                        history,
                                                                    );
                                                                    rule_name
                                                                        .set(preset.name.clone());
                                                                    clear_rule_draft(
                                                                        draft_host,
                                                                        draft_path,
                                                                        draft_op,
                                                                        draft_from,
                                                                        draft_to,
                                                                        draft_modes,
                                                                        draft_kind,
                                                                    );
                                                                    apply_saved_profile(
                                                                        &preset,
                                                                        filter_apply,
                                                                        bypass_apply,
                                                                        rules_apply,
                                                                        upstream_apply,
                                                                        rules_on_apply,
                                                                        breaks_on_apply,
                                                                        tunnels_apply,
                                                                        &preset_controls,
                                                                    );
                                                                    record_profile_history(
                                                                        &preset.name,
                                                                        &preset.filter,
                                                                        &preset.bypass_hosts,
                                                                        &preset.rules,
                                                                        &preset.upstream_proxy,
                                                                        preset.rules_enabled,
                                                                        preset.breaks_enabled,
                                                                        preset.show_tunnel_failures,
                                                                        history,
                                                                    );
                                                                })
                                                                .child(
                                                                    view()
                                                                        .w(px(6.0))
                                                                        .h(px(6.0))
                                                                        .rounded_full()
                                                                        .flex_shrink_0()
                                                                        .bg(move || {
                                                                            if is_active() {
                                                                                accent_blue()
                                                                            } else {
                                                                                fg_muted()
                                                                            }
                                                                        }),
                                                                )
                                                                .child(
                                                                    view()
                                                                        .h(px(18.0))
                                                                        .max_w(px(124.0))
                                                                        .px_1p5()
                                                                        .rounded(px(4.0))
                                                                        .flex_shrink_0()
                                                                        .overflow_hidden()
                                                                        .flex_row()
                                                                        .items_center()
                                                                        .bg(accent_blue_soft())
                                                                        .child(
                                                                            text(move || {
                                                                                display_name.with(
                                                                                    |preset| {
                                                                                        preset
                                                                                            .name
                                                                                            .clone()
                                                                                    },
                                                                                )
                                                                            })
                                                                            .max_w(px(112.0))
                                                                            .min_w(px(0.0))
                                                                            .number_of_lines(1)
                                                                            .text_overflow(
                                                                                EllipsizeMode::Tail,
                                                                            )
                                                                            .text_size(px(11.0))
                                                                            .font_weight(
                                                                                FontWeight::MEDIUM,
                                                                            )
                                                                            .text_color(
                                                                                accent_blue_text(),
                                                                            ),
                                                                        ),
                                                                )
                                                                .child(
                                                                    control_text(move || {
                                                                        display_rules
                                                                            .with(profile_preview)
                                                                    })
                                                                    .flex_1()
                                                                    .min_w(px(0.0))
                                                                    .number_of_lines(1)
                                                                    .text_overflow(
                                                                        EllipsizeMode::Tail,
                                                                    )
                                                                    .text_color(fg_secondary()),
                                                                )
                                                                .child(when(is_active, || {
                                                                    text(tr(
                                                                        "点击取消",
                                                                        "Click to clear",
                                                                    ))
                                                                    .flex_shrink_0()
                                                                    .text_size(px(10.5))
                                                                    .font_weight(FontWeight::MEDIUM)
                                                                    .text_color(accent_blue_text())
                                                                })),
                                                        )
                                                        .child(
                                                            view()
                                                                .w(px(26.0))
                                                                .h(px(26.0))
                                                                .flex_shrink_0()
                                                                .flex_row()
                                                                .items_center()
                                                                .justify_center()
                                                                .rounded(px(6.0))
                                                                .cursor(Cursor::Pointer)
                                                                .hover(|style| {
                                                                    style.bg(bg_control_hover())
                                                                })
                                                                .active(|style| style.opacity(0.75))
                                                                .label("Delete setup")
                                                                .tooltip(label_tip(
                                                                    "删除配置",
                                                                    "Delete setup",
                                                                ))
                                                                .on_press(move || {
                                                                    let target = remove.with(
                                                                        |preset| preset.clone(),
                                                                    );
                                                                    adopted.set(true);
                                                                    drop_saved_profile(
                                                                        &target, rule_name, saved,
                                                                        history,
                                                                    );
                                                                    if saved.with(Vec::is_empty) {
                                                                        dropdown_open.set(false);
                                                                    }
                                                                })
                                                                .child(glyph_px(
                                                                    12.0,
                                                                    || Glyph::Trash,
                                                                    || color(fg_muted()),
                                                                )),
                                                        )
                                                },
                                            )),
                                        ),
                                ),
                        ),
                )
                .child(
                    menu()
                        .open(move || Some(history_open.get() && !history.with(Vec::is_empty)))
                        .on_open_change(move |event: &nsh_base::MenuEvent| {
                            if let nsh_base::MenuEvent::OpenChange(next, _) = event {
                                history_open.set(*next && !history.with(Vec::is_empty));
                                if *next {
                                    dropdown_open.set(false);
                                }
                            }
                        })
                        .placement(Placement::Top)
                        .align(Align::Start)
                        .offset(px(6.0))
                        .margin(px(8.0))
                        .trigger(
                            view()
                                .h(px(30.0))
                                .px_2()
                                .flex_shrink_0()
                                .rounded(px(7.0))
                                .border_1()
                                .border_color(move || {
                                    if history_open.get() {
                                        accent_blue_border()
                                    } else {
                                        border_control()
                                    }
                                })
                                .bg(move || {
                                    if history_open.get() {
                                        accent_blue_soft()
                                    } else {
                                        bg_control()
                                    }
                                })
                                .hover(|style| {
                                    style.bg(move || {
                                        if history.with(Vec::is_empty) {
                                            bg_control()
                                        } else if history_open.get() {
                                            accent_blue_soft_hover()
                                        } else {
                                            bg_control_hover()
                                        }
                                    })
                                })
                                .active(|style| {
                                    style.opacity(move || {
                                        if history.with(Vec::is_empty) {
                                            1.0_f32
                                        } else {
                                            0.8_f32
                                        }
                                    })
                                })
                                .flex_row()
                                .items_center()
                                .gap_1()
                                .cursor(Cursor::Pointer)
                                .disabled(move || history.with(Vec::is_empty))
                                .disabled_style(|style| style.cursor(Cursor::Default))
                                .tooltip(label_tip(
                                    "最近 10 个配置历史版本，点击可恢复",
                                    "Last 10 setup versions, click to restore",
                                ))
                                .child(glyph_px(
                                    12.5,
                                    || Glyph::History,
                                    move || {
                                        color(if history_open.get() {
                                            accent_blue_text()
                                        } else if history.with(Vec::is_empty) {
                                            fg_muted()
                                        } else {
                                            fg_secondary()
                                        })
                                    },
                                ))
                                .child(
                                    text(tr("历史", "History"))
                                        .text_size(px(11.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(move || {
                                            if history_open.get() {
                                                accent_blue_text()
                                            } else if history.with(Vec::is_empty) {
                                                fg_muted()
                                            } else {
                                                fg_secondary()
                                            }
                                        }),
                                )
                                .child(when(
                                    move || !history.with(Vec::is_empty),
                                    move || {
                                        view()
                                            .h(px(16.0))
                                            .px_1()
                                            .rounded(px(4.0))
                                            .flex_shrink_0()
                                            .flex_row()
                                            .items_center()
                                            .bg(bg_card())
                                            .child(
                                                mono(move || history.with(Vec::len).to_string())
                                                    .tabular_nums()
                                                    .text_size(px(10.5))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(fg_muted()),
                                            )
                                    },
                                )),
                        )
                        .child(
                            view()
                                .w(px(380.0))
                                .max_w(move || px((window().width.0 - 24.0).clamp(220.0, 380.0)))
                                .max_h(move || px((window().height.0 - 96.0).clamp(140.0, 240.0)))
                                .overflow_hidden()
                                .rounded(px(10.0))
                                .border_1()
                                .border_color(border_strong())
                                .bg(bg_card())
                                .shadow_md()
                                .child(
                                    scroll()
                                        .w_full()
                                        .max_h(move || {
                                            px((window().height.0 - 98.0).clamp(138.0, 238.0))
                                        })
                                        .child(
                                            view().w_full().p_1().gap_0p5().child(each(
                                                history,
                                                |item| {
                                                    format!(
                                                        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                                                        item.name,
                                                        item.filter,
                                                        item.bypass_hosts,
                                                        item.rules,
                                                        item.upstream_proxy,
                                                        item.rules_enabled,
                                                        item.breaks_enabled,
                                                        item.show_tunnel_failures
                                                    )
                                                },
                                                move |item, index| {
                                                    let apply = item;
                                                    let mark = item;
                                                    let display_name = item;
                                                    let display_rules = item;
                                                    let history_controls =
                                                        history_controls.clone();
                                                    let filter_apply = filter;
                                                    let bypass_apply = bypass_text;
                                                    let rules_apply = rules_text;
                                                    let upstream_apply = upstream_text;
                                                    let rules_on_apply = rules_on;
                                                    let breaks_on_apply = breaks_on;
                                                    let tunnels_apply = show_tunnels;
                                                    let is_current = move || {
                                                        mark.with(|preset| {
                                                            preset.filter.trim()
                                                                == filter.get().trim()
                                                                && preset.bypass_hosts.trim()
                                                                    == bypass_text.get().trim()
                                                                && preset.rules.trim()
                                                                    == rules_text.get().trim()
                                                                && preset.upstream_proxy.trim()
                                                                    == upstream_text.get().trim()
                                                                && preset.rules_enabled
                                                                    == rules_on.get()
                                                                && preset.breaks_enabled
                                                                    == breaks_on.get()
                                                                && preset.show_tunnel_failures
                                                                    == show_tunnels.get()
                                                        })
                                                    };
                                                    menu_item()
                                                        .w_full()
                                                        .h(px(30.0))
                                                        .px_2()
                                                        .rounded(px(6.0))
                                                        .overflow_hidden()
                                                        .flex_row()
                                                        .items_center()
                                                        .gap_1p5()
                                                        .border_1()
                                                        .border_color(move || {
                                                            if is_current() {
                                                                accent_blue_border()
                                                            } else {
                                                                AdaptiveColor::TRANSPARENT
                                                            }
                                                        })
                                                        .bg(move || {
                                                            if is_current() {
                                                                accent_blue_soft()
                                                            } else {
                                                                AdaptiveColor::TRANSPARENT
                                                            }
                                                        })
                                                        .hover(|style| {
                                                            style.bg(move || {
                                                                if is_current() {
                                                                    accent_blue_soft_hover()
                                                                } else {
                                                                    bg_control_hover()
                                                                }
                                                            })
                                                        })
                                                        .active(|style| style.opacity(0.8))
                                                        .cursor(Cursor::Pointer)
                                                        .on_select(move || {
                                                            let preset = apply
                                                                .with(|preset| preset.clone());
                                                            record_profile_history(
                                                                &rule_name.get(),
                                                                &filter_apply.get(),
                                                                &bypass_apply.get(),
                                                                &rules_apply.get(),
                                                                &upstream_apply.get(),
                                                                rules_on_apply.get(),
                                                                breaks_on_apply.get(),
                                                                tunnels_apply.get(),
                                                                history,
                                                            );
                                                            rule_name.set(preset.name.clone());
                                                            clear_rule_draft(
                                                                draft_host,
                                                                draft_path,
                                                                draft_op,
                                                                draft_from,
                                                                draft_to,
                                                                draft_modes,
                                                                draft_kind,
                                                            );
                                                            apply_saved_profile(
                                                                &preset,
                                                                filter_apply,
                                                                bypass_apply,
                                                                rules_apply,
                                                                upstream_apply,
                                                                rules_on_apply,
                                                                breaks_on_apply,
                                                                tunnels_apply,
                                                                &history_controls,
                                                            );
                                                            set_status(
                                                                &history_controls,
                                                                StatusMsg::ProfileRestored(
                                                                    preset.name.clone(),
                                                                ),
                                                            );
                                                        })
                                                        .child(
                                                            mono(move || {
                                                                format!("#{}", index.get() + 1)
                                                            })
                                                            .tabular_nums()
                                                            .text_size(px(10.5))
                                                            .text_color(fg_muted()),
                                                        )
                                                        .child(
                                                            view()
                                                                .h(px(18.0))
                                                                .max_w(px(120.0))
                                                                .px_1p5()
                                                                .rounded(px(4.0))
                                                                .flex_shrink_0()
                                                                .overflow_hidden()
                                                                .flex_row()
                                                                .items_center()
                                                                .bg(accent_blue_soft())
                                                                .child(
                                                                    text(move || {
                                                                        display_name.with(
                                                                            |preset| {
                                                                                preset.name.clone()
                                                                            },
                                                                        )
                                                                    })
                                                                    .max_w(px(108.0))
                                                                    .min_w(px(0.0))
                                                                    .number_of_lines(1)
                                                                    .text_overflow(
                                                                        EllipsizeMode::Tail,
                                                                    )
                                                                    .text_size(px(11.0))
                                                                    .font_weight(FontWeight::MEDIUM)
                                                                    .text_color(accent_blue_text()),
                                                                ),
                                                        )
                                                        .child(
                                                            control_text(move || {
                                                                display_rules.with(profile_preview)
                                                            })
                                                            .flex_1()
                                                            .min_w(px(0.0))
                                                            .number_of_lines(1)
                                                            .text_overflow(EllipsizeMode::Tail)
                                                            .text_color(fg_secondary()),
                                                        )
                                                },
                                            )),
                                        ),
                                ),
                        ),
                )
                .child(view().flex_1().min_w(px(0.0)).child(side_input(
                    rule_name,
                    tr("配置名称 (可选，同名覆盖)", "Setup name (optional)"),
                    30.0,
                    false,
                    move |text| rule_name.set(text.to_string()),
                )))
                .child(
                    view()
                        .h(px(30.0))
                        .px_2p5()
                        .flex_shrink_0()
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(border_control())
                        .bg(bg_control())
                        .hover(|style| style.bg(bg_control_hover()))
                        .active(|style| style.opacity(0.8))
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .cursor(Cursor::Pointer)
                        .tooltip(label_tip(
                            "导出配置 TOML 到剪贴板",
                            "Copy setups as TOML to clipboard",
                        ))
                        .on_press(move || {
                            dropdown_open.set(false);
                            let kind_now = draft_kind.get();
                            let rules_now = draft_rules_with_op(
                                &rules_text.get(),
                                &draft_host.get(),
                                &draft_path.get(),
                                &draft_op.get(),
                                &draft_from.get(),
                                &draft_to.get(),
                                draft_modes.mode_for(kind_now),
                                kind_now,
                            );
                            let profiles = profiles_for_export(
                                &saved.get(),
                                &rule_name.get(),
                                &filter.get(),
                                &bypass_text.get(),
                                &rules_now,
                                &upstream_text.get(),
                                rules_on.get(),
                                breaks_on.get(),
                                show_tunnels.get(),
                            );
                            export_profiles_clipboard(&export_controls, &profiles);
                        })
                        .child(
                            text(tr("导出", "Export"))
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg_secondary()),
                        ),
                )
                .child(
                    view()
                        .h(px(30.0))
                        .px_2p5()
                        .flex_shrink_0()
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(move || {
                            if import_open.get() {
                                accent_blue_border()
                            } else {
                                border_control()
                            }
                        })
                        .bg(move || {
                            if import_open.get() {
                                accent_blue_soft()
                            } else {
                                bg_control()
                            }
                        })
                        .hover(|style| {
                            style.bg(move || {
                                if import_open.get() {
                                    accent_blue_soft_hover()
                                } else {
                                    bg_control_hover()
                                }
                            })
                        })
                        .active(|style| style.opacity(0.8))
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .cursor(Cursor::Pointer)
                        .tooltip(label_tip(
                            "粘贴 TOML 配置文本导入（或拖拽 .toml 到窗口）",
                            "Paste TOML setup text to import (or drop .toml)",
                        ))
                        .on_press(move || {
                            dropdown_open.set(false);
                            if !import_open.get() {
                                import_open.set(true);
                            } else if import_text.get().trim().is_empty() {
                                import_open.set(false);
                            } else if import_profiles_text(
                                &import_controls,
                                &import_text.get(),
                                profiles,
                            ) {
                                clear_rule_draft(
                                    draft_host,
                                    draft_path,
                                    draft_op,
                                    draft_from,
                                    draft_to,
                                    draft_modes,
                                    draft_kind,
                                );
                                import_text.set(String::new());
                                import_open.set(false);
                            }
                        })
                        .child(
                            text(tr("导入", "Import"))
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || {
                                    if import_open.get() {
                                        accent_blue_text()
                                    } else {
                                        fg_secondary()
                                    }
                                }),
                        ),
                )
                .child(
                    view()
                        .h(px(30.0))
                        .px_3()
                        .flex_shrink_0()
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(border_control())
                        .bg(bg_control())
                        .hover(|style| style.bg(bg_control_hover()))
                        .active(|style| style.opacity(0.8))
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .cursor(Cursor::Pointer)
                        .on_press(move || {
                            dropdown_open.set(false);
                            history_open.set(false);
                            adopted.set(true);
                            record_profile_history(
                                &rule_name.get(),
                                &filter.get(),
                                &bypass_text.get(),
                                &rules_text.get(),
                                &upstream_text.get(),
                                rules_on.get(),
                                breaks_on.get(),
                                show_tunnels.get(),
                                history,
                            );
                            blank_working_setup(
                                rule_name,
                                filter,
                                bypass_text,
                                rules_text,
                                upstream_text,
                                rules_on,
                                breaks_on,
                                show_tunnels,
                                draft_host,
                                draft_path,
                                draft_op,
                                draft_from,
                                draft_to,
                                draft_modes,
                                draft_kind,
                                &blank_controls,
                            );
                        })
                        .child(
                            text(tr("新建配置", "New setup"))
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg_secondary()),
                        ),
                )
                .child({
                    let save_ready: Rc<dyn Fn() -> bool> = Rc::new(move || {
                        let kind_now = draft_kind.get();
                        profile_save_ready(
                            &saved.get(),
                            &rule_name.get(),
                            &filter.get(),
                            &bypass_text.get(),
                            &rules_text.get(),
                            &upstream_text.get(),
                            rules_on.get(),
                            breaks_on.get(),
                            show_tunnels.get(),
                            &draft_host.get(),
                            &draft_path.get(),
                            &draft_op.get(),
                            &draft_from.get(),
                            &draft_to.get(),
                            draft_modes.mode_for(kind_now),
                            kind_now,
                        )
                    });
                    let ready_bg = save_ready.clone();
                    let ready_border = save_ready.clone();
                    let ready_hover = save_ready.clone();
                    let ready_active = save_ready.clone();
                    let ready_off = save_ready.clone();
                    let ready_press = save_ready.clone();
                    let ready_text = save_ready;
                    view()
                        .h(px(30.0))
                        .px_3p5()
                        .flex_shrink_0()
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(move || {
                            if ready_border() {
                                accent_blue()
                            } else {
                                border_control()
                            }
                        })
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .bg(move || {
                            if ready_bg() {
                                accent_blue()
                            } else {
                                bg_control()
                            }
                        })
                        .hover(|style| {
                            style.bg(move || {
                                if ready_hover() {
                                    accent_blue_hover()
                                } else {
                                    bg_control()
                                }
                            })
                        })
                        .active(|style| {
                            style.opacity(move || {
                                if ready_active() { 0.85_f32 } else { 1.0_f32 }
                            })
                        })
                        .cursor(Cursor::Pointer)
                        .disabled(move || !ready_off())
                        .disabled_style(|style| style.cursor(Cursor::Default))
                        .on_press(move || {
                            if !ready_press() {
                                return;
                            }
                            dropdown_open.set(false);
                            // A row the user actually filled is part of this
                            // setup. An untouched row must not become `*`.
                            let host = draft_host.get();
                            let path = draft_path.get();
                            let op = draft_op.get();
                            let from = draft_from.get();
                            let to = draft_to.get();
                            let kind_now = draft_kind.get();
                            let current = rules_text.get();
                            let rules_now = draft_rules_with_op(
                                &current,
                                &host,
                                &path,
                                &op,
                                &from,
                                &to,
                                draft_modes.mode_for(kind_now),
                                kind_now,
                            );
                            if rules_now != current {
                                rules_text.set(rules_now.clone());
                                save_controls.hub.set_rules(&rules_now);
                                let persisted = rules_now.clone();
                                let _ = save_config(|config| config.rules = persisted);
                                draft_host.set(String::new());
                                draft_path.set(String::new());
                                draft_op.set(String::new());
                                draft_from.set(String::new());
                                draft_to.set(String::new());
                            }
                            let saved_name = resolve_profile_name(
                                &rule_name.get(),
                                filter.get().trim(),
                                rules_now.trim(),
                            );
                            rule_name.set(saved_name.clone());
                            store_saved_profile(
                                &saved_name,
                                &filter.get(),
                                &bypass_text.get(),
                                &rules_now,
                                &upstream_text.get(),
                                rules_on.get(),
                                breaks_on.get(),
                                show_tunnels.get(),
                                saved,
                                history,
                            );
                            set_status(&save_controls, StatusMsg::ProfileSaved(saved_name));
                        })
                        .child(
                            text(tr("保存", "Save"))
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || {
                                    if ready_text() {
                                        AdaptiveColor::rgb(0xffffff, 0xffffff)
                                    } else {
                                        fg_muted()
                                    }
                                }),
                        )
                }),
        )
}

#[derive(Clone, Copy)]
struct QuickRuleModeSignals {
    /// Kinds `0` (Replace), `6` (Header), `11` (Json): `false` = Req, `true` = Res.
    req_res: Signal<bool>,
    /// Kind `2` (Local): `false` = Local, `true` = Fallback.
    local_fallback: Signal<bool>,
    /// Kind `10` (Cert): `false` = MITM, `true` = mTLS.
    mitm_mtls: Signal<bool>,
    /// Kind `13` (Block): `false` = Status, `true` = Drop.
    status_drop: Signal<bool>,
}

impl QuickRuleModeSignals {
    fn new() -> Self {
        Self {
            req_res: signal(true),
            local_fallback: signal(false),
            mitm_mtls: signal(false),
            status_drop: signal(false),
        }
    }

    fn reset(self) {
        self.req_res.set(true);
        self.local_fallback.set(false);
        self.mitm_mtls.set(false);
        self.status_drop.set(false);
    }

    fn signal_for(self, kind: u8) -> Signal<bool> {
        match kind % 14 {
            2 => self.local_fallback,
            10 => self.mitm_mtls,
            13 => self.status_drop,
            _ => self.req_res,
        }
    }

    fn mode_for(self, kind: u8) -> bool {
        self.signal_for(kind).get()
    }
}

/// Filters `from` and `to` down to the inputs actually rendered for `(kind, mode)`,
/// so hidden leftover text from another rule tab cannot mutate `ready_to_add` or `draft_rules`.
fn effective_quick_inputs<'a>(
    kind: u8,
    mode: bool,
    from: &'a str,
    to: &'a str,
) -> (&'a str, &'a str) {
    let k = kind % 14;
    let from_used = !matches!(k, 4 | 5 | 8 | 12) && !(k == 13 && mode);
    let to_used = matches!(k, 0 | 2 | 6 | 7 | 9 | 10 | 11);
    (
        if from_used { from } else { "" },
        if to_used { to } else { "" },
    )
}

fn effective_quick_op(kind: u8, op: &str) -> &str {
    if quick_kind(kind).supports_graphql_op() {
        op
    } else {
        ""
    }
}

fn clear_rule_draft(
    host: Signal<String>,
    path: Signal<String>,
    op: Signal<String>,
    from: Signal<String>,
    to: Signal<String>,
    modes: QuickRuleModeSignals,
    kind: Signal<u8>,
) {
    host.set(String::new());
    path.set(String::new());
    op.set(String::new());
    from.set(String::new());
    to.set(String::new());
    modes.reset();
    kind.set(0);
}

fn clear_working_setup_config(config: &mut config::Config) {
    config.active_profile.clear();
    config.filter.clear();
    config.bypass_hosts.clear();
    config.rules.clear();
    config.upstream_proxy.clear();
    config.rules_enabled = true;
    config.breaks_enabled = true;
    config.show_tunnel_failures = false;
}

fn deselect_active_profile(profiles: ProfileSignals, controls: &Controls) {
    record_profile_history(
        &profiles.name.get(),
        &profiles.filter.get(),
        &profiles.bypass.get(),
        &profiles.rules.get(),
        &profiles.upstream.get(),
        profiles.rules_on.get(),
        profiles.breaks_on.get(),
        profiles.show_tunnels.get(),
        profiles.history,
    );
    profiles
        .deselect_rev
        .update(|rev| *rev = rev.wrapping_add(1));
    profiles.name.set(String::new());
    profiles.filter.set(String::new());
    profiles.bypass.set(String::new());
    profiles.rules.set(String::new());
    profiles.upstream.set(String::new());
    profiles.rules_on.set(true);
    profiles.breaks_on.set(true);
    profiles.show_tunnels.set(false);
    controls.hub.set_bypass("");
    controls.hub.set_rules("");
    controls.hub.set_upstream("");
    controls.hub.set_rules_enabled(true);
    controls.hub.set_breaks_enabled(true);
    controls.hub.set_show_tunnel_failures(false);
    let _ = save_config(clear_working_setup_config);
}

#[allow(clippy::too_many_arguments)]
fn blank_working_setup(
    name: Signal<String>,
    filter: Signal<String>,
    bypass: Signal<String>,
    rules: Signal<String>,
    upstream: Signal<String>,
    rules_on: Signal<bool>,
    breaks_on: Signal<bool>,
    tunnels: Signal<bool>,
    draft_host: Signal<String>,
    draft_path: Signal<String>,
    draft_op: Signal<String>,
    draft_from: Signal<String>,
    draft_to: Signal<String>,
    draft_modes: QuickRuleModeSignals,
    draft_kind: Signal<u8>,
    controls: &Controls,
) {
    name.set(String::new());
    filter.set(String::new());
    bypass.set(String::new());
    rules.set(String::new());
    upstream.set(String::new());
    clear_rule_draft(
        draft_host,
        draft_path,
        draft_op,
        draft_from,
        draft_to,
        draft_modes,
        draft_kind,
    );
    rules_on.set(true);
    breaks_on.set(true);
    tunnels.set(false);
    controls.hub.set_bypass("");
    controls.hub.set_rules("");
    controls.hub.set_upstream("");
    controls.hub.set_rules_enabled(true);
    controls.hub.set_breaks_enabled(true);
    controls.hub.set_show_tunnel_failures(false);
    let _ = save_config(clear_working_setup_config);
}

#[cfg(test)]
const SAVED_FILTER_CAP: usize = 16;

#[cfg(test)]
fn split_saved_filter(entry: &str) -> (&str, &str) {
    match entry.split_once('\t') {
        Some((name, expr)) => (name.trim(), expr.trim()),
        None => ("", entry.trim()),
    }
}

#[cfg(test)]
fn format_saved_filter(name: &str, expr: &str) -> String {
    let name = name.trim().replace('\t', " ");
    let name = name.trim();
    let expr = expr.trim();
    if name.is_empty() {
        expr.to_string()
    } else {
        format!("{name}\t{expr}")
    }
}

#[cfg(test)]
fn push_saved_filter_named(saved: &[String], name: &str, current: &str) -> Vec<String> {
    let expr = current.trim();
    if expr.is_empty() {
        return saved.to_vec();
    }
    let clean_name = name.trim().replace('\t', " ");
    let clean_name = clean_name.trim();
    let mut next: Vec<String> = saved
        .iter()
        .filter(|item| {
            let (existing_name, existing_expr) = split_saved_filter(item);
            existing_expr != expr
                && (clean_name.is_empty() || !existing_name.eq_ignore_ascii_case(clean_name))
        })
        .cloned()
        .collect();
    next.insert(0, format_saved_filter(clean_name, expr));
    next.truncate(SAVED_FILTER_CAP);
    next
}

#[cfg(test)]
fn push_saved_filter(saved: &[String], current: &str) -> Vec<String> {
    push_saved_filter_named(saved, "", current)
}

fn rule_preset_preview(rules: &str) -> String {
    let first = rules
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or("");
    let count = crate::rules::parse_rules(rules).len();
    if count > 1 {
        format!("({count}) {first}")
    } else {
        first.to_string()
    }
}

#[cfg(test)]
fn push_saved_rule(
    saved: &[config::SavedRuleSet],
    name: &str,
    rules: &str,
) -> Vec<config::SavedRuleSet> {
    let rules = rules.trim();
    if rules.is_empty() {
        return saved.to_vec();
    }
    let name = name.trim();
    let resolved_name = if name.is_empty() {
        rules
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .unwrap_or("规则")
            .chars()
            .take(28)
            .collect::<String>()
    } else {
        name.to_string()
    };
    let mut next: Vec<config::SavedRuleSet> = saved
        .iter()
        .filter(|item| {
            item.rules.trim() != rules && !item.name.eq_ignore_ascii_case(&resolved_name)
        })
        .cloned()
        .collect();
    next.insert(
        0,
        config::SavedRuleSet {
            name: resolved_name,
            rules: rules.to_string(),
        },
    );
    next.truncate(SAVED_RULE_CAP);
    next
}

fn profile_preview(profile: &config::SavedProfile) -> String {
    let mut parts = Vec::new();
    let filter = profile.filter.trim();
    if !filter.is_empty() {
        parts.push(filter.to_string());
    }
    let rules = rule_preset_preview(&profile.rules);
    if !rules.is_empty() {
        parts.push(rules);
    }
    let bypass = profile.bypass_hosts.trim();
    if !bypass.is_empty() {
        parts.push(bypass.to_string());
    }
    let upstream = profile.upstream_proxy.trim();
    if !upstream.is_empty() {
        parts.push(upstream.to_string());
    }
    parts.join(" · ")
}

#[allow(clippy::too_many_arguments)]
fn store_saved_profile(
    name: &str,
    filter: &str,
    bypass: &str,
    rules: &str,
    upstream: &str,
    rules_enabled: bool,
    breaks_enabled: bool,
    show_tunnel_failures: bool,
    saved: Signal<Vec<config::SavedProfile>>,
    history: Signal<Vec<config::SavedProfile>>,
) {
    let resolved_name = resolve_profile_name(name, filter.trim(), rules.trim());
    if let Some(old) = saved
        .get()
        .into_iter()
        .find(|item| item.name.eq_ignore_ascii_case(&resolved_name))
    {
        record_profile_history(
            &old.name,
            &old.filter,
            &old.bypass_hosts,
            &old.rules,
            &old.upstream_proxy,
            old.rules_enabled,
            old.breaks_enabled,
            old.show_tunnel_failures,
            history,
        );
    }
    record_profile_history(
        &resolved_name,
        filter,
        bypass,
        rules,
        upstream,
        rules_enabled,
        breaks_enabled,
        show_tunnel_failures,
        history,
    );
    let next = push_saved_profile(
        &saved.get(),
        name,
        filter,
        bypass,
        rules,
        upstream,
        rules_enabled,
        breaks_enabled,
        show_tunnel_failures,
    );
    let saved_entry = next
        .iter()
        .find(|item| item.name.eq_ignore_ascii_case(&resolved_name))
        .cloned();
    let changed = next != saved.get();
    if changed {
        saved.set(next.clone());
    }
    if let Some(entry) = saved_entry {
        let _ = save_config(move |config| {
            config.saved_profiles = next;
            config.active_profile = entry.name;
            config.filter = entry.filter;
            config.bypass_hosts = entry.bypass_hosts;
            config.rules = entry.rules;
            config.upstream_proxy = entry.upstream_proxy;
            config.rules_enabled = entry.rules_enabled;
            config.breaks_enabled = entry.breaks_enabled;
            config.show_tunnel_failures = entry.show_tunnel_failures;
        });
    } else if changed {
        let _ = save_config(|config| config.saved_profiles = next);
    }
}

#[allow(clippy::too_many_arguments)]
fn record_profile_history(
    name: &str,
    filter: &str,
    bypass: &str,
    rules: &str,
    upstream: &str,
    rules_enabled: bool,
    breaks_enabled: bool,
    show_tunnel_failures: bool,
    history: Signal<Vec<config::SavedProfile>>,
) {
    let next = push_profile_history(
        &history.get(),
        name,
        filter,
        bypass,
        rules,
        upstream,
        rules_enabled,
        breaks_enabled,
        show_tunnel_failures,
    );
    if next == history.get() {
        return;
    }
    history.set(next.clone());
    let _ = save_config(|config| config.profile_history = next);
}

#[allow(clippy::too_many_arguments)]
fn push_profile_history(
    history: &[config::SavedProfile],
    name: &str,
    filter: &str,
    bypass: &str,
    rules: &str,
    upstream: &str,
    rules_enabled: bool,
    breaks_enabled: bool,
    show_tunnel_failures: bool,
) -> Vec<config::SavedProfile> {
    let filter = filter.trim();
    let bypass = bypass.trim();
    let rules = rules.trim();
    let upstream = upstream.trim();
    if filter.is_empty() && bypass.is_empty() && rules.is_empty() && upstream.is_empty() {
        return history.to_vec();
    }
    let resolved_name = resolve_profile_name(name, filter, rules);
    let candidate = config::SavedProfile {
        name: resolved_name,
        filter: filter.to_string(),
        bypass_hosts: bypass.to_string(),
        rules: rules.to_string(),
        upstream_proxy: upstream.to_string(),
        rules_enabled,
        breaks_enabled,
        show_tunnel_failures,
    };
    let mut next: Vec<config::SavedProfile> = history
        .iter()
        .filter(|item| !config::same_profile_content(item, &candidate))
        .cloned()
        .collect();
    next.insert(0, candidate);
    next.truncate(config::PROFILE_HISTORY_CAP);
    next
}

fn apply_saved_profile(
    profile: &config::SavedProfile,
    filter: Signal<String>,
    bypass: Signal<String>,
    rules: Signal<String>,
    upstream: Signal<String>,
    rules_on: Signal<bool>,
    breaks_on: Signal<bool>,
    tunnels: Signal<bool>,
    controls: &Controls,
) {
    filter.set(profile.filter.clone());
    bypass.set(profile.bypass_hosts.clone());
    rules.set(profile.rules.clone());
    upstream.set(profile.upstream_proxy.clone());
    rules_on.set(profile.rules_enabled);
    breaks_on.set(profile.breaks_enabled);
    tunnels.set(profile.show_tunnel_failures);
    controls.hub.set_bypass(&profile.bypass_hosts);
    controls.hub.set_rules(&profile.rules);
    controls.hub.set_upstream(&profile.upstream_proxy);
    controls.hub.set_rules_enabled(profile.rules_enabled);
    controls.hub.set_breaks_enabled(profile.breaks_enabled);
    controls
        .hub
        .set_show_tunnel_failures(profile.show_tunnel_failures);
    let profile = profile.clone();
    let _ = save_config(move |config| {
        config.active_profile = profile.name.clone();
        config.filter = profile.filter.clone();
        config.bypass_hosts = profile.bypass_hosts.clone();
        config.rules = profile.rules.clone();
        config.upstream_proxy = profile.upstream_proxy.clone();
        config.rules_enabled = profile.rules_enabled;
        config.breaks_enabled = profile.breaks_enabled;
        config.show_tunnel_failures = profile.show_tunnel_failures;
    });
}

fn drop_saved_profile(
    target: &config::SavedProfile,
    rule_name: Signal<String>,
    saved: Signal<Vec<config::SavedProfile>>,
    history: Signal<Vec<config::SavedProfile>>,
) {
    record_profile_history(
        &target.name,
        &target.filter,
        &target.bypass_hosts,
        &target.rules,
        &target.upstream_proxy,
        target.rules_enabled,
        target.breaks_enabled,
        target.show_tunnel_failures,
        history,
    );
    if rule_name.get().trim().eq_ignore_ascii_case(&target.name) {
        rule_name.set(String::new());
    }
    let next: Vec<config::SavedProfile> = saved
        .get()
        .into_iter()
        .filter(|item| item != target)
        .collect();
    saved.set(next.clone());
    let removed_name = target.name.clone();
    let _ = save_config(move |config| {
        if config
            .active_profile
            .trim()
            .eq_ignore_ascii_case(&removed_name)
        {
            config.active_profile.clear();
        }
        config.saved_profiles = next;
    });
}

/// Rules after 保存 commits a filled add-rule row. An empty row stays out,
/// so it cannot turn into a global `*`.
#[cfg_attr(not(test), allow(dead_code))]
fn draft_rules(
    rules: &str,
    host: &str,
    path: &str,
    from: &str,
    to: &str,
    response: bool,
    kind: u8,
) -> String {
    draft_rules_with_op(rules, host, path, "", from, to, response, kind)
}

fn draft_rules_with_op(
    rules: &str,
    host: &str,
    path: &str,
    op: &str,
    from: &str,
    to: &str,
    response: bool,
    kind: u8,
) -> String {
    let (from, to) = effective_quick_inputs(kind, response, from, to);
    let op = effective_quick_op(kind, op);
    let touched = !host.trim().is_empty()
        || !path.trim().is_empty()
        || !op.trim().is_empty()
        || !from.trim().is_empty()
        || !to.trim().is_empty();
    if !touched {
        return rules.to_string();
    }
    let combined = crate::rules::combine_rule_pattern(host, path);
    if op.trim().is_empty() {
        crate::rules::append_quick_rule(rules, quick_kind(kind), &combined, response, from, to)
            .unwrap_or_else(|| rules.to_string())
    } else {
        crate::rules::append_quick_rule_with_op(
            rules,
            quick_kind(kind),
            &combined,
            response,
            from,
            to,
            op,
        )
        .unwrap_or_else(|| rules.to_string())
    }
}

fn resolve_profile_name(name: &str, filter: &str, rules: &str) -> String {
    let name = name.trim();
    if !name.is_empty() {
        return name.to_string();
    }
    if !filter.is_empty() {
        return filter.chars().take(28).collect();
    }
    let fallback = t(ui_lang(ui_langs()), "配置", "Profile");
    if !rules.is_empty() {
        return rules
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .unwrap_or(fallback)
            .chars()
            .take(28)
            .collect();
    }
    fallback.to_string()
}

/// True when 保存 would store a different snapshot. The open setup is the
/// saved profile with the same name; an unchanged one stays disabled.
fn profile_save_ready(
    saved: &[config::SavedProfile],
    name: &str,
    filter: &str,
    bypass: &str,
    rules: &str,
    upstream: &str,
    rules_enabled: bool,
    breaks_enabled: bool,
    show_tunnel_failures: bool,
    draft_host: &str,
    draft_path: &str,
    draft_op: &str,
    draft_from: &str,
    draft_to: &str,
    draft_response: bool,
    draft_kind: u8,
) -> bool {
    let rules = draft_rules_with_op(
        rules,
        draft_host,
        draft_path,
        draft_op,
        draft_from,
        draft_to,
        draft_response,
        draft_kind,
    );
    let filter = filter.trim();
    let bypass = bypass.trim();
    let rules = rules.trim();
    let upstream = upstream.trim();
    if filter.is_empty() && bypass.is_empty() && rules.is_empty() && upstream.is_empty() {
        return false;
    }
    let resolved = resolve_profile_name(name, filter, rules);
    let Some(preset) = saved
        .iter()
        .find(|item| item.name.eq_ignore_ascii_case(&resolved))
    else {
        return true;
    };
    preset.name != resolved
        || preset.filter.trim() != filter
        || preset.bypass_hosts.trim() != bypass
        || preset.rules.trim() != rules
        || preset.upstream_proxy.trim() != upstream
        || preset.rules_enabled != rules_enabled
        || preset.breaks_enabled != breaks_enabled
        || preset.show_tunnel_failures != show_tunnel_failures
}

fn push_saved_profile(
    saved: &[config::SavedProfile],
    name: &str,
    filter: &str,
    bypass: &str,
    rules: &str,
    upstream: &str,
    rules_enabled: bool,
    breaks_enabled: bool,
    show_tunnel_failures: bool,
) -> Vec<config::SavedProfile> {
    let filter = filter.trim();
    let bypass = bypass.trim();
    let rules = rules.trim();
    let upstream = upstream.trim();
    if filter.is_empty() && bypass.is_empty() && rules.is_empty() && upstream.is_empty() {
        return saved.to_vec();
    }
    let resolved_name = resolve_profile_name(name, filter, rules);
    let mut next: Vec<config::SavedProfile> = saved
        .iter()
        .filter(|item| !item.name.eq_ignore_ascii_case(&resolved_name))
        .cloned()
        .collect();
    next.insert(
        0,
        config::SavedProfile {
            name: resolved_name,
            filter: filter.to_string(),
            bypass_hosts: bypass.to_string(),
            rules: rules.to_string(),
            upstream_proxy: upstream.to_string(),
            rules_enabled,
            breaks_enabled,
            show_tunnel_failures,
        },
    );
    next.truncate(SAVED_RULE_CAP);
    next
}

fn clamp_list_px(width: f32) -> f32 {
    let rounded = if width.is_finite() && width > 0.0 {
        width.round().min(u32::MAX as f32) as u32
    } else {
        0
    };
    config::clamp_list_width(rounded) as f32
}

fn clamp_list_px_for_window(width: f32, window_w: f32) -> f32 {
    let base = clamp_list_px(width);
    if window_w.is_finite() && window_w > 0.0 {
        let max_for_win = (window_w - 340.0)
            .round()
            .clamp(config::LIST_WIDTH_MIN as f32, config::LIST_WIDTH_MAX as f32);
        base.min(max_for_win)
    } else {
        base
    }
}

fn adaptive_filter_panel_width(window_w: f32, list_w: f32) -> f32 {
    if window_w.is_finite() && window_w > 0.0 {
        let list_px = clamp_list_px_for_window(list_w, window_w);
        let max_w = (window_w - 20.0).max(280.0);
        (window_w - list_px * 0.7 - 16.0)
            .round()
            .clamp(680.0, 1360.0)
            .min(max_w)
    } else {
        760.0
    }
}

const FILTER_PANEL_CHROME_CLEARANCE: f32 = 80.0;
const FILTER_PANEL_DOCK_HEIGHT: f32 = 54.0;
const FILTER_PANEL_IMPORT_EXTRA_HEIGHT: f32 = 182.0;
const FILTER_PANEL_RIGHT_FIXED_HEIGHT: f32 = 620.0;

fn adaptive_filter_panel_max_height(window_h: f32) -> f32 {
    if window_h.is_finite() && window_h > 0.0 {
        (window_h - FILTER_PANEL_CHROME_CLEARANCE)
            .round()
            .max(260.0)
    } else {
        672.0
    }
}

fn adaptive_filter_body_max_height(window_h: f32, import_open: bool) -> f32 {
    let import_h = if import_open {
        FILTER_PANEL_IMPORT_EXTRA_HEIGHT
    } else {
        0.0
    };
    (adaptive_filter_panel_max_height(window_h) - FILTER_PANEL_DOCK_HEIGHT - import_h)
        .round()
        .max(if import_open { 24.0 } else { 120.0 })
}

#[cfg(test)]
fn adaptive_rules_editor_height(window_h: f32) -> f32 {
    adaptive_rules_editor_height_for(window_h, false)
}

fn adaptive_rules_editor_height_for(window_h: f32, import_open: bool) -> f32 {
    if window_h.is_finite() && window_h > 0.0 {
        let body_budget = adaptive_filter_body_max_height(window_h, import_open);
        let min_h = if import_open { 56.0 } else { 84.0 };
        let max_h = if import_open { 320.0 } else { 460.0 };
        (body_budget - FILTER_PANEL_RIGHT_FIXED_HEIGHT)
            .round()
            .clamp(min_h, max_h)
    } else {
        105.0
    }
}

fn column_splitter(width: Signal<f32>, controls: Arc<Controls>) -> impl IntoElement {
    let origin = signal(0.0_f32);
    let width_grant = width;
    let width_move = width;
    let width_release = width;
    let width_cancel = width;
    let controls_release = controls.clone();
    let controls_cancel = controls;
    let hot = signal(false);
    let dragging = signal(false);
    view()
        .w(px(6.0))
        .flex_shrink_0()
        .self_stretch()
        .flex_row()
        .justify_center()
        .cursor(Cursor::ColResize)
        .claim_touches(TouchClaim::on_start())
        .keep_touches(true)
        .on_mouse_enter(move |_: &MouseEvent| hot.set(true))
        .on_mouse_leave(move |_: &MouseEvent| hot.set(false))
        .on_responder_grant(move |_event: &ResponderEvent| {
            dragging.set(true);
            origin.set(clamp_list_px_for_window(
                width_grant.get(),
                window().width.0,
            ));
        })
        .on_responder_move(move |event: &ResponderEvent| {
            width_move.set(clamp_list_px_for_window(
                origin.get() + event.gesture.delta.x.0,
                window().width.0,
            ));
        })
        .on_responder_release(move |_event: &ResponderEvent| {
            dragging.set(false);
            store_list_width(
                clamp_list_px_for_window(width_release.get(), window().width.0),
                &controls_release,
            );
        })
        .on_responder_terminate(move |_event: &ResponderEvent| {
            dragging.set(false);
            store_list_width(
                clamp_list_px_for_window(width_cancel.get(), window().width.0),
                &controls_cancel,
            );
        })
        .child(
            view()
                .w(move || {
                    if hot.get() || dragging.get() {
                        px(3.0)
                    } else {
                        px(1.0)
                    }
                })
                .h_full()
                .bg(move || {
                    if hot.get() || dragging.get() {
                        accent_blue()
                    } else {
                        border_strong()
                    }
                }),
        )
}

fn store_list_width(width: f32, controls: &Controls) {
    let stored = clamp_list_px(width).round() as u32;
    if let Err(err) = save_config(|config| config.list_width = stored) {
        set_status(
            controls,
            StatusMsg::PrefUnsaved {
                name: PrefName::List,
                err: err.to_string(),
            },
        );
    }
}

fn merge_secondary_filter(main: &str, secondary: &str) -> String {
    let norm_main = crate::session::normalize_filter_pipes(main);
    let norm_sec = crate::session::normalize_filter_pipes(secondary);
    let mut out: Vec<String> = norm_main
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();
    for tok in norm_sec.split_whitespace().filter(|t| !t.is_empty()) {
        if !out
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(tok))
        {
            out.push(tok.to_string());
        }
    }
    out.join(" ")
}

#[allow(clippy::too_many_arguments)]
fn flow_list(
    rows: Signal<Vec<Arc<RowLine>>>,
    facets: Signal<session::SessionFacets>,
    facets_open: Signal<bool>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
    compose: Compose,
    main_filter: Signal<String>,
    list_filter: Signal<String>,
    effective_filter: Signal<String>,
    rules_text: Signal<String>,
    width: Signal<f32>,
    columns: Signal<ListColumns>,
    sort: Signal<Option<ListSort>>,
    controls: Arc<Controls>,
    shown: Shown,
    baseline: Signal<Option<u64>>,
    filter_focus: ElementRef,
) -> impl IntoElement {
    let row_controls = controls.clone();
    let column_controls = controls;
    let picked = signal(None::<u64>);
    let menu_open = signal(false);
    let menu_at = signal(Point::<Points>::default());
    let facet_tab = signal(0_u8);
    let effective_width = move || clamp_list_px_for_window(width.get(), window().width.0);
    view()
        .w(move || px(effective_width()))
        .flex_shrink_0()
        .overflow_hidden()
        .bg(bg_list())
        .child(
            view()
                .w(move || px(effective_width()))
                .h(px(46.0))
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_1p5()
                .pl(px(LIST_INSET))
                .pr(px(LIST_PAD_END))
                .bg(bg_chrome())
                .border_b_1()
                .border_b_color(border_strong())
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .relative()
                        .flex_row()
                        .items_center()
                        .child(
                            input()
                                .element_ref(&filter_focus)
                                .value(list_filter)
                                .placeholder(tr(
                                    "在当前结果中再次过滤 (主机、路径、状态、body:…)",
                                    "Filter current results (host, path, status, body:…)",
                                ))
                                .placeholder_color(move || color(fg_muted()))
                                .w_full()
                                .h(px(30.0))
                                .pl_2p5()
                                .pr(move || {
                                    if list_filter.get().is_empty() {
                                        px(10.0)
                                    } else {
                                        px(28.0)
                                    }
                                })
                                .rounded(px(7.0))
                                .bg(bg_code())
                                .border_1()
                                .border_color(border_control())
                                .focus(|style| style.border_color(accent_blue()))
                                .text_size(px(12.5))
                                .text_color(fg_primary())
                                .on_change(move |text: &str| list_filter.set(text.to_string())),
                        )
                        .child(when(
                            move || !list_filter.get().is_empty(),
                            move || {
                                view()
                                    .absolute()
                                    .right(px(4.0))
                                    .top(px(4.0))
                                    .w(px(22.0))
                                    .h(px(22.0))
                                    .flex_shrink_0()
                                    .flex_row()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(5.0))
                                    .hover(|style| style.bg(bg_control_hover()))
                                    .active(|style| style.opacity(0.75))
                                    .cursor(Cursor::Pointer)
                                    .label("Clear secondary filter")
                                    .tooltip(label_tip("清空二次过滤", "Clear secondary filter"))
                                    .on_press(move || list_filter.set(String::new()))
                                    .child(
                                        control_text("×")
                                            .text_size(px(14.0))
                                            .text_color(fg_secondary()),
                                    )
                            },
                        )),
                )
                .child(when(
                    move || !list_filter.get().trim().is_empty(),
                    move || {
                        view()
                            .h(px(30.0))
                            .px_2()
                            .flex_shrink_0()
                            .flex_row()
                            .items_center()
                            .justify_center()
                            .rounded(px(CONTROL_RADIUS))
                            .border_1()
                            .border_color(accent_blue_border())
                            .bg(accent_blue_soft())
                            .hover(|style| style.bg(accent_blue_soft_hover()))
                            .active(|style| style.opacity(0.8))
                            .cursor(Cursor::Pointer)
                            .label(tr("并入主过滤", "Merge"))
                            .tooltip(label_tip(
                                "将当前二次筛选并入顶栏主过滤条件并可保存进预设",
                                "Merge secondary filter into main filter",
                            ))
                            .on_press(move || {
                                let merged =
                                    merge_secondary_filter(&main_filter.get(), &list_filter.get());
                                main_filter.set(merged.clone());
                                list_filter.set(String::new());
                                let _ = save_config(|cfg| cfg.filter = merged);
                            })
                            .child(
                                control_text(tr("并入主过滤", "Merge"))
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(accent_blue_text()),
                            )
                    },
                ))
                .child(facet_toggle_button(facets_open, list_filter))
                .child(column_menu(columns, column_controls)),
        )
        .child(when(
            move || facets_open.get(),
            move || facet_drawer(facets, facet_tab, list_filter, effective_width),
        ))
        .child(view().flex_1().min_h(px(0.0)).child(list_menu(
            rows,
            selected,
            paused,
            compose,
            columns,
            sort,
            row_controls,
            list_filter,
            effective_filter,
            shown,
            baseline,
            rules_text,
            picked,
            menu_open,
            menu_at,
        )))
}

fn facet_toggle_button(facets_open: Signal<bool>, filter: Signal<String>) -> impl IntoElement {
    let has_facet_filter = move || {
        let f = filter.get();
        !extract_host_or(&f).is_empty() || !extract_app_or(&f).is_empty()
    };
    view()
        .w(px(30.0))
        .h(px(30.0))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(move || {
            if facets_open.get() || has_facet_filter() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if facets_open.get() || has_facet_filter() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if facets_open.get() || has_facet_filter() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .label("Domains & Apps")
        .tooltip(label_tip("域名与应用导航", "Domains & Apps"))
        .on_press(move || facets_open.set(!facets_open.get()))
        .child(glyph(
            || Glyph::List,
            move || {
                if facets_open.get() || has_facet_filter() {
                    color(accent_blue_text())
                } else {
                    color(fg_secondary())
                }
            },
        ))
}

fn facet_drawer(
    facets: Signal<session::SessionFacets>,
    facet_tab: Signal<u8>,
    filter: Signal<String>,
    effective_width: impl Fn() -> f32 + Copy + 'static,
) -> impl IntoElement {
    let tab_btn = move |idx: u8,
                        zh: &'static str,
                        en: &'static str,
                        count_fn: fn(&session::SessionFacets) -> usize| {
        let langs = ui_langs();
        view()
            .h(px(22.0))
            .px_2()
            .rounded(px(5.0))
            .flex_row()
            .items_center()
            .gap_1()
            .bg(move || {
                if facet_tab.get() == idx {
                    accent_blue_soft()
                } else {
                    AdaptiveColor::TRANSPARENT
                }
            })
            .border_1()
            .border_color(move || {
                if facet_tab.get() == idx {
                    accent_blue_border()
                } else {
                    AdaptiveColor::TRANSPARENT
                }
            })
            .hover(|style| {
                style.bg(move || {
                    if facet_tab.get() == idx {
                        accent_blue_soft_hover()
                    } else {
                        bg_control_hover()
                    }
                })
            })
            .active(|style| style.opacity(0.8))
            .cursor(Cursor::Pointer)
            .on_press(move || facet_tab.set(idx))
            .child(
                control_text(move || -> SharedString {
                    let count = facets.with(count_fn);
                    format!("{} ({count})", t(ui_lang(langs), zh, en)).into()
                })
                .tabular_nums()
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if facet_tab.get() == idx {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
            )
    };
    let items_list = move || -> Vec<session::FacetEntry> {
        if facet_tab.get() == 0 {
            facets.with(|f| f.domains.clone())
        } else {
            facets.with(|f| f.apps.clone())
        }
    };
    view()
        .w(move || px(effective_width()))
        .flex_shrink_0()
        .bg(bg_chrome())
        .border_b_1()
        .border_b_color(border_strong())
        .pl(px(LIST_INSET))
        .py_1p5()
        .gap_1p5()
        .child(
            view()
                .pr(px(LIST_PAD_END))
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_1p5()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(tab_btn(0, "域名", "Domains", |f| f.domains.len()))
                        .child(tab_btn(1, "应用 / 来源", "Apps / Clients", |f| {
                            f.apps.len()
                        })),
                )
                .child(
                    text(move || -> SharedString {
                        if facet_tab.get() == 0 {
                            tr("点击只看 · × 屏蔽", "Click to filter · × to mute")().into()
                        } else {
                            tr("点击按进程/来源筛选", "Click to filter by app/client")().into()
                        }
                    })
                    .text_size(px(11.0))
                    .text_color(fg_muted()),
                ),
        )
        .child(
            when(
                move || items_list().is_empty(),
                move || {
                    view().pr(px(LIST_PAD_END)).py_1p5().child(
                        text(tr(
                            "暂无捕获到的域名或应用",
                            "No captured domains or apps yet",
                        ))
                        .text_size(px(11.5))
                        .text_color(fg_muted()),
                    )
                },
            )
            .otherwise(move || {
                scroll()
                    .w_full()
                    .max_h(move || px((window().height.0 * 0.22).round().clamp(64.0, 132.0)))
                    .child(
                        view()
                            .w_full()
                            .pr(px(LIST_PAD_END))
                            .flex_row()
                            .flex_wrap()
                            .gap_1()
                            .child(each(
                                items_list,
                                |entry| entry.token.clone(),
                                move |entry, _index| {
                                    let is_domain = facet_tab.get() == 0;
                                    let is_active = move || {
                                        let f = filter.get();
                                        let tok = entry.with(|e| e.token.clone());
                                        if is_domain {
                                            host_facet_active(&f, &tok)
                                        } else {
                                            app_facet_active(&f, &tok)
                                        }
                                    };
                                    let is_muted = move || {
                                        is_domain
                                            && mute_facet_active(
                                                &filter.get(),
                                                &entry.with(|e| e.token.clone()),
                                            )
                                    };
                                    view()
                                        .h(px(23.0))
                                        .max_w(move || {
                                            px((effective_width() - LIST_INSET - LIST_PAD_END)
                                                .max(96.0))
                                        })
                                        .rounded(px(5.0))
                                        .border_1()
                                        .border_color(move || {
                                            if is_active() {
                                                accent_blue_border()
                                            } else if is_muted() {
                                                status_err_fg()
                                            } else {
                                                border_control()
                                            }
                                        })
                                        .bg(move || {
                                            if is_active() {
                                                accent_blue_soft()
                                            } else if is_muted() {
                                                status_err_bg()
                                            } else {
                                                bg_control()
                                            }
                                        })
                                        .flex_row()
                                        .items_center()
                                        .overflow_hidden()
                                        .child(
                                            view()
                                                .min_w(px(0.0))
                                                .h_full()
                                                .px_2()
                                                .flex_row()
                                                .items_center()
                                                .gap_1()
                                                .hover(|style| {
                                                    style.bg(move || {
                                                        if is_active() {
                                                            accent_blue_soft_hover()
                                                        } else if is_muted() {
                                                            status_err_bg_hover()
                                                        } else {
                                                            bg_control_hover()
                                                        }
                                                    })
                                                })
                                                .active(|style| style.opacity(0.8))
                                                .cursor(Cursor::Pointer)
                                                .on_press(move || {
                                                    let cur = filter.get();
                                                    let tok = entry.with(|e| e.token.clone());
                                                    let next = if is_domain {
                                                        toggle_host_facet(&cur, &tok)
                                                    } else {
                                                        toggle_app_facet(&cur, &tok)
                                                    };
                                                    filter.set(next);
                                                })
                                                .child(
                                                    mono(move || {
                                                        let label = entry.with(|e| e.label.clone());
                                                        if is_muted() {
                                                            format!("− {label}")
                                                        } else {
                                                            label
                                                        }
                                                    })
                                                    .min_w(px(0.0))
                                                    .number_of_lines(1)
                                                    .text_overflow(EllipsizeMode::Middle)
                                                    .text_size(px(11.0))
                                                    .text_color(move || {
                                                        if is_active() {
                                                            accent_blue_text()
                                                        } else if is_muted() {
                                                            status_err_fg()
                                                        } else {
                                                            fg_primary()
                                                        }
                                                    }),
                                                )
                                                .child(
                                                    mono(move || {
                                                        entry.with(|e| format!("{}", e.count))
                                                    })
                                                    .flex_shrink_0()
                                                    .tabular_nums()
                                                    .text_size(px(11.0))
                                                    .text_color(move || {
                                                        if entry.with(|e| e.error_count > 0) {
                                                            status_err_fg()
                                                        } else {
                                                            fg_muted()
                                                        }
                                                    }),
                                                ),
                                        )
                                        .child(when(
                                            move || is_domain,
                                            move || {
                                                view()
                                                    .w(px(24.0))
                                                    .h_full()
                                                    .flex_shrink_0()
                                                    .flex_row()
                                                    .items_center()
                                                    .justify_center()
                                                    .border_l_1()
                                                    .border_l_color(border_subtle())
                                                    .hover(|style| {
                                                        style.bg(move || {
                                                            if is_muted() {
                                                                status_err_bg_hover()
                                                            } else {
                                                                bg_control_hover()
                                                            }
                                                        })
                                                    })
                                                    .active(|style| style.opacity(0.8))
                                                    .cursor(Cursor::Pointer)
                                                    .label("Mute domain")
                                                    .on_press(move || {
                                                        let tok = entry.with(|e| e.token.clone());
                                                        filter.set(toggle_mute_facet(
                                                            &filter.get(),
                                                            &tok,
                                                        ));
                                                    })
                                                    .child(
                                                        control_text("×")
                                                            .text_size(px(11.5))
                                                            .text_color(move || {
                                                                if is_muted() {
                                                                    status_err_fg()
                                                                } else {
                                                                    fg_muted()
                                                                }
                                                            }),
                                                    )
                                            },
                                        ))
                                },
                            )),
                    )
            }),
        )
}

const JSON_ALIASES: &[&str] = &["type:json", "ct:json"];
const FAIL_ALIASES: &[&str] = &["失败", "fail", "err", "error"];
const PIN_ALIASES: &[&str] = &["pin:"];
const SLOW_ALIASES: &[&str] = &["slow:"];

fn remember_filter(current: &str) {
    let current = current.trim().to_string();
    if saved_config().filter == current {
        return;
    }
    if let Err(err) = save_config(|config| config.filter = current.clone()) {
        // The box still has the text. A status line is enough if the file
        // cannot be written; the next pause tries again.
        let _ = err;
    }
}

fn remember_window_size(width: u32, height: u32) {
    let width = config::clamp_window_width(width);
    let height = config::clamp_window_height(height);
    let current = saved_config();
    if current.window_width == width && current.window_height == height {
        return;
    }
    let _ = save_config(|config| {
        config.window_width = width;
        config.window_height = height;
    });
}

fn filter_chip(
    filter: Signal<String>,
    zh: &'static str,
    en: &'static str,
    token: &'static str,
    aliases: &'static [&'static str],
) -> impl IntoElement {
    view()
        .h(px(28.0))
        .px_2p5()
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .justify_center()
        .gap_1p5()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || {
            if filter_has_token(&filter.get(), aliases) {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if filter_has_token(&filter.get(), aliases) {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if filter_has_token(&filter.get(), aliases) {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .on_press(move || filter.set(toggle_filter_chip(&filter.get(), token, aliases)))
        .child(view().w(px(5.0)).h(px(5.0)).rounded_full().bg(move || {
            if filter_has_token(&filter.get(), aliases) {
                accent_blue()
            } else {
                fg_muted()
            }
        }))
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(11.5))
                .text_color(move || {
                    if filter_has_token(&filter.get(), aliases) {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

fn filter_has_token(filter: &str, aliases: &[&str]) -> bool {
    let normalized = crate::session::normalize_filter_pipes(filter);
    normalized.split_whitespace().any(|part| {
        let lower = part.to_ascii_lowercase();
        aliases.iter().any(|alias| lower == *alias)
    })
}

/// Adds `token` as its own word, or removes every alias of that chip.
fn toggle_filter_chip(filter: &str, token: &str, aliases: &[&str]) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let mut kept = Vec::new();
    let mut found = false;
    for part in normalized.split_whitespace() {
        let lower = part.to_ascii_lowercase();
        if aliases.iter().any(|alias| lower == *alias) {
            found = true;
            continue;
        }
        kept.push(part.to_string());
    }
    if !found {
        kept.push(token.to_string());
    }
    kept.join(" ")
}

fn split_ui_or_items(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for item in text
        .split(|ch: char| ch == '|' || ch == ',' || ch.is_whitespace())
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        if !out
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(item))
        {
            out.push(item.to_string());
        }
    }
    out
}

fn extract_host_or(filter: &str) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let mut items = Vec::new();
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if let Some(rest) = lower
            .strip_prefix("host:")
            .or_else(|| lower.strip_prefix("domain:"))
        {
            for part in split_ui_or_items(rest) {
                if !items
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(&part))
                {
                    items.push(part);
                }
            }
        }
    }
    items.join(" | ")
}

fn set_host_or(filter: &str, input: &str) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let items = split_ui_or_items(input);
    let mut out = Vec::new();
    let mut inserted = false;
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if lower.starts_with("host:") || lower.starts_with("domain:") {
            if !inserted && !items.is_empty() {
                out.push(format!("host:{}", items.join("|")));
                inserted = true;
            }
            continue;
        }
        out.push(token.to_string());
    }
    if !inserted && !items.is_empty() {
        out.insert(0, format!("host:{}", items.join("|")));
    }
    out.join(" ")
}

fn extract_path_or(filter: &str) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let mut items = Vec::new();
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("path:") {
            for part in split_ui_or_items(rest) {
                if !items
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(&part))
                {
                    items.push(part);
                }
            }
        }
    }
    items.join(" | ")
}

fn set_path_or(filter: &str, input: &str) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let items = split_ui_or_items(input);
    let mut out = Vec::new();
    let mut inserted = false;
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if lower.starts_with("path:") {
            if !inserted && !items.is_empty() {
                out.push(format!("path:{}", items.join("|")));
                inserted = true;
            }
            continue;
        }
        out.push(token.to_string());
    }
    if !inserted && !items.is_empty() {
        out.push(format!("path:{}", items.join("|")));
    }
    out.join(" ")
}

fn extract_exclude_or(filter: &str) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let mut items = Vec::new();
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        let rest = lower.strip_prefix("mute:").or_else(|| {
            lower
                .strip_prefix('-')
                .filter(|rest| !rest.is_empty() && !rest.contains(':'))
        });
        if let Some(rest) = rest {
            for part in split_ui_or_items(rest) {
                if !items
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(&part))
                {
                    items.push(part);
                }
            }
        }
    }
    items.join(" | ")
}

fn set_exclude_or(filter: &str, input: &str) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let items: Vec<String> = split_ui_or_items(input)
        .into_iter()
        .map(|item| {
            item.strip_prefix("mute:")
                .or_else(|| item.strip_prefix('-'))
                .unwrap_or(&item)
                .to_string()
        })
        .filter(|item| !item.is_empty())
        .collect();
    let mut out = Vec::new();
    let mut inserted = false;
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        let is_exclude = lower.starts_with("mute:")
            || lower
                .strip_prefix('-')
                .is_some_and(|rest| !rest.is_empty() && !rest.contains(':'));
        if is_exclude {
            if !inserted && !items.is_empty() {
                out.push(format!("mute:{}", items.join("|")));
                inserted = true;
            }
            continue;
        }
        out.push(token.to_string());
    }
    if !inserted && !items.is_empty() {
        out.push(format!("mute:{}", items.join("|")));
    }
    out.join(" ")
}

fn extract_app_or(filter: &str) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let mut items = Vec::new();
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        let rest_len = if lower.starts_with("app:") {
            Some(4)
        } else if lower.starts_with("proc:") {
            Some(5)
        } else if lower.starts_with("ip:") {
            Some(3)
        } else if lower.starts_with("client:") {
            Some(7)
        } else {
            None
        };
        if let Some(prefix_len) = rest_len {
            for part in split_ui_or_items(&token[prefix_len..]) {
                if !items
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(&part))
                {
                    items.push(part);
                }
            }
        }
    }
    items.join(" | ")
}

fn set_app_or(filter: &str, input: &str) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let items = split_ui_or_items(input);
    let mut out = Vec::new();
    let mut inserted = false;
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if lower.starts_with("app:")
            || lower.starts_with("proc:")
            || lower.starts_with("ip:")
            || lower.starts_with("client:")
        {
            if !inserted && !items.is_empty() {
                out.push(format!("app:{}", items.join("|")));
                inserted = true;
            }
            continue;
        }
        out.push(token.to_string());
    }
    if !inserted && !items.is_empty() {
        out.push(format!("app:{}", items.join("|")));
    }
    out.join(" ")
}

fn host_facet_active(filter: &str, host: &str) -> bool {
    split_ui_or_items(&extract_host_or(filter))
        .iter()
        .any(|item| item.eq_ignore_ascii_case(host))
}

fn toggle_host_facet(filter: &str, host: &str) -> String {
    let mut items = split_ui_or_items(&extract_host_or(filter));
    if let Some(pos) = items
        .iter()
        .position(|item| item.eq_ignore_ascii_case(host))
    {
        items.remove(pos);
    } else {
        items.push(host.to_string());
    }
    set_host_or(filter, &items.join(" | "))
}

fn app_facet_active(filter: &str, app_token: &str) -> bool {
    split_ui_or_items(&extract_app_or(filter))
        .iter()
        .any(|item| item.eq_ignore_ascii_case(app_token))
}

fn toggle_app_facet(filter: &str, app_token: &str) -> String {
    let mut items = split_ui_or_items(&extract_app_or(filter));
    if let Some(pos) = items
        .iter()
        .position(|item| item.eq_ignore_ascii_case(app_token))
    {
        items.remove(pos);
    } else {
        items.push(app_token.to_string());
    }
    set_app_or(filter, &items.join(" | "))
}

fn mute_facet_active(filter: &str, token: &str) -> bool {
    split_ui_or_items(&extract_exclude_or(filter))
        .iter()
        .any(|item| item.eq_ignore_ascii_case(token))
}

fn toggle_mute_facet(filter: &str, token: &str) -> String {
    let mut items = split_ui_or_items(&extract_exclude_or(filter));
    if let Some(pos) = items
        .iter()
        .position(|item| item.eq_ignore_ascii_case(token))
    {
        items.remove(pos);
    } else {
        items.push(token.to_string());
    }
    set_exclude_or(filter, &items.join(" | "))
}

fn side_input(
    value: Signal<String>,
    placeholder: impl IntoReactive<String> + 'static,
    height: f32,
    multiline: bool,
    on_change: impl Fn(&str) + 'static,
) -> impl IntoElement {
    focused_input(value, placeholder, height, multiline, on_change, None)
}

fn focused_input(
    value: Signal<String>,
    placeholder: impl IntoReactive<String> + 'static,
    height: f32,
    multiline: bool,
    on_change: impl Fn(&str) + 'static,
    focus: Option<ElementRef>,
) -> impl IntoElement {
    let field = input()
        .value(value)
        .placeholder(placeholder)
        .placeholder_color(move || color(fg_muted()))
        .w_full()
        .h(px(height))
        .px_2p5()
        .rounded(px(7.0))
        .bg(bg_code())
        .border_1()
        .border_color(border_control())
        .focus(|style| style.border_color(accent_blue()))
        .text_size(px(12.5))
        .text_color(fg_primary())
        .multiline(multiline)
        .on_change(on_change);
    let field = if multiline { field.py_2() } else { field };
    match focus {
        Some(focus) => field.element_ref(&focus),
        None => field,
    }
}

fn config_set(bypass: Signal<String>, rules: Signal<String>, upstream: Signal<String>) -> bool {
    !bypass.get().trim().is_empty()
        || !rules.get().trim().is_empty()
        || !upstream.get().trim().is_empty()
}

fn capture_muted(rules_on: Signal<bool>, breaks_on: Signal<bool>) -> bool {
    !rules_on.get() || !breaks_on.get()
}

fn enable_switch(
    on: Signal<bool>,
    zh: &'static str,
    en: &'static str,
    apply: impl Fn(bool) + 'static,
) -> impl IntoElement {
    view()
        .h(px(28.0))
        .px_2()
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .justify_center()
        .gap_1p5()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(move || {
            if on.get() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if on.get() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if on.get() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .on_press(move || {
            let next = !on.get();
            on.set(next);
            apply(next);
        })
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .bg(move || if on.get() { accent_blue() } else { fg_muted() }),
        )
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(12.0))
                .text_color(move || {
                    if on.get() {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

fn rules_editor(
    value: Signal<String>,
    controls: Arc<Controls>,
    import_open: Signal<bool>,
) -> impl IntoElement {
    view()
        .w_full()
        .h(move || {
            px(adaptive_rules_editor_height_for(
                window().height.0,
                import_open.get(),
            ))
        })
        .rounded(px(7.0))
        .border_1()
        .border_color(border_control())
        .bg(bg_code())
        .overflow_hidden()
        .child(
            editor()
                .value(move || SharedString::from(value.get()))
                .language("plaintext")
                .line_numbers(true)
                .folding(false)
                .soft_wrap(true)
                .font_size(Points(12.0))
                .line_height(Points(18.0))
                .scroll_beyond_last_line(false)
                .placeholder(move || {
                    SharedString::from(tr(
                        "一行一条：break example.com/api",
                        "One rule per line: break example.com/api",
                    )())
                })
                .on_change(move |text: &SharedString| {
                    let text = text.to_string();
                    value.set(text.clone());
                    controls.hub.set_rules(&text);
                    if let Err(err) = save_config(|config| config.rules = text) {
                        set_status(
                            &controls,
                            StatusMsg::PrefUnsaved {
                                name: PrefName::Rules,
                                err: err.to_string(),
                            },
                        );
                    }
                })
                .size_full(),
        )
}

fn quick_side_chip(
    response: Signal<bool>,
    target_res: bool,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    view()
        .h(px(26.0))
        .px_2()
        .flex_row()
        .items_center()
        .justify_center()
        .border_r(move || if !target_res { px(1.0) } else { px(0.0) })
        .border_r_color(border_control())
        .bg(move || {
            if response.get() == target_res {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if response.get() == target_res {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .on_press(move || response.set(target_res))
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if response.get() == target_res {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

fn replace_row(
    rules_text: Signal<String>,
    controls: Arc<Controls>,
    host_pattern: Signal<String>,
    path_pattern: Signal<String>,
    op_pattern: Signal<String>,
    from: Signal<String>,
    to: Signal<String>,
    modes: QuickRuleModeSignals,
    kind: Signal<u8>,
) -> impl IntoElement {
    let ready_to_add = move || {
        let kind_now = kind.get();
        let mode_now = modes.mode_for(kind_now);
        let from_raw = from.get();
        let to_raw = to.get();
        let op_raw = op_pattern.get();
        let (eff_from, eff_to) = effective_quick_inputs(kind_now, mode_now, &from_raw, &to_raw);
        let eff_op = effective_quick_op(kind_now, &op_raw);
        let combined = crate::rules::combine_rule_pattern(&host_pattern.get(), &path_pattern.get());
        crate::rules::append_quick_rule_with_op(
            "",
            quick_kind(kind_now),
            &combined,
            mode_now,
            eff_from,
            eff_to,
            eff_op,
        )
        .is_some()
    };
    view()
        .gap_1()
        .w_full()
        .child(
            text(tr("快速添加规则", "Quick Add Rule"))
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_muted()),
        )
        .child(
            view()
                .w_full()
                .p_2()
                .gap_1p5()
                .rounded(px(8.0))
                .border_1()
                .border_color(border_subtle())
                .bg(bg_canvas())
                .child(
                    view()
                        .w_full()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_1()
                        .children((0_u8..14).map(move |idx| {
                            let (zh, en) = quick_kind(idx).label();
                            view()
                                .h(px(26.0))
                                .px_2()
                                .flex_shrink_0()
                                .flex_row()
                                .items_center()
                                .justify_center()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(move || {
                                    if kind.get() == idx {
                                        accent_blue_border()
                                    } else {
                                        border_control()
                                    }
                                })
                                .bg(move || {
                                    if kind.get() == idx {
                                        accent_blue_soft()
                                    } else {
                                        bg_control()
                                    }
                                })
                                .hover(|style| {
                                    style.bg(move || {
                                        if kind.get() == idx {
                                            accent_blue_soft_hover()
                                        } else {
                                            bg_control_hover()
                                        }
                                    })
                                })
                                .active(|style| style.opacity(0.8))
                                .cursor(Cursor::Pointer)
                                .on_press(move || {
                                    kind.set(idx);
                                })
                                .child(
                                    text(tr(zh, en))
                                        .number_of_lines(1)
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(move || {
                                            if kind.get() == idx {
                                                accent_blue_text()
                                            } else {
                                                fg_secondary()
                                            }
                                        }),
                                )
                        })),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(view().flex_1().min_w(px(0.0)).child(side_input(
                            host_pattern,
                            tr("域名 (*)", "Host (*)"),
                            28.0,
                            false,
                            move |text| host_pattern.set(text.to_string()),
                        )))
                        .child(view().flex_1().min_w(px(0.0)).child(side_input(
                            path_pattern,
                            tr("Path (*)", "Path (*)"),
                            28.0,
                            false,
                            move |text| path_pattern.set(text.to_string()),
                        )))
                        .child(when(
                            move || quick_kind(kind.get()).supports_graphql_op(),
                            move || {
                                view().w(px(94.0)).flex_shrink(1.0).min_w(px(0.0)).child(
                                    side_input(
                                        op_pattern,
                                        tr("GraphQL op", "GraphQL op"),
                                        28.0,
                                        false,
                                        move |text| op_pattern.set(text.to_string()),
                                    ),
                                )
                            },
                        ))
                        .child(when(
                            move || matches!(kind.get(), 0 | 2 | 6 | 10 | 11 | 13),
                            move || {
                                view()
                                    .h(px(28.0))
                                    .flex_shrink_0()
                                    .flex_row()
                                    .items_center()
                                    .rounded(px(6.0))
                                    .border_1()
                                    .border_color(border_control())
                                    .overflow_hidden()
                                    .child(when(
                                        move || kind.get() == 10,
                                        move || {
                                            view()
                                                .flex_row()
                                                .items_center()
                                                .child(quick_side_chip(
                                                    modes.mitm_mtls,
                                                    false,
                                                    "MITM",
                                                    "MITM",
                                                ))
                                                .child(quick_side_chip(
                                                    modes.mitm_mtls,
                                                    true,
                                                    "mTLS",
                                                    "mTLS",
                                                ))
                                        },
                                    ))
                                    .child(when(
                                        move || kind.get() == 2,
                                        move || {
                                            view()
                                                .flex_row()
                                                .items_center()
                                                .child(quick_side_chip(
                                                    modes.local_fallback,
                                                    false,
                                                    "本地",
                                                    "Local",
                                                ))
                                                .child(quick_side_chip(
                                                    modes.local_fallback,
                                                    true,
                                                    "回源",
                                                    "Fallback",
                                                ))
                                        },
                                    ))
                                    .child(when(
                                        move || kind.get() == 13,
                                        move || {
                                            view()
                                                .flex_row()
                                                .items_center()
                                                .child(quick_side_chip(
                                                    modes.status_drop,
                                                    false,
                                                    "状态码",
                                                    "Status",
                                                ))
                                                .child(quick_side_chip(
                                                    modes.status_drop,
                                                    true,
                                                    "断开",
                                                    "Drop",
                                                ))
                                        },
                                    ))
                                    .child(when(
                                        move || !matches!(kind.get(), 2 | 10 | 13),
                                        move || {
                                            view()
                                                .flex_row()
                                                .items_center()
                                                .child(quick_side_chip(
                                                    modes.req_res,
                                                    false,
                                                    "请求",
                                                    "Req",
                                                ))
                                                .child(quick_side_chip(
                                                    modes.req_res,
                                                    true,
                                                    "响应",
                                                    "Res",
                                                ))
                                        },
                                    ))
                            },
                        )),
                )
                .child({
                    let row = view().flex_row().items_center().gap_1().child(when(
                        move || {
                            !matches!(kind.get(), 4 | 5 | 8 | 12)
                                && !(kind.get() == 13 && modes.status_drop.get())
                        },
                        move || {
                            let placeholder = move || {
                                let lang = ui_lang(ui_langs());
                                match kind.get() {
                                    1 => t(
                                        lang,
                                        "目标地址: https://other.test/v1",
                                        "Target URL: https://other.test/v1",
                                    )
                                    .to_string(),
                                    2 => t(
                                        lang,
                                        "本地文件或目录路径 (或留空仅填状态码)",
                                        "Local file or directory (or empty for status)",
                                    )
                                    .to_string(),
                                    3 => t(
                                        lang,
                                        "延迟时长: 2000ms 或 2s",
                                        "Delay duration: 2000ms or 2s",
                                    )
                                    .to_string(),
                                    6 => t(lang, "Header 名称: X-Debug", "Header name: X-Debug")
                                        .to_string(),
                                    7 => t(lang, "参数名: env", "Query param: env").to_string(),
                                    9 => t(
                                        lang,
                                        "下行速率或预设: 3g / 256kbps",
                                        "Down rate or preset: 3g / 256kbps",
                                    )
                                    .to_string(),
                                    10 => t(
                                        lang,
                                        "证书链文件 (.crt / .pem)",
                                        "Certificate chain (.crt / .pem)",
                                    )
                                    .to_string(),
                                    11 => t(
                                        lang,
                                        "JSON 路径: /data/enabled 或 items[0].price",
                                        "JSON path: /data/enabled or items[0].price",
                                    )
                                    .to_string(),
                                    13 => t(
                                        lang,
                                        "状态码 (默认 403): 403 / 404 / 503",
                                        "Status (default 403): 403 / 404 / 503",
                                    )
                                    .to_string(),
                                    _ => t(lang, "查找原文 (from)", "Find text (from)").to_string(),
                                }
                            };
                            view().flex_1().min_w(px(0.0)).child(side_input(
                                from,
                                placeholder,
                                28.0,
                                false,
                                move |text| from.set(text.to_string()),
                            ))
                        },
                    ));
                    let row = desktop_child!(
                        row,
                        when(
                            move || kind.get() == 2 || kind.get() == 10,
                            move || {
                                view()
                                    .h(px(28.0))
                                    .px_2p5()
                                    .flex_shrink_0()
                                    .flex_row()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(6.0))
                                    .border_1()
                                    .border_color(border_control())
                                    .bg(bg_control())
                                    .hover(|style| style.bg(bg_control_hover()))
                                    .cursor(Cursor::Pointer)
                                    .on_press(move || {
                                        spawn_detached(async move {
                                            if let Ok(Some(paths)) =
                                                pick_paths(open_dialog_with_default_dir()).await
                                                && let Some(path) = paths.first()
                                            {
                                                from.set(path.display().to_string());
                                            }
                                        });
                                    })
                                    .child(
                                        text(move || {
                                            if kind.get() == 10 {
                                                tr("选择证书", "Cert…")()
                                            } else {
                                                tr("选择文件", "Browse")()
                                            }
                                        })
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(fg_secondary()),
                                    )
                            },
                        )
                    );
                    let row = row.child(when(
                        move || matches!(kind.get(), 0 | 2 | 6 | 7 | 9 | 10 | 11),
                        move || {
                            let placeholder = move || {
                                let lang = ui_lang(ui_langs());
                                match kind.get() {
                                    2 => t(lang, "状态码: 200 (可选)", "Status: 200 (optional)")
                                        .to_string(),
                                    6 => t(lang, "Header 值 (留空删除)", "Value (empty to delete)")
                                        .to_string(),
                                    7 => t(lang, "参数值 (留空删除)", "Value (empty to delete)")
                                        .to_string(),
                                    9 => t(
                                        lang,
                                        "上行速率 (可选): 64kbps",
                                        "Up rate (optional): 64kbps",
                                    )
                                    .to_string(),
                                    10 => t(
                                        lang,
                                        "私钥文件 (.key，合并 PEM 可留空)",
                                        "Private key (.key, optional if combined)",
                                    )
                                    .to_string(),
                                    11 => t(
                                        lang,
                                        "JSON 值: true / 42 / \"text\" (留空删除)",
                                        "JSON value: true / 42 / \"text\" (empty to delete)",
                                    )
                                    .to_string(),
                                    _ => t(lang, "替换成 (to)", "Replace with (to)").to_string(),
                                }
                            };
                            view().flex_1().min_w(px(0.0)).child(side_input(
                                to,
                                placeholder,
                                28.0,
                                false,
                                move |text| to.set(text.to_string()),
                            ))
                        },
                    ));
                    let row = desktop_child!(
                        row,
                        when(
                            move || kind.get() == 10,
                            move || {
                                view()
                                    .h(px(28.0))
                                    .px_2p5()
                                    .flex_shrink_0()
                                    .flex_row()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(6.0))
                                    .border_1()
                                    .border_color(border_control())
                                    .bg(bg_control())
                                    .hover(|style| style.bg(bg_control_hover()))
                                    .cursor(Cursor::Pointer)
                                    .on_press(move || {
                                        spawn_detached(async move {
                                            if let Ok(Some(paths)) =
                                                pick_paths(open_dialog_with_default_dir()).await
                                                && let Some(path) = paths.first()
                                            {
                                                to.set(path.display().to_string());
                                            }
                                        });
                                    })
                                    .child(
                                        text(tr("选择私钥", "Key…"))
                                            .text_size(px(11.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(fg_secondary()),
                                    )
                            },
                        )
                    );
                    row.child(
                        view()
                            .h(px(28.0))
                            .px_3()
                            .flex_grow(move || {
                                if matches!(kind.get(), 4 | 5 | 8 | 12)
                                    || (kind.get() == 13 && modes.status_drop.get())
                                {
                                    1.0_f32
                                } else {
                                    0.0_f32
                                }
                            })
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(move || {
                                if ready_to_add() {
                                    accent_blue_border()
                                } else {
                                    border_control()
                                }
                            })
                            .bg(move || {
                                if ready_to_add() {
                                    accent_blue_soft()
                                } else {
                                    bg_control()
                                }
                            })
                            .hover(|style| {
                                style.bg(move || {
                                    if ready_to_add() {
                                        accent_blue_soft_hover()
                                    } else {
                                        bg_control()
                                    }
                                })
                            })
                            .active(|style| {
                                style.opacity(
                                    move || {
                                        if ready_to_add() { 0.8_f32 } else { 1.0_f32 }
                                    },
                                )
                            })
                            .disabled(move || !ready_to_add())
                            .disabled_style(|style| style.opacity(0.55).cursor(Cursor::Default))
                            .cursor(Cursor::Pointer)
                            .on_press(move || {
                                let kind_now = kind.get();
                                let mode_now = modes.mode_for(kind_now);
                                let from_raw = from.get();
                                let to_raw = to.get();
                                let op_raw = op_pattern.get();
                                let (eff_from, eff_to) =
                                    effective_quick_inputs(kind_now, mode_now, &from_raw, &to_raw);
                                let eff_op = effective_quick_op(kind_now, &op_raw);
                                let combined_pattern = crate::rules::combine_rule_pattern(
                                    &host_pattern.get(),
                                    &path_pattern.get(),
                                );
                                let Some(next) = crate::rules::append_quick_rule_with_op(
                                    &rules_text.get(),
                                    quick_kind(kind_now),
                                    &combined_pattern,
                                    mode_now,
                                    eff_from,
                                    eff_to,
                                    eff_op,
                                ) else {
                                    return;
                                };
                                rules_text.set(next.clone());
                                controls.hub.set_rules(&next);
                                if let Err(err) = save_config(|config| config.rules = next) {
                                    set_status(
                                        &controls,
                                        StatusMsg::PrefUnsaved {
                                            name: PrefName::Rules,
                                            err: err.to_string(),
                                        },
                                    );
                                }
                                host_pattern.set(String::new());
                                path_pattern.set(String::new());
                                op_pattern.set(String::new());
                                from.set(String::new());
                                to.set(String::new());
                            })
                            .child(
                                text(tr("+ 添加规则", "+ Add"))
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(move || {
                                        if ready_to_add() {
                                            accent_blue_text()
                                        } else {
                                            fg_muted()
                                        }
                                    }),
                            ),
                    )
                }),
        )
}

fn config_field(zh: &'static str, en: &'static str, field: impl IntoElement) -> impl IntoElement {
    view()
        .gap_1()
        .w_full()
        .child(
            text(tr(zh, en))
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_muted()),
        )
        .child(field)
}

fn list_empty(filter: Signal<String>) -> impl IntoElement {
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
                    if filter.get().is_empty() {
                        t(lang, "还没有流量", "No traffic yet").into()
                    } else {
                        t(lang, "没有匹配的流量", "No matching traffic").into()
                    }
                }
            })
            .text_center()
            .text_size(px(13.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(fg_secondary()),
        )
        .child(
            text({
                let langs = ui_langs();
                move || -> SharedString {
                    let lang = ui_lang(langs);
                    if filter.get().is_empty() {
                        t(
                            lang,
                            "点击右上角「开始」捕获 HTTP(S) 请求",
                            "Press Start to capture HTTP(S)",
                        )
                        .into()
                    } else {
                        t(lang, "试试调整上方的过滤关键词", "Try a different filter").into()
                    }
                }
            })
            .text_center()
            .text_size(px(12.0))
            .text_color(fg_muted()),
        )
}

#[allow(clippy::too_many_arguments)]
fn list_menu(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
    compose: Compose,
    columns: Signal<ListColumns>,
    sort: Signal<Option<ListSort>>,
    controls: Arc<Controls>,
    list_filter: Signal<String>,
    effective_filter: Signal<String>,
    shown: Shown,
    baseline: Signal<Option<u64>>,
    rules_text: Signal<String>,
    picked: Signal<Option<u64>>,
    menu_open: Signal<bool>,
    menu_at: Signal<Point<Points>>,
) -> impl IntoElement {
    let header_menu = signal(false);
    let header_controls = controls.clone();
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
            effective_filter,
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
                        rules_text,
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

fn flow_table(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    compose: Compose,
    columns: Signal<ListColumns>,
    sort: Signal<Option<ListSort>>,
    controls: Arc<Controls>,
    filter: Signal<String>,
    picked: Signal<Option<u64>>,
    menu_open: Signal<bool>,
    header_menu: Signal<bool>,
    menu_at: Signal<Point<Points>>,
) -> impl IntoElement {
    let widths = signal(saved_config().column_widths);
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
    };
    let save_job = stored(None::<Spawned<()>>);
    let widths_for_cols = widths;
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
            Some(
                data_table()
                    .component_ref(&table)
                    .flex_1()
                    .min_h(px(0.0))
                    .w_full()
                    .classes(flow_table_classes())
                    .columns(move || {
                        let shown = columns.get();
                        let current_widths = untracked(|| widths_for_cols.get());
                        let mut built = flow_table_columns(shown_at_mount, &current_widths);
                        for column in &mut built {
                            let key = column.key.to_string();
                            column.title = Some(flow_column_title(&key, shown.full_url).into());
                        }
                        built
                    })
                    .data(DataTableRows::default())
                    .row_count(move || Some(rows.with(Vec::len) as u32))
                    .row_height(px(34.0))
                    .header_height(px(28.0))
                    .font_size(px(12.0))
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
                            hover,
                            cell,
                        )
                    })
                    .render_header_cell(move |cell: &DataTableHeaderCell| {
                        let blur_header = Rc::clone(&blur_header);
                        let column = ListColumn::from_key(cell.key.as_ref());
                        flow_header(cell, columns, sort).on_press(move || {
                            if let Some(column) = column {
                                sort.set(ListSort::toggle(sort.get(), column));
                            }
                            blur_header();
                        })
                    })
                    .empty(list_empty(filter))
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
                            if selected.get() != next {
                                reset_compose(compose);
                            }
                            selected.set(next);
                            menu_open.set(true);
                        } else {
                            menu_open.set(false);
                            header_menu.set(true);
                        }
                    })
                    .on_visible_rows_change(move |range: &DataTableRange| {
                        hover.text.set(None);
                        let count = rows.with(Vec::len) as u32;
                        if count == 0 {
                            follow.replace(true);
                            followed_count.replace(0);
                            return;
                        }
                        let near = range.end.saturating_add(1) >= count;
                        if follow.get() && count > followed_count.get() {
                            followed_count.replace(count);
                            if !near && let Some(api) = table.api::<DataTableApi>() {
                                api.scroll_to_row(count - 1, Some(false));
                            }
                            return;
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
                        widths.update(|map| {
                            if px == limit.default {
                                map.remove(key.as_ref());
                            } else {
                                map.insert(key.to_string(), px);
                            }
                        });
                        let widths = widths;
                        let controls = controls.clone();
                        save_job.replace(Some(set_timeout(
                            Duration::from_millis(300),
                            move || {
                                let map = widths.get();
                                if let Err(err) = save_config(|config| config.column_widths = map) {
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
struct CellHover {
    text: Signal<Option<String>>,
    at: Signal<Point<Points>>,
}

fn show_cell_hover(hover: CellHover, event: &MouseEvent, text: String) {
    if text.is_empty() {
        hover.text.set(None);
        return;
    }
    hover.at.set(event.page_position);
    hover.text.set(Some(text));
}

fn column_menu(columns: Signal<ListColumns>, controls: Arc<Controls>) -> impl IntoElement {
    let open = signal(false);
    menu()
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(4.0))
        .margin(px(8.0))
        .on_open_change(move |event: &nsh_base::MenuEvent| {
            if let nsh_base::MenuEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .trigger(column_picker_trigger(open))
        .child(column_picker_panel(columns, controls))
}

fn column_picker_trigger(open: Signal<bool>) -> impl IntoElement {
    view()
        .w(px(30.0))
        .h(px(30.0))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(move || {
            if open.get() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if open.get() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if open.get() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .label("Columns")
        .tooltip(label_tip("列", "Columns"))
        .child(glyph(
            || Glyph::Columns,
            move || {
                if open.get() {
                    color(accent_blue_text())
                } else {
                    color(fg_secondary())
                }
            },
        ))
}

fn column_picker_panel(columns: Signal<ListColumns>, controls: Arc<Controls>) -> impl IntoElement {
    let fixed_controls = controls.clone();
    let scroll_controls = controls.clone();
    let reset_controls = controls.clone();
    view()
        .w(px(204.0))
        .max_w(move || px((window().width.0 - 16.0).max(160.0)))
        .max_h(move || px((window().height.0 - 24.0).max(180.0)))
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
        .overflow_hidden()
        .child(
            scroll()
                .w_full()
                .max_h(move || px((window().height.0 - 26.0).max(178.0)))
                .child(
                    view()
                        .w_full()
                        .p_1()
                        .gap_0p5()
                        .children((0..3).map(move |slot| {
                            column_slot_row(columns, fixed_controls.clone(), slot)
                        }))
                        .child(menu_rule())
                        .children((3..ListColumn::ORDERED.len()).map(move |slot| {
                            column_slot_row(columns, scroll_controls.clone(), slot)
                        }))
                        .child(when(
                            move || columns.get().has_custom_order(),
                            move || column_reset_order(columns, reset_controls.clone()),
                        ))
                        .child(menu_rule())
                        .child(column_full_url_choice(
                            columns,
                            controls.clone(),
                            false,
                            "仅展示 Path",
                            "Path only",
                        ))
                        .child(column_full_url_choice(
                            columns,
                            controls.clone(),
                            true,
                            "完整 URL",
                            "Full URL",
                        ))
                        .child(menu_rule())
                        .child(column_ellipsize_choice(
                            columns,
                            controls.clone(),
                            config::ListEllipsize::Head,
                            "开头省略",
                            "Ellipsize head",
                        ))
                        .child(column_ellipsize_choice(
                            columns,
                            controls.clone(),
                            config::ListEllipsize::Middle,
                            "中间省略",
                            "Ellipsize middle",
                        ))
                        .child(column_ellipsize_choice(
                            columns,
                            controls,
                            config::ListEllipsize::Tail,
                            "结尾省略",
                            "Ellipsize tail",
                        )),
                ),
        )
}

fn column_slot_row(
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
    slot: usize,
) -> impl IntoElement {
    let toggle_controls = controls.clone();
    let up_controls = controls.clone();
    let down_controls = controls;
    let is_on = move || {
        let cols = columns.get();
        cols.on(cols.order[slot])
    };
    let locked = move || is_on() && columns.get().visible_count() <= 1;
    view()
        .h(px(30.0))
        .pr_0p5()
        .gap_0p5()
        .flex_row()
        .items_center()
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .pl_2p5()
                .pr_1p5()
                .gap_2()
                .rounded(px(6.0))
                .flex_row()
                .items_center()
                .opacity(move || if locked() { 0.55_f32 } else { 1.0_f32 })
                .hover(|style| {
                    style.bg(move || {
                        if locked() {
                            AdaptiveColor::TRANSPARENT
                        } else {
                            bg_control_hover()
                        }
                    })
                })
                .active(|style| style.opacity(move || if locked() { 0.55_f32 } else { 0.75_f32 }))
                .cursor(Cursor::Pointer)
                .disabled(locked)
                .disabled_style(|style| style.cursor(Cursor::Default))
                .on_press(move || {
                    let current = columns.get();
                    let next = current.toggle(current.order[slot]);
                    if next == current {
                        return;
                    }
                    columns.set(next);
                    if let Err(err) = save_config(|config| next.write(config)) {
                        set_status(
                            &toggle_controls,
                            StatusMsg::PrefUnsaved {
                                name: PrefName::List,
                                err: err.to_string(),
                            },
                        );
                    }
                })
                .child(
                    view()
                        .w(px(8.5))
                        .h(px(8.5))
                        .flex_shrink_0()
                        .rounded(px(2.5))
                        .border_1()
                        .border_color(move || if is_on() { accent_blue() } else { fg_muted() })
                        .bg(move || if is_on() { accent_blue() } else { bg_card() }),
                )
                .child(
                    control_text(move || {
                        let (zh, en) = columns.get().order[slot].labels();
                        tr(zh, en)()
                    })
                    .flex_1()
                    .min_w(px(0.0))
                    .number_of_lines(1)
                    .text_color(move || {
                        if is_on() {
                            accent_blue_text()
                        } else {
                            fg_secondary()
                        }
                    }),
                ),
        )
        .child(column_move_button(columns, up_controls, slot, -1))
        .child(column_move_button(columns, down_controls, slot, 1))
}

fn column_move_button(
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
    slot: usize,
    delta: isize,
) -> impl IntoElement {
    let can_move = ListColumns::can_move_slot(slot, delta);
    let icon = if delta < 0 {
        Glyph::ChevronUp
    } else {
        Glyph::Chevron
    };
    let (zh, en) = if delta < 0 {
        ("上移", "Move up")
    } else {
        ("下移", "Move down")
    };
    let btn = view()
        .w(px(22.0))
        .h(px(22.0))
        .flex_shrink_0()
        .rounded(px(5.0))
        .flex_row()
        .items_center()
        .justify_center()
        .child(glyph_px(
            11.5,
            move || icon,
            move || color(if can_move { fg_secondary() } else { fg_muted() }),
        ));
    if can_move {
        btn.hover(|style| style.bg(bg_control_hover()))
            .active(|style| style.opacity(0.75))
            .cursor(Cursor::Pointer)
            .label(en)
            .tooltip(label_tip(zh, en))
            .on_press(move || {
                let current = columns.get();
                let column = current.order[slot];
                let next = current.move_by(column, delta);
                if next == current {
                    return;
                }
                columns.set(next);
                if let Err(err) = save_config(|config| next.write(config)) {
                    set_status(
                        &controls,
                        StatusMsg::PrefUnsaved {
                            name: PrefName::List,
                            err: err.to_string(),
                        },
                    );
                }
            })
    } else {
        btn.opacity(0.28).cursor(Cursor::Default)
    }
}

fn column_reset_order(columns: Signal<ListColumns>, controls: Arc<Controls>) -> impl IntoElement {
    view()
        .h(px(28.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.opacity(0.75))
        .cursor(Cursor::Pointer)
        .on_press(move || {
            let current = columns.get();
            let next = current.reset_order();
            if next == current {
                return;
            }
            columns.set(next);
            if let Err(err) = save_config(|config| next.write(config)) {
                set_status(
                    &controls,
                    StatusMsg::PrefUnsaved {
                        name: PrefName::List,
                        err: err.to_string(),
                    },
                );
            }
        })
        .child(view().w(px(8.5)).h(px(8.5)).flex_shrink_0())
        .child(
            control_text(tr("恢复默认列序", "Reset column order"))
                .text_size(px(11.5))
                .text_color(accent_blue_text()),
        )
}

fn column_full_url_choice(
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
    full_url: bool,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    let marked = columns;
    marked_radio_stay(
        move || marked.get().full_url == full_url,
        tr(zh, en),
        move || {
            let next = columns.get().with_full_url(full_url);
            if next == columns.get() {
                return;
            }
            columns.set(next);
            if let Err(err) = save_config(|config| next.write(config)) {
                set_status(
                    &controls,
                    StatusMsg::PrefUnsaved {
                        name: PrefName::List,
                        err: err.to_string(),
                    },
                );
            }
        },
    )
}

fn column_ellipsize_choice(
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
    ellipsize: config::ListEllipsize,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    let marked = columns;
    marked_radio_stay(
        move || marked.get().ellipsize == ellipsize,
        tr(zh, en),
        move || {
            let next = columns.get().with_ellipsize(ellipsize);
            if next == columns.get() {
                return;
            }
            columns.set(next);
            if let Err(err) = save_config(|config| next.write(config)) {
                set_status(
                    &controls,
                    StatusMsg::PrefUnsaved {
                        name: PrefName::List,
                        err: err.to_string(),
                    },
                );
            }
        },
    )
}

fn edit_action_label(
    lang: Lang,
    target_id: Option<u64>,
    paused: Option<(u64, BreakSide)>,
) -> &'static str {
    match (target_id, paused) {
        (Some(id), Some((pid, BreakSide::Response))) if id == pid => {
            t(lang, "编辑响应", "Edit response")
        }
        (Some(id), Some((pid, BreakSide::Request))) if id == pid => {
            t(lang, "编辑请求", "Edit request")
        }
        _ => t(lang, "编辑重发", "Edit & resend"),
    }
}

fn row_context_menu(
    picked: Signal<Option<u64>>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
    compose: Compose,
    controls: Arc<Controls>,
    filter: Signal<String>,
    shown: Shown,
    baseline: Signal<Option<u64>>,
    rules_text: Signal<String>,
    menu_open: Signal<bool>,
) -> impl IntoElement {
    let copy_url = controls.clone();
    let break_controls = controls.clone();
    let copy_curl = controls.clone();
    let focus_controls = controls.clone();
    let mute_controls = controls.clone();
    let pin_controls = controls.clone();
    let pin_mark = controls.clone();
    let pin_label = controls.clone();
    let base_controls = controls.clone();
    let local_check = controls.clone();
    let map_label_check = controls.clone();
    let map_local_controls = controls.clone();
    let repeat_controls = controls.clone();
    let edit_controls = controls.clone();
    let replay_controls = controls;
    let can_local = move || {
        let controls = &local_check;
        picked.get().is_some_and(|id| {
            controls.hub.with_session(|session| {
                session
                    .get(id)
                    .is_some_and(|flow| flow.relay_source.is_none())
            })
        })
    };
    let can_replay = can_local.clone();
    let can_repeat = can_local.clone();
    let can_edit = can_local.clone();
    let can_break = can_local.clone();
    let can_map = can_local;
    view()
        .w(px(196.0))
        .max_w(move || px((window().width.0 - 16.0).max(160.0)))
        .max_h(move || px((window().height.0 - 24.0).max(160.0)))
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
        .overflow_hidden()
        .child(
            scroll()
                .w_full()
                .max_h(move || px((window().height.0 - 26.0).max(158.0)))
                .child(
                    view()
                        .w_full()
                        .p_1()
                        .gap_0p5()
                        .child(action_item(
                            tr("复制 URL", "Copy URL"),
                            closing(menu_open, move || {
                                copy_part(&copy_url, picked.get(), CopyPart::Url);
                            }),
                        ))
                        .child(action_item(
                            "cURL",
                            closing(menu_open, move || {
                                copy_part(&copy_curl, picked.get(), CopyPart::Curl);
                            }),
                        ))
                        .child(menu_rule())
                        .child(action_item(
                            tr("只看", "Focus"),
                            closing(menu_open, move || {
                                focus_host(&focus_controls, picked.get(), filter);
                            }),
                        ))
                        .child(action_item(
                            tr("屏蔽", "Mute"),
                            closing(menu_open, move || {
                                mute_host(&mute_controls, picked.get(), filter);
                            }),
                        ))
                        .child(menu_rule())
                        .child(marked_action(
                            move || flow_pinned(&pin_mark, picked.get()),
                            {
                                let langs = ui_langs();
                                move || -> SharedString {
                                    if flow_pinned(&pin_label, picked.get()) {
                                        t(ui_lang(langs), "取消星标", "Unstar").into()
                                    } else {
                                        t(ui_lang(langs), "星标", "Star").into()
                                    }
                                }
                            },
                            closing(menu_open, move || toggle_pin(&pin_controls, picked.get())),
                        ))
                        .child(marked_action(
                            move || baseline.get().is_some_and(|id| picked.get() == Some(id)),
                            {
                                let langs = ui_langs();
                                let picked_label = picked;
                                let baseline_label = baseline;
                                move || -> SharedString {
                                    if baseline_label
                                        .get()
                                        .is_some_and(|id| picked_label.get() == Some(id))
                                    {
                                        t(ui_lang(langs), "取消基准", "Clear baseline").into()
                                    } else {
                                        t(ui_lang(langs), "基准", "Baseline").into()
                                    }
                                }
                            },
                            {
                                let langs = ui_langs();
                                closing(menu_open, move || {
                                    set_baseline(
                                        &base_controls,
                                        picked.get(),
                                        baseline,
                                        &shown,
                                        ui_lang(langs),
                                    );
                                })
                            },
                        ))
                        .child(action_item(
                            tr("备注…", "Note…"),
                            closing(menu_open, move || {
                                shown.repeating.set(false);
                                shown.editing_comment.set(true);
                            }),
                        ))
                        .child(menu_rule())
                        .child(action_item_enabled(
                            tr("重放", "Replay"),
                            can_replay,
                            closing(menu_open, move || {
                                replay_flow(
                                    &replay_controls,
                                    picked.get(),
                                    ReplayOverride::default(),
                                );
                            }),
                        ))
                        .child(action_item_enabled(
                            tr("批量重放…", "Repeat…"),
                            can_repeat,
                            closing(menu_open, move || {
                                if refuse_relay(&repeat_controls, picked.get()) {
                                    return;
                                }
                                shown.editing_comment.set(false);
                                shown.repeating.set(true);
                            }),
                        ))
                        .child(action_item_enabled(
                            {
                                let langs = ui_langs();
                                move || -> SharedString {
                                    edit_action_label(ui_lang(langs), picked.get(), paused.get())
                                        .into()
                                }
                            },
                            can_edit,
                            closing(menu_open, move || {
                                let target = picked.get();
                                if target.is_some() {
                                    selected.set(target);
                                }
                                begin_compose(&edit_controls, target, compose);
                            }),
                        ))
                        .child(action_item_enabled(
                            tr("填入断点", "Add break"),
                            can_break,
                            closing(menu_open, move || {
                                fill_break_rule(&break_controls, picked.get(), rules_text);
                            }),
                        ))
                        .child(action_item_enabled(
                            {
                                let langs = ui_langs();
                                move || -> SharedString {
                                    #[cfg(not(target_arch = "wasm32"))]
                                    let saves_file = picked.get().is_some_and(|id| {
                                        map_label_check.hub.with_session(|session| {
                                            session
                                                .get(id)
                                                .is_some_and(|flow| !flow.response_body.is_empty())
                                        })
                                    });
                                    #[cfg(target_arch = "wasm32")]
                                    let saves_file = {
                                        let _ = (&picked, &map_label_check);
                                        false
                                    };
                                    if saves_file {
                                        t(ui_lang(langs), "本地映射…", "Map local…").into()
                                    } else {
                                        t(ui_lang(langs), "本地映射", "Map local").into()
                                    }
                                }
                            },
                            can_map,
                            closing(menu_open, move || {
                                map_local_from_flow(
                                    map_local_controls.clone(),
                                    picked.get(),
                                    rules_text,
                                );
                            }),
                        )),
                ),
        )
}

fn closing(open: Signal<bool>, run: impl Fn() + 'static) -> impl Fn() + 'static {
    move || {
        open.set(false);
        run();
    }
}

fn quick_kind(kind: u8) -> crate::rules::QuickRule {
    use crate::rules::QuickRule;
    match kind % 14 {
        0 => QuickRule::Replace,
        1 => QuickRule::Remote,
        2 => QuickRule::Local,
        3 => QuickRule::Delay,
        4 => QuickRule::Break,
        5 => QuickRule::BreakRes,
        6 => QuickRule::Header,
        7 => QuickRule::Query,
        8 => QuickRule::Cors,
        9 => QuickRule::Throttle,
        10 => QuickRule::Cert,
        11 => QuickRule::Json,
        12 => QuickRule::NoCache,
        _ => QuickRule::Block,
    }
}

fn fill_break_rule(controls: &Controls, id: Option<u64>, rules_text: Signal<String>) {
    if refuse_relay(controls, id) {
        return;
    }
    let Some(flow) = selected_flow(controls, id) else {
        return;
    };
    let Some(next) = crate::rules::append_break_line(&rules_text.get(), &flow.host, &flow.path)
    else {
        return;
    };
    rules_text.set(next.clone());
    controls.hub.set_rules(&next);
    if let Err(err) = save_config(|config| config.rules = next) {
        set_status(
            controls,
            StatusMsg::PrefUnsaved {
                name: PrefName::Rules,
                err: err.to_string(),
            },
        );
    }
}

fn map_local_from_flow(controls: Arc<Controls>, id: Option<u64>, rules_text: Signal<String>) {
    if refuse_relay(&controls, id) {
        return;
    }
    let Some(flow) = selected_flow(&controls, id) else {
        return;
    };
    let host = flow.host.clone();
    let path = flow.path.clone();
    let status = flow.status;
    #[cfg(target_arch = "wasm32")]
    let status_only = true;
    #[cfg(not(target_arch = "wasm32"))]
    let status_only = flow.response_body.is_empty();
    if status_only {
        let Some(next) = crate::rules::append_map_local_line(
            &rules_text.get(),
            &host,
            &path,
            None,
            Some(status.unwrap_or(200)),
        ) else {
            return;
        };
        rules_text.set(next.clone());
        controls.hub.set_rules(&next);
        if let Err(err) = save_config(|config| config.rules = next) {
            set_status(
                &controls,
                StatusMsg::PrefUnsaved {
                    name: PrefName::Rules,
                    err: err.to_string(),
                },
            );
            return;
        }
        set_status(&controls, StatusMsg::MappedLocal(format!("{host}{path}")));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let ext = present::content_type(&flow.response_headers)
            .and_then(present::image_extension)
            .unwrap_or_else(|| body_extension(&flow.response_headers, &flow.response_body));
        let dialog = save_dialog_with_default_dir()
            .file_name(format!("{}.{}", flow.id, ext))
            .filter(nsh_core::file_dialog::FileFilter::new(
                ext.to_ascii_uppercase(),
                [ext],
            ));
        let body = flow.response_body;
        spawn_detached(async move {
            match pick_save_path(dialog).await {
                Ok(Some(save_path)) => {
                    if let Err(err) = write_export_file(&save_path, &body) {
                        set_status(&controls, StatusMsg::SaveFailed(err.to_string()));
                        return;
                    }
                    let file_text = save_path.display().to_string();
                    let Some(next) = crate::rules::append_map_local_line(
                        &rules_text.get(),
                        &host,
                        &path,
                        Some(&file_text),
                        status,
                    ) else {
                        return;
                    };
                    rules_text.set(next.clone());
                    controls.hub.set_rules(&next);
                    if let Err(err) = save_config(|config| config.rules = next) {
                        set_status(
                            &controls,
                            StatusMsg::PrefUnsaved {
                                name: PrefName::Rules,
                                err: err.to_string(),
                            },
                        );
                        return;
                    }
                    set_clipboard(file_text.clone());
                    set_status(&controls, StatusMsg::MappedLocal(file_text));
                }
                Ok(None) => set_status(&controls, StatusMsg::SaveCancelled),
                Err(err) => set_status(&controls, StatusMsg::SaveFailed(err)),
            }
        });
    }
}

fn flow_pinned(controls: &Controls, id: Option<u64>) -> bool {
    id.is_some_and(|id| {
        controls
            .hub
            .with_session(|session| session.get(id).is_some_and(|flow| flow.pinned))
    })
}

fn flow_cell(
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

fn column_pad_right(column: Option<ListColumn>, align_right: bool, columns: ListColumns) -> Points {
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

fn flow_header(
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

fn row_edge_mark(
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

fn method_cell(
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

fn id_cell(
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

fn status_cell(
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

fn time_cell(
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

fn elapsed_cell(
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

fn host_cell(
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

fn client_ip_cell(
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

fn protocol_cell(
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

fn size_cell(
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

fn path_cell(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    columns: Signal<ListColumns>,
    hover: CellHover,
    index: usize,
) -> impl IntoElement {
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
                let langs = ui_langs();
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
            let langs = ui_langs();
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

fn row_target_text(row: &RowLine, full_url: bool) -> &str {
    if full_url && !row.url.is_empty() {
        &row.url
    } else {
        &row.path
    }
}

fn row_target_display(row: &RowLine, full_url: bool) -> String {
    let base = row_target_text(row, full_url);
    if row.comment.is_empty() {
        base.to_string()
    } else {
        format!("{base} · #{}", row.comment)
    }
}

fn label_tip(zh: &'static str, en: &'static str) -> impl IntoTooltip {
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
fn cell_tip(full: impl Fn() -> String + 'static) -> impl IntoElement {
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

#[allow(clippy::too_many_arguments)]
fn detail_pane(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
    compose: Compose,
    seq_builder: SeqBuilder,
    rows: Signal<Vec<Arc<RowLine>>>,
    list_filter: Signal<String>,
    effective_filter: Signal<String>,
    rules_text: Signal<String>,
    baseline: Signal<Option<u64>>,
    folded: Signal<u8>,
    pane_px: Rc<Cell<f32>>,
    filtered_stats: Signal<session::FilteredStats>,
    detail_scroll: ElementRef,
) -> impl IntoElement {
    let ready_controls = controls.clone();
    scroll()
        .element_ref(&detail_scroll)
        .flex_1()
        .min_w(px(0.0))
        .bg(bg_canvas())
        .on_layout(move |bounds: &Bounds<Points>| {
            let width = bounds.size.width.0;
            pane_px.set(width);
            if width > 0.0 && (width - shown.pane_width.get_untracked()).abs() > 1.0 {
                shown.pane_width.set(width);
            }
        })
        .child(
            when(
                move || shown.phase.get() == Phase::Ready,
                move || {
                    ready_detail(
                        shown,
                        ready_controls.clone(),
                        selected,
                        paused,
                        compose,
                        list_filter,
                        rules_text,
                        baseline,
                        folded,
                    )
                },
            )
            .otherwise(move || {
                let compose_controls = controls.clone();
                let seq_controls = controls.clone();
                let summary_controls = controls.clone();
                when(
                    move || compose.composing.get(),
                    move || {
                        view()
                            .pl_3p5()
                            .pr_5()
                            .py_3p5()
                            .gap_3p5()
                            .child(compose_box(
                                compose,
                                compose_controls.clone(),
                                selected,
                                paused,
                            ))
                    },
                )
                .otherwise(move || {
                    let seq_controls = seq_controls.clone();
                    let summary_controls = summary_controls.clone();
                    when(
                        move || seq_builder.open.get(),
                        move || {
                            view()
                                .pl_3p5()
                                .pr_5()
                                .py_3p5()
                                .gap_3p5()
                                .child(sequence_diagram_panel(
                                    seq_builder,
                                    seq_controls.clone(),
                                    selected,
                                    compose,
                                ))
                        },
                    )
                    .otherwise(move || {
                        let redact_controls = summary_controls.clone();
                        let summary_controls = summary_controls.clone();
                        when(
                            move || seq_builder.redact_open.get(),
                            move || {
                                view()
                                    .pl_3p5()
                                    .pr_5()
                                    .py_3p5()
                                    .gap_3p5()
                                    .child(redact_preview_panel(
                                        seq_builder,
                                        redact_controls.clone(),
                                        rows,
                                        effective_filter,
                                    ))
                            },
                        )
                        .otherwise(move || {
                            let summary_controls = summary_controls.clone();
                            view()
                                .flex_grow(1.0)
                                .flex_shrink_0()
                                .pl_3p5()
                                .pr_5()
                                .py_3p5()
                                .gap_3p5()
                                .child(when(
                                    move || filtered_stats.with(|s| s.total_count > 0),
                                    move || {
                                        let controls = summary_controls.clone();
                                        filtered_summary_card(filtered_stats, selected, move || {
                                            begin_sequence_diagram(
                                                &controls,
                                                rows,
                                                seq_builder,
                                                compose,
                                                selected,
                                                shown.phase,
                                            );
                                        })
                                    },
                                ))
                                .child(
                                    view()
                                        .flex_grow(1.0)
                                        .flex_shrink_0()
                                        .items_center()
                                        .justify_center()
                                        .gap_1p5()
                                        .px_6()
                                        .py_8()
                                        .child(glyph_px(
                                            28.0,
                                            || Glyph::List,
                                            || color(fg_muted()),
                                        ))
                                        .child(
                                            text({
                                                let langs = ui_langs();
                                                move || -> SharedString {
                                                    let lang = ui_lang(langs);
                                                    match shown.phase.get() {
                                                        Phase::Gone => t(
                                                            lang,
                                                            "这条记录已经不在了",
                                                            "That flow is gone",
                                                        )
                                                        .into(),
                                                        _ => t(
                                                            lang,
                                                            "还没有选择",
                                                            "Nothing selected",
                                                        )
                                                        .into(),
                                                    }
                                                }
                                            })
                                            .text_center()
                                            .text_size(px(13.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(fg_secondary()),
                                        )
                                        .child(
                                            text({
                                                let langs = ui_langs();
                                                move || -> SharedString {
                                                    let lang = ui_lang(langs);
                                                    match shown.phase.get() {
                                                        Phase::Gone => t(
                                                            lang,
                                                            "它因为内存上限被丢掉了。",
                                                            "It was dropped because the session hit its memory cap.",
                                                        )
                                                        .into(),
                                                        _ => t(
                                                            lang,
                                                            "从左边选一条流量，查看请求头、响应头和 body。",
                                                            "Pick a flow on the left to see headers and the body.",
                                                        )
                                                        .into(),
                                                    }
                                                }
                                            })
                                            .text_center()
                                            .text_size(px(12.0))
                                            .text_color(fg_muted()),
                                        )
                                        .child(
                                            view()
                                                .mt_1p5()
                                                .h(px(28.0))
                                                .px_3()
                                                .rounded(px(6.0))
                                                .border_1()
                                                .border_color(border_control())
                                                .bg(bg_control())
                                                .hover(|style| style.bg(bg_control_hover()))
                                                .active(|style| style.opacity(0.8))
                                                .cursor(Cursor::Pointer)
                                                .flex_row()
                                                .items_center()
                                                .gap_1p5()
                                                .on_press(move || begin_new_request(compose, selected))
                                                .child(glyph_px(
                                                    13.0,
                                                    || Glyph::Prompt,
                                                    || color(fg_secondary()),
                                                ))
                                                .child(
                                                    text(tr("构造请求", "New request"))
                                                        .number_of_lines(1)
                                                        .text_size(px(12.0))
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(fg_primary()),
                                                ),
                                        ),
                                )
                        })
                    })
                })
            }),
        )
}

fn filtered_summary_card(
    stats: Signal<session::FilteredStats>,
    selected: Signal<Option<u64>>,
    on_seq: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .p_3p5()
        .gap_3()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .child(section_title(tr("当前列表统计", "Filtered summary")))
                .child(detail_button(tr("生成时序图", "Sequence diagram"), on_seq)),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2p5()
                .child(summary_stat_tile(
                    tr("总请求", "Total requests"),
                    move || stats.with(|s| s.total_count.to_string()),
                ))
                .child(summary_stat_tile(
                    tr("总流量", "Total transfer"),
                    move || {
                        stats.with(|s| {
                            format!(
                                "{} (↑{} · ↓{})",
                                present::bytes(s.total_bytes),
                                present::bytes(s.total_req_bytes),
                                present::bytes(s.total_res_bytes)
                            )
                        })
                    },
                ))
                .child(summary_stat_tile(
                    tr("平均耗时", "Avg latency"),
                    move || {
                        stats.with(|s| match s.avg_elapsed_ms {
                            Some(ms) => format!("{ms} ms"),
                            None => "—".to_string(),
                        })
                    },
                )),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_1p5()
                .child(status_count_badge(
                    "2xx",
                    move || stats.with(|s| s.s2xx),
                    status_ok_bg(),
                    status_ok_fg(),
                ))
                .child(status_count_badge(
                    "3xx",
                    move || stats.with(|s| s.s3xx),
                    status_warn_bg(),
                    status_warn_fg(),
                ))
                .child(status_count_badge(
                    "4xx",
                    move || stats.with(|s| s.s4xx),
                    status_err_bg(),
                    status_err_fg(),
                ))
                .child(status_count_badge(
                    "5xx",
                    move || stats.with(|s| s.s5xx),
                    status_err_bg(),
                    status_err_fg(),
                ))
                .child(status_count_badge(
                    "ERR",
                    move || stats.with(|s| s.failed),
                    status_err_bg(),
                    status_err_fg(),
                ))
                .child({
                    let langs = ui_langs();
                    view()
                        .h(px(22.0))
                        .px_2()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.0))
                        .bg(bg_control())
                        .child(
                            mono(move || {
                                format!(
                                    "{} {}",
                                    t(ui_lang(langs), "进行中", "Pending"),
                                    stats.with(|s| s.pending)
                                )
                            })
                            .tabular_nums()
                            .number_of_lines(1)
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg_muted()),
                        )
                }),
        )
        .child(when(
            move || stats.with(|s| s.max_elapsed.is_some()),
            move || {
                view()
                    .px_3()
                    .py_2()
                    .rounded(px(7.0))
                    .border_1()
                    .border_color(border_subtle())
                    .bg(bg_code())
                    .hover(|style| style.bg(bg_control_hover()))
                    .active(|style| style.opacity(0.8))
                    .cursor(Cursor::Pointer)
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .on_press(move || {
                        if let Some(id) =
                            stats.with(|s| s.max_elapsed.as_ref().map(|(id, _, _)| *id))
                        {
                            selected.set(Some(id));
                        }
                    })
                    .child(
                        text(tr("最慢请求", "Slowest"))
                            .flex_shrink_0()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg_muted()),
                    )
                    .child(
                        mono(move || {
                            stats.with(|s| match &s.max_elapsed {
                                Some((id, ms, target)) => format!("#{id} · {ms} ms · {target}"),
                                None => String::new(),
                            })
                        })
                        .tabular_nums()
                        .flex_1()
                        .min_w(px(0.0))
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Middle)
                        .text_size(px(12.0))
                        .text_color(accent_blue_text()),
                    )
            },
        ))
}

fn summary_stat_tile(
    label: impl IntoReactive<SharedString>,
    value: impl Fn() -> String + 'static,
) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(128.0))
        .px_3()
        .py_2()
        .gap_0p5()
        .rounded(px(7.0))
        .border_1()
        .border_color(border_subtle())
        .bg(bg_code())
        .child(
            text(label)
                .number_of_lines(1)
                .text_size(px(11.0))
                .text_color(fg_muted()),
        )
        .child(
            mono(value)
                .tabular_nums()
                .number_of_lines(1)
                .text_overflow(EllipsizeMode::Tail)
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_primary()),
        )
}

fn status_count_badge(
    tag: &'static str,
    count: impl Fn() -> usize + 'static,
    bg_color: AdaptiveColor,
    fg_color: AdaptiveColor,
) -> impl IntoElement {
    view()
        .h(px(22.0))
        .px_2()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .bg(bg_color)
        .child(
            mono(move || format!("{tag} {}", count()))
                .tabular_nums()
                .number_of_lines(1)
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_color),
        )
}

#[allow(clippy::too_many_arguments)]
fn ready_detail(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
    compose: Compose,
    filter: Signal<String>,
    rules_text: Signal<String>,
    baseline: Signal<Option<u64>>,
    folded: Signal<u8>,
) -> impl IntoElement {
    let no_form = signal(Vec::<HeaderLine>::new());
    view()
        .pl_3p5()
        .pr_5()
        .py_3p5()
        .gap_3p5()
        .child(summary(
            shown,
            controls.clone(),
            selected,
            paused,
            compose,
            filter,
            rules_text,
            baseline,
        ))
        .child(when(move || compose.composing.get(), {
            let controls = controls.clone();
            move || compose_box(compose, controls.clone(), selected, paused)
        }))
        .child(when(
            move || {
                !shown.response_body.get().is_empty()
                    || !shown.response_note.get().is_empty()
                    || (shown.pretty.get() && !shown.preview.get().is_empty())
            },
            {
                let controls = controls.clone();
                move || {
                    body_section(
                        "响应 body",
                        "Response body",
                        shown.response_body,
                        shown.response_hex,
                        shown.response_note,
                        shown.response_language,
                        shown.response_doc,
                        no_form,
                        shown.res_search,
                        true,
                        shown,
                        controls.clone(),
                        selected,
                        baseline,
                        folded,
                        FOLD_RES_BODY,
                    )
                }
            },
        ))
        .child(header_section(
            "响应头",
            "Response headers",
            shown.response_headers,
            shown.pane_width,
            folded,
            FOLD_RES_HEADERS,
        ))
        .child(when(
            move || {
                !shown.request_body.get().is_empty()
                    || !shown.request_note.get().is_empty()
                    || !shown.request_form.with(Vec::is_empty)
            },
            {
                let controls = controls.clone();
                move || {
                    body_section(
                        "请求 body",
                        "Request body",
                        shown.request_body,
                        shown.request_hex,
                        shown.request_note,
                        shown.request_language,
                        shown.request_doc,
                        shown.request_form,
                        shown.req_search,
                        false,
                        shown,
                        controls.clone(),
                        selected,
                        baseline,
                        folded,
                        FOLD_REQ_BODY,
                    )
                }
            },
        ))
        .child(when(
            move || !shown.query.with(Vec::is_empty),
            move || {
                header_section(
                    "查询参数",
                    "Query",
                    shown.query,
                    shown.pane_width,
                    folded,
                    FOLD_QUERY,
                )
            },
        ))
        .child(header_section(
            "请求头",
            "Request headers",
            shown.request_headers,
            shown.pane_width,
            folded,
            FOLD_REQ_HEADERS,
        ))
        .child(when(
            move || !shown.diff.get().is_empty(),
            move || diff_card(shown, folded),
        ))
        .child(when(
            move || !shown.messages.with(Vec::is_empty),
            move || messages_section(shown, controls.clone(), folded),
        ))
}

fn summary(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
    compose: Compose,
    filter: Signal<String>,
    rules_text: Signal<String>,
    baseline: Signal<Option<u64>>,
) -> impl IntoElement {
    view()
        .p_3p5()
        .gap_2p5()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .child(
            view()
                .w_full()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(
                    view()
                        .px_2()
                        .py_0p5()
                        .rounded(px(5.0))
                        .bg(move || method_bg(&shown.method.get()))
                        .child(
                            mono(move || shown.method.get())
                                .text_size(px(11.5))
                                .font_weight(FontWeight::BOLD)
                                .text_color(move || method_fg(&shown.method.get())),
                        ),
                )
                .child(
                    view()
                        .px_2()
                        .py_0p5()
                        .rounded(px(5.0))
                        .bg(move || {
                            http_status_bg(
                                shown.status.get(),
                                !shown.error.get().is_empty() && shown.status.get().is_none(),
                            )
                        })
                        .child(
                            mono({
                                let langs = ui_langs();
                                move || match shown.status.get() {
                                    Some(code) => code.to_string(),
                                    None if !shown.error.get().is_empty() => "ERR".to_string(),
                                    None => t(ui_lang(langs), "等待响应", "Waiting").into(),
                                }
                            })
                            .tabular_nums()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || {
                                http_status_fg(
                                    shown.status.get(),
                                    !shown.error.get().is_empty() && shown.status.get().is_none(),
                                )
                            }),
                        ),
                )
                .child(
                    view()
                        .px_2()
                        .py_0p5()
                        .rounded(px(5.0))
                        .bg(bg_control())
                        .child(
                            mono(move || shown.protocol.get())
                                .text_size(px(11.5))
                                .text_color(fg_muted()),
                        ),
                )
                .child(when(
                    move || !shown.rules.get().is_empty(),
                    move || {
                        view()
                            .max_w(px(180.0))
                            .overflow_hidden()
                            .px_2()
                            .py_0p5()
                            .rounded(px(5.0))
                            .bg(accent_blue_soft())
                            .child(
                                mono(move || shown.rules.get())
                                    .number_of_lines(1)
                                    .text_overflow(EllipsizeMode::Tail)
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(accent_blue_text()),
                            )
                    },
                ))
                .child(when(
                    move || !shown.comment.get().is_empty() && !shown.editing_comment.get(),
                    move || {
                        view()
                            .max_w(px(180.0))
                            .overflow_hidden()
                            .px_2()
                            .py_0p5()
                            .rounded(px(5.0))
                            .border_1()
                            .border_color(accent_blue_border())
                            .bg(accent_blue_soft())
                            .hover(|style| style.bg(accent_blue_soft_hover()))
                            .active(|style| style.opacity(0.8))
                            .cursor(Cursor::Pointer)
                            .on_press(move || {
                                shown.repeating.set(false);
                                shown.editing_comment.set(true);
                            })
                            .child(
                                text(move || format!("# {}", shown.comment.get()))
                                    .number_of_lines(1)
                                    .text_overflow(EllipsizeMode::Tail)
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(accent_blue_text()),
                            )
                    },
                ))
                .child(view().flex_1())
                .child(detail_actions(
                    shown,
                    controls.clone(),
                    selected,
                    paused,
                    compose,
                    filter,
                    rules_text,
                    baseline,
                )),
        )
        .child(when(
            move || !shown.relay.get().is_empty(),
            move || {
                let langs = ui_langs();
                text(move || {
                    let short = shown.relay.get();
                    format!(
                        "{} {} {short}",
                        t(ui_lang(langs), "来源", "From"),
                        t(ui_lang(langs), "中继", "relay")
                    )
                })
                .number_of_lines(1)
                .text_size(px(12.0))
                .text_color(fg_secondary())
            },
        ))
        .child(
            view()
                .w_full()
                .overflow_hidden()
                .px_3()
                .py_2()
                .rounded(px(7.0))
                .bg(bg_code())
                .border_1()
                .border_color(border_subtle())
                .on_press_up(dismiss_keyboard)
                .child(
                    mono(move || shown.url.get())
                        .selectable(true)
                        .text_size(px(12.5))
                        .text_color(fg_primary()),
                ),
        )
        .child(when(move || shown.repeating.get(), {
            let controls = controls.clone();
            move || repeat_inline_bar(shown, controls.clone(), selected)
        }))
        .child(when(move || shown.editing_comment.get(), {
            let controls = controls.clone();
            move || {
                let save_controls = controls.clone();
                let submit_controls = controls.clone();
                let clear_controls = controls.clone();
                view()
                    .flex_row()
                    .items_center()
                    .gap_1p5()
                    .child(
                        view().flex_1().min_w(px(0.0)).child(
                            compose_input_on(
                                shown.comment,
                                tr(
                                    "添加流量备注（可用 note: 搜索）…",
                                    "Add flow note (searchable via note:)…",
                                ),
                                26.0,
                                false,
                                move |text| shown.comment.set(text.to_string()),
                            )
                            .on_submit(move |text: &str| {
                                shown.comment.set(text.to_string());
                                save_flow_comment(&submit_controls, selected.get(), shown);
                            }),
                        ),
                    )
                    .child(detail_button(tr("保存", "Save"), move || {
                        save_flow_comment(&save_controls, selected.get(), shown);
                    }))
                    .child(detail_button(tr("清除", "Clear"), move || {
                        shown.comment.set(String::new());
                        save_flow_comment(&clear_controls, selected.get(), shown);
                    }))
            }
        }))
        .child(
            mono(move || shown.metrics.get())
                .tabular_nums()
                .selectable(true)
                .on_press_up(dismiss_keyboard)
                .number_of_lines(1)
                .text_overflow(EllipsizeMode::Tail)
                .text_size(px(11.5))
                .text_color(fg_muted()),
        )
        .child(when(
            move || shown.timing_bar.with(present::TimingBreakdown::has_content),
            move || summary_timing_bar(shown.timing_bar),
        ))
        .child(when(
            move || !shown.error.get().is_empty(),
            move || {
                view()
                    .w_full()
                    .overflow_hidden()
                    .px_3()
                    .py_2()
                    .rounded(px(7.0))
                    .bg(status_err_bg())
                    .border_1()
                    .border_color(status_err_fg())
                    .child(
                        text({
                            let langs = ui_langs();
                            move || localize_error(ui_lang(langs), &shown.error.get())
                        })
                        .selectable(true)
                        .on_press_up(dismiss_keyboard)
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(status_err_fg()),
                    )
            },
        ))
}

fn phase_color_dns() -> AdaptiveColor {
    AdaptiveColor::rgb(0x0284c7, 0x38bdf8)
}

fn phase_color_tcp() -> AdaptiveColor {
    AdaptiveColor::rgb(0xd97706, 0xfbbf24)
}

fn phase_color_tls() -> AdaptiveColor {
    AdaptiveColor::rgb(0x7c3aed, 0xa78bfa)
}

fn phase_color_wait() -> AdaptiveColor {
    AdaptiveColor::rgb(0x059669, 0x34d399)
}

fn phase_color_download() -> AdaptiveColor {
    AdaptiveColor::rgb(0x2563eb, 0x60a5fa)
}

fn summary_timing_bar(timing: Signal<present::TimingBreakdown>) -> impl IntoElement {
    view()
        .px_2p5()
        .py_2()
        .gap_1p5()
        .rounded(px(7.0))
        .bg(bg_code())
        .border_1()
        .border_color(border_subtle())
        .child(
            view()
                .w_full()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_1p5()
                        .child(when(
                            move || timing.with(|tb| tb.reused),
                            || {
                                view()
                                    .px_1p5()
                                    .py(px(1.5))
                                    .rounded(px(4.0))
                                    .bg(status_ok_bg())
                                    .child(
                                        text(tr("复用连接", "Reused"))
                                            .text_size(px(10.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(status_ok_fg()),
                                    )
                            },
                        ))
                        .child(when(
                            move || {
                                timing.with(|tb| {
                                    !tb.reused
                                        && (tb.dns_ms.is_some()
                                            || tb.tcp_ms.is_some()
                                            || tb.tls_ms.is_some())
                                })
                            },
                            || {
                                view()
                                    .px_1p5()
                                    .py(px(1.5))
                                    .rounded(px(4.0))
                                    .bg(accent_blue_soft())
                                    .child(
                                        text(tr("新建连接", "New conn"))
                                            .text_size(px(10.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(accent_blue_text()),
                                    )
                            },
                        ))
                        .child(when(
                            move || timing.with(|tb| !tb.tls_version.is_empty()),
                            move || {
                                view()
                                    .px_1p5()
                                    .py(px(1.5))
                                    .rounded(px(4.0))
                                    .bg(bg_control())
                                    .child(
                                        mono(move || timing.with(|tb| tb.tls_version.clone()))
                                            .text_size(px(10.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(fg_secondary()),
                                    )
                            },
                        ))
                        .child(when(
                            move || timing.with(|tb| !tb.server_addr.is_empty()),
                            move || {
                                view()
                                    .px_1p5()
                                    .py(px(1.5))
                                    .rounded(px(4.0))
                                    .bg(bg_control())
                                    .child(
                                        mono({
                                            let langs = ui_langs();
                                            move || {
                                                let addr = timing.with(|tb| tb.server_addr.clone());
                                                format!(
                                                    "{} {addr}",
                                                    t(ui_lang(langs), "远端", "Peer")
                                                )
                                            }
                                        })
                                        .selectable(true)
                                        .text_size(px(10.5))
                                        .text_color(fg_muted()),
                                    )
                            },
                        )),
                )
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_2p5()
                        .child(timing_phase_chip(
                            "DNS",
                            "DNS",
                            phase_color_dns(),
                            move || timing.with(|tb| tb.dns_ms),
                        ))
                        .child(timing_phase_chip(
                            "TCP",
                            "TCP",
                            phase_color_tcp(),
                            move || timing.with(|tb| tb.tcp_ms),
                        ))
                        .child(timing_phase_chip(
                            "TLS",
                            "TLS",
                            phase_color_tls(),
                            move || timing.with(|tb| tb.tls_ms),
                        ))
                        .child(timing_phase_chip(
                            "等待",
                            "Wait",
                            phase_color_wait(),
                            move || timing.with(|tb| tb.wait_ms),
                        ))
                        .child(timing_phase_chip(
                            "下载",
                            "Download",
                            phase_color_download(),
                            move || timing.with(|tb| tb.download_ms),
                        )),
                ),
        )
        .child(when(
            move || timing.with(present::TimingBreakdown::has_phases),
            move || {
                view()
                    .w_full()
                    .h(px(5.0))
                    .rounded(px(2.5))
                    .bg(bg_control())
                    .overflow_hidden()
                    .flex_row()
                    .gap(px(1.0))
                    .child(timing_phase_segment(phase_color_dns(), move || {
                        timing.with(|tb| tb.dns_ms)
                    }))
                    .child(timing_phase_segment(phase_color_tcp(), move || {
                        timing.with(|tb| tb.tcp_ms)
                    }))
                    .child(timing_phase_segment(phase_color_tls(), move || {
                        timing.with(|tb| tb.tls_ms)
                    }))
                    .child(timing_phase_segment(phase_color_wait(), move || {
                        timing.with(|tb| tb.wait_ms)
                    }))
                    .child(timing_phase_segment(phase_color_download(), move || {
                        timing.with(|tb| tb.download_ms)
                    }))
            },
        ))
}

fn timing_phase_chip(
    zh: &'static str,
    en: &'static str,
    color: AdaptiveColor,
    get_ms: impl Fn() -> Option<u64> + Copy + 'static,
) -> impl IntoElement {
    when(
        move || get_ms().is_some(),
        move || {
            let langs = ui_langs();
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(view().w(px(6.5)).h(px(6.5)).rounded_full().bg(color))
                .child(
                    mono(move || {
                        let ms = get_ms().unwrap_or(0);
                        format!("{} {ms} ms", t(ui_lang(langs), zh, en))
                    })
                    .tabular_nums()
                    .text_size(px(11.0))
                    .text_color(fg_secondary()),
                )
        },
    )
}

fn timing_phase_segment(
    color: AdaptiveColor,
    get_ms: impl Fn() -> Option<u64> + Copy + 'static,
) -> impl IntoElement {
    when(
        move || get_ms().is_some(),
        move || {
            view()
                .h_full()
                .min_w(px(3.0))
                .flex_grow(move || (get_ms().unwrap_or(0).max(1)) as f32)
                .bg(color)
        },
    )
}

fn repeat_inline_bar(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
) -> impl IntoElement {
    let submit_count_controls = controls.clone();
    let submit_interval_controls = controls.clone();
    let submit_concurrency_controls = controls.clone();
    let run_controls = controls;
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_1p5()
        .px_3()
        .py_2()
        .rounded(px(7.0))
        .bg(bg_code())
        .border_1()
        .border_color(border_subtle())
        .child(
            text(tr("次数", "Count"))
                .number_of_lines(1)
                .text_size(px(11.5))
                .text_color(fg_secondary()),
        )
        .child(
            view().w(px(56.0)).flex_shrink_0().child(
                compose_input_on(shown.repeat_count, "5", 26.0, false, move |text| {
                    shown.repeat_count.set(text.to_string())
                })
                .on_submit(move |text: &str| {
                    shown.repeat_count.set(text.to_string());
                    repeat_flow_batch(&submit_count_controls, selected.get(), shown);
                }),
            ),
        )
        .child(repeat_preset_chip(shown.repeat_count, "5", "5×"))
        .child(repeat_preset_chip(shown.repeat_count, "10", "10×"))
        .child(repeat_preset_chip(shown.repeat_count, "20", "20×"))
        .child(
            text(tr("并发", "Concurrency"))
                .pl_1()
                .number_of_lines(1)
                .text_size(px(11.5))
                .text_color(fg_secondary()),
        )
        .child(
            view().w(px(48.0)).flex_shrink_0().child(
                compose_input_on(shown.repeat_concurrency, "1", 26.0, false, move |text| {
                    shown.repeat_concurrency.set(text.to_string())
                })
                .on_submit(move |text: &str| {
                    shown.repeat_concurrency.set(text.to_string());
                    repeat_flow_batch(&submit_concurrency_controls, selected.get(), shown);
                }),
            ),
        )
        .child(
            text(tr("间隔 ms", "Interval (ms)"))
                .pl_1()
                .number_of_lines(1)
                .text_size(px(11.5))
                .text_color(fg_secondary()),
        )
        .child(
            view().w(px(68.0)).flex_shrink_0().child(
                compose_input_on(shown.repeat_interval, "200", 26.0, false, move |text| {
                    shown.repeat_interval.set(text.to_string())
                })
                .on_submit(move |text: &str| {
                    shown.repeat_interval.set(text.to_string());
                    repeat_flow_batch(&submit_interval_controls, selected.get(), shown);
                }),
            ),
        )
        .child(view().flex_1())
        .child(
            view()
                .h(px(26.0))
                .px_2p5()
                .rounded(px(6.0))
                .bg(accent_blue())
                .hover(|style| style.bg(accent_blue_hover()))
                .active(|style| style.opacity(0.85))
                .cursor(Cursor::Pointer)
                .flex_row()
                .items_center()
                .on_press(move || {
                    repeat_flow_batch(&run_controls, selected.get(), shown);
                })
                .child(
                    text(tr("开始重放", "Run"))
                        .number_of_lines(1)
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(white()),
                ),
        )
        .child(detail_button(tr("取消", "Cancel"), move || {
            shown.repeating.set(false);
        }))
}

fn repeat_preset_chip(
    count_sig: Signal<String>,
    val: &'static str,
    label: &'static str,
) -> impl IntoElement {
    let is_active = move || count_sig.get().trim() == val;
    view()
        .h(px(24.0))
        .px_2()
        .rounded(px(5.0))
        .border_1()
        .border_color(move || {
            if is_active() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if is_active() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if is_active() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .flex_row()
        .items_center()
        .on_press(move || count_sig.set(val.to_string()))
        .child(
            mono(label)
                .tabular_nums()
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || {
                    if is_active() {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

fn card_folded(folded: Signal<u8>, bit: u8) -> bool {
    folded.get() & bit != 0
}

fn toggle_card(folded: Signal<u8>, bit: u8) {
    folded.set(folded.get() ^ bit);
}

fn fold_mark(folded: Signal<u8>, bit: u8) -> impl IntoElement {
    glyph(
        move || {
            if card_folded(folded, bit) {
                Glyph::ChevronRight
            } else {
                Glyph::Chevron
            }
        },
        || color(fg_muted()),
    )
}

fn kv_key_width(pane_w: f32) -> f32 {
    if pane_w > 0.0 {
        (pane_w * 0.32).round().clamp(108.0, 176.0)
    } else {
        176.0
    }
}

fn kv_value_cell(line: ReadSignal<HeaderLine>) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(0.0))
        .gap_1()
        .child(
            mono(move || line.with(|line| line.value.clone()))
                .selectable(true)
                .text_size(px(12.0))
                .text_color(fg_primary()),
        )
        .child(when(
            move || line.with(|line| line.hint.is_some()),
            move || {
                view()
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .bg(bg_control())
                    .border_1()
                    .border_color(border_subtle())
                    .child(
                        mono(move || line.with(|line| line.hint.clone().unwrap_or_default()))
                            .selectable(true)
                            .text_size(px(11.0))
                            .text_color(fg_secondary()),
                    )
            },
        ))
}

fn header_section(
    zh: &'static str,
    en: &'static str,
    lines: Signal<Vec<HeaderLine>>,
    pane_width: Signal<f32>,
    folded: Signal<u8>,
    bit: u8,
) -> impl IntoElement {
    view()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .overflow_hidden()
        .child(
            view()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_1p5()
                .px_3p5()
                .py_2()
                .border_b(move || {
                    if card_folded(folded, bit) {
                        px(0.0)
                    } else {
                        px(1.0)
                    }
                })
                .border_b_color(border_subtle())
                .hover(|style| style.bg(bg_control_hover()))
                .active(|style| style.opacity(0.85))
                .cursor(Cursor::Pointer)
                .on_press(move || toggle_card(folded, bit))
                .child(fold_mark(folded, bit))
                .child(section_title(tr(zh, en)))
                .child(
                    view()
                        .px_1p5()
                        .py(px(1.0))
                        .rounded(px(4.0))
                        .bg(bg_control())
                        .child(
                            mono(move || format!("{}", lines.with(Vec::len)))
                                .tabular_nums()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(fg_muted()),
                        ),
                ),
        )
        .child(when(
            move || !card_folded(folded, bit),
            move || {
                when(
                    move || lines.with(Vec::is_empty),
                    || view().px_3p5().py_2p5().child(muted_line(tr("无", "None"))),
                )
                .otherwise(move || {
                    each(
                        lines,
                        |line| line.key,
                        move |line, index| {
                            view()
                                .flex_row()
                                .gap_3()
                                .px_3p5()
                                .py_2()
                                .bg(move || {
                                    if index.get() % 2 == 1 {
                                        bg_code()
                                    } else {
                                        bg_card()
                                    }
                                })
                                .border_t(move || if index.get() > 0 { px(1.0) } else { px(0.0) })
                                .border_t_color(border_subtle())
                                .on_press_up(dismiss_keyboard)
                                .child(
                                    mono(move || line.with(|line| line.name.clone()))
                                        .w(move || px(kv_key_width(pane_width.get())))
                                        .flex_shrink_0()
                                        .selectable(true)
                                        .number_of_lines(1)
                                        .text_overflow(EllipsizeMode::Tail)
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(fg_muted()),
                                )
                                .child(kv_value_cell(line))
                        },
                    )
                })
            },
        ))
}

fn filter_message_lines(lines: &[HeaderLine], query: &str, dir: u8, kind: u8) -> Vec<HeaderLine> {
    if query.trim().is_empty() && dir == 0 && kind == 0 {
        return lines.to_vec();
    }
    lines
        .iter()
        .filter(|line| present::message_matches_filter(&line.name, &line.value, query, dir, kind))
        .cloned()
        .collect()
}

fn msg_filter_chip(
    state: Signal<u8>,
    val: u8,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    let active = move || state.get() == val;
    view()
        .h(px(22.0))
        .px_2()
        .flex_row()
        .items_center()
        .bg(move || if active() { accent_blue() } else { bg_card() })
        .hover(|style| {
            style.bg(move || {
                if active() {
                    accent_blue_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .on_press(move || state.set(val))
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if active() {
                        AdaptiveColor::rgb(0xffffff, 0xffffff)
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

fn msg_search_box(
    search: Signal<String>,
    lines: Signal<Vec<HeaderLine>>,
    dir: Signal<u8>,
    kind: Signal<u8>,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(
            input()
                .value(search)
                .placeholder(tr("搜索…", "Search…"))
                .placeholder_color(move || color(fg_muted()))
                .w(px(120.0))
                .h(px(22.0))
                .px_2()
                .rounded(px(6.0))
                .bg(bg_code())
                .border_1()
                .border_color(move || {
                    if search.get().trim().is_empty() {
                        border_control()
                    } else {
                        accent_blue_border()
                    }
                })
                .focus(|style| style.border_color(accent_blue()))
                .text_size(px(11.5))
                .text_color(fg_primary())
                .on_change(move |text: &str| search.set(text.to_string())),
        )
        .child(when(
            move || !search.get().trim().is_empty(),
            move || {
                view()
                    .h(px(22.0))
                    .px_1p5()
                    .rounded(px(6.0))
                    .bg(accent_blue_soft())
                    .hover(|style| style.bg(accent_blue_soft_hover()))
                    .active(|style| style.opacity(0.8))
                    .border_1()
                    .border_color(accent_blue_border())
                    .flex_row()
                    .items_center()
                    .cursor(Cursor::Pointer)
                    .tooltip(label_tip("点击清空搜索", "Click to clear search"))
                    .on_press(move || search.set(String::new()))
                    .child(
                        mono(move || {
                            let matched = filter_message_lines(
                                &lines.get(),
                                &search.get(),
                                dir.get(),
                                kind.get(),
                            )
                            .len();
                            format!("{matched} ×")
                        })
                        .tabular_nums()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(accent_blue_text()),
                    )
            },
        ))
}

fn messages_section(shown: Shown, controls: Arc<Controls>, folded: Signal<u8>) -> impl IntoElement {
    let lines = shown.messages;
    let search = shown.msg_search;
    let dir = shown.msg_dir;
    let kind = shown.msg_kind;
    let pane_width = shown.pane_width;
    let copy_all_controls = controls.clone();
    let row_controls = controls;
    let has_ws_dirs =
        move || lines.with(|items| items.iter().any(|line| present::is_ws_message(&line.name)));
    view()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .overflow_hidden()
        .child(
            view()
                .flex_shrink_0()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .px_3p5()
                .py_2()
                .border_b(move || {
                    if card_folded(folded, FOLD_MESSAGES) {
                        px(0.0)
                    } else {
                        px(1.0)
                    }
                })
                .border_b_color(border_subtle())
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .px_1()
                        .py_0p5()
                        .rounded(px(5.0))
                        .hover(|style| style.bg(bg_control_hover()))
                        .active(|style| style.opacity(0.85))
                        .cursor(Cursor::Pointer)
                        .on_press(move || toggle_card(folded, FOLD_MESSAGES))
                        .child(fold_mark(folded, FOLD_MESSAGES))
                        .child(section_title(tr("消息", "Messages")))
                        .child(
                            view()
                                .px_1p5()
                                .py(px(1.0))
                                .rounded(px(4.0))
                                .bg(bg_control())
                                .child(
                                    mono(move || {
                                        let total = lines.with(Vec::len);
                                        if search.get().trim().is_empty()
                                            && dir.get() == 0
                                            && kind.get() == 0
                                        {
                                            format!("{total}")
                                        } else {
                                            let matched = filter_message_lines(
                                                &lines.get(),
                                                &search.get(),
                                                dir.get(),
                                                kind.get(),
                                            )
                                            .len();
                                            format!("{matched}/{total}")
                                        }
                                    })
                                    .tabular_nums()
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(fg_muted()),
                                ),
                        ),
                )
                .child(when(
                    move || !card_folded(folded, FOLD_MESSAGES),
                    move || {
                        let copy_all_controls = copy_all_controls.clone();
                        view()
                            .flex_row()
                            .flex_wrap()
                            .items_center()
                            .gap_1p5()
                            .child(when(has_ws_dirs, move || {
                                view()
                                    .flex_row()
                                    .h(px(22.0))
                                    .rounded(px(6.0))
                                    .border_1()
                                    .border_color(border_control())
                                    .overflow_hidden()
                                    .child(msg_filter_chip(dir, 0, "全部", "All"))
                                    .child(msg_filter_chip(dir, 1, "↑ 客户端", "↑ Client"))
                                    .child(msg_filter_chip(dir, 2, "↓ 源站", "↓ Origin"))
                            }))
                            .child(when(has_ws_dirs, move || {
                                view()
                                    .flex_row()
                                    .h(px(22.0))
                                    .rounded(px(6.0))
                                    .border_1()
                                    .border_color(border_control())
                                    .overflow_hidden()
                                    .child(msg_filter_chip(kind, 0, "全部", "All"))
                                    .child(msg_filter_chip(kind, 1, "文本", "Text"))
                                    .child(msg_filter_chip(kind, 2, "二进制", "Binary"))
                                    .child(msg_filter_chip(kind, 3, "控制", "Control"))
                            }))
                            .child(msg_search_box(search, lines, dir, kind))
                            .child(when(
                                move || {
                                    filter_message_lines(
                                        &lines.get(),
                                        &search.get(),
                                        dir.get(),
                                        kind.get(),
                                    )
                                    .iter()
                                    .any(|line| line.name != "…")
                                },
                                move || {
                                    let controls = copy_all_controls.clone();
                                    detail_button(tr("复制全部", "Copy all"), move || {
                                        let matched = filter_message_lines(
                                            &lines.get(),
                                            &search.get(),
                                            dir.get(),
                                            kind.get(),
                                        );
                                        let payload = matched
                                            .into_iter()
                                            .filter(|line| line.name != "…")
                                            .map(|line| format!("[{}]\n{}", line.name, line.value))
                                            .collect::<Vec<_>>()
                                            .join("\n\n");
                                        if !payload.is_empty() {
                                            set_clipboard(payload);
                                            set_status(
                                                &controls,
                                                StatusMsg::CopiedBody {
                                                    side: Side::Response,
                                                    hex: false,
                                                },
                                            );
                                        }
                                    })
                                },
                            ))
                    },
                )),
        )
        .child(when(
            move || !card_folded(folded, FOLD_MESSAGES),
            move || {
                let row_controls = row_controls.clone();
                let filtered = move || {
                    filter_message_lines(&lines.get(), &search.get(), dir.get(), kind.get())
                };
                when(
                    move || filtered().is_empty(),
                    || view().px_3p5().py_2p5().child(muted_line(tr("无", "None"))),
                )
                .otherwise(move || {
                    let row_controls = row_controls.clone();
                    each(
                        filtered,
                        |line| line.key,
                        move |line, index| {
                            let controls = row_controls.clone();
                            view()
                                .flex_row()
                                .items_start()
                                .gap_3()
                                .px_3p5()
                                .py_2()
                                .bg(move || {
                                    if index.get() % 2 == 1 {
                                        bg_code()
                                    } else {
                                        bg_card()
                                    }
                                })
                                .border_t(move || if index.get() > 0 { px(1.0) } else { px(0.0) })
                                .border_t_color(border_subtle())
                                .on_press_up(dismiss_keyboard)
                                .child(
                                    mono(move || line.with(|line| line.name.clone()))
                                        .w(move || px(kv_key_width(pane_width.get())))
                                        .flex_shrink_0()
                                        .selectable(true)
                                        .number_of_lines(1)
                                        .text_overflow(EllipsizeMode::Tail)
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(fg_muted()),
                                )
                                .child(
                                    mono(move || line.with(|line| line.value.clone()))
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .selectable(true)
                                        .text_size(px(12.0))
                                        .text_color(fg_primary()),
                                )
                                .child(when(
                                    move || line.with(|l| l.name != "…"),
                                    move || {
                                        let controls = controls.clone();
                                        view()
                                            .w(px(22.0))
                                            .h(px(22.0))
                                            .flex_shrink_0()
                                            .flex_row()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(5.0))
                                            .border_1()
                                            .border_color(border_control())
                                            .bg(bg_control())
                                            .hover(|style| style.bg(bg_control_hover()))
                                            .active(|style| style.opacity(0.75))
                                            .cursor(Cursor::Pointer)
                                            .label(tr("复制消息", "Copy message"))
                                            .tooltip(label_tip("复制此条消息", "Copy message"))
                                            .on_press(move || {
                                                let payload = line.with(|l| l.value.clone());
                                                set_clipboard(payload);
                                                set_status(
                                                    &controls,
                                                    StatusMsg::CopiedBody {
                                                        side: Side::Response,
                                                        hex: false,
                                                    },
                                                );
                                            })
                                            .child(
                                                canvas(move || {
                                                    Painter::new()
                                                        .line_width(1.3)
                                                        .stroke_color(color(fg_secondary()))
                                                        .stroke_svg(
                                                            "M4.5 4.5 H10.5 V11.0 H4.5 Z M2.5 8.5 H2.0 V2.0 H8.0 V3.0",
                                                        )
                                                })
                                                .w(px(12.5))
                                                .h(px(12.5)),
                                            )
                                    },
                                ))
                        },
                    )
                })
            },
        ))
}

fn diff_card(shown: Shown, folded: Signal<u8>) -> impl IntoElement {
    view()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .overflow_hidden()
        .child(
            view()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_1p5()
                .px_3p5()
                .py_2()
                .border_b(move || {
                    if card_folded(folded, FOLD_DIFF) {
                        px(0.0)
                    } else {
                        px(1.0)
                    }
                })
                .border_b_color(border_subtle())
                .hover(|style| style.bg(bg_control_hover()))
                .active(|style| style.opacity(0.85))
                .cursor(Cursor::Pointer)
                .on_press(move || toggle_card(folded, FOLD_DIFF))
                .child(fold_mark(folded, FOLD_DIFF))
                .child(section_title(tr("对比", "Diff"))),
        )
        .child(when(
            move || !card_folded(folded, FOLD_DIFF),
            move || {
                view()
                    .bg(bg_code())
                    .h(px(320.0))
                    .flex_col()
                    .overflow_hidden()
                    .child(diff_legend())
                    .child(
                        view().flex_1().min_h(px(0.0)).overflow_hidden().child(
                            diff_view()
                                .old_value(move || SharedString::from(shown.diff_old.get()))
                                .new_value(move || SharedString::from(shown.diff_new.get()))
                                .layout(DiffLayout::Unified)
                                .language("plaintext")
                                .soft_wrap(true)
                                .font_size(Points(12.0))
                                .line_height(Points(18.0))
                                .size_full(),
                        ),
                    )
            },
        ))
}

fn diff_legend() -> impl IntoElement {
    view()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_3()
        .px_3()
        .py_1p5()
        .border_b_1()
        .border_b_color(border_subtle())
        .child(diff_key(baseline_mark(), tr("基准", "Baseline")))
        .child(diff_key(diff_current_mark(), tr("当前", "Current")))
}

fn diff_key(color: AdaptiveColor, label: impl Fn() -> String + 'static) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(
            view()
                .w(px(2.0))
                .h(px(12.0))
                .rounded(px(1.0))
                .flex_shrink_0()
                .bg(color),
        )
        .child(
            text(label)
                .number_of_lines(1)
                .text_size(px(11.0))
                .text_color(fg_muted()),
        )
}

#[allow(clippy::too_many_arguments)]
fn body_section(
    zh: &'static str,
    en: &'static str,
    body: Signal<String>,
    hex: Signal<Vec<u8>>,
    note: Signal<String>,
    code: Signal<String>,
    doc: Signal<Option<nsh_base::text_view::Document>>,
    form: Signal<Vec<HeaderLine>>,
    search: Signal<String>,
    embed_image: bool,
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    folded: Signal<u8>,
    bit: u8,
) -> impl IntoElement {
    let preview_flag = if embed_image {
        shown.res_doc_preview
    } else {
        shown.req_doc_preview
    };
    let image_on = move || embed_image && shown.pretty.get() && !shown.preview.get().is_empty();
    let doc_on = move || preview_flag.get() && doc.with(Option::is_some);
    let has_body_below =
        move || doc_on() || image_on() || !form.with(Vec::is_empty) || !body.get().is_empty();
    let can_search = move || {
        !card_folded(folded, bit)
            && !doc_on()
            && (!form.with(Vec::is_empty) || !body.get().is_empty())
    };
    let pane_width = shown.pane_width;
    view()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .overflow_hidden()
        .child(
            view()
                .flex_shrink_0()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .px_3p5()
                .py_2()
                .border_b(move || {
                    if card_folded(folded, bit) {
                        px(0.0)
                    } else {
                        px(1.0)
                    }
                })
                .border_b_color(border_subtle())
                .child(
                    view()
                        .flex_1()
                        .min_w(px(96.0))
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .px_1()
                        .py_0p5()
                        .rounded(px(5.0))
                        .hover(|style| style.bg(bg_control_hover()))
                        .active(|style| style.opacity(0.85))
                        .cursor(Cursor::Pointer)
                        .on_press(move || toggle_card(folded, bit))
                        .child(fold_mark(folded, bit))
                        .child(section_title(tr(zh, en))),
                )
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_1p5()
                        .child(when(
                            move || {
                                let window = if embed_image {
                                    shown.response_window.get()
                                } else {
                                    shown.request_window.get()
                                };
                                window.has_prev() || window.has_more()
                            },
                            {
                                let controls = controls.clone();
                                move || {
                                    body_window_nav(
                                        embed_image,
                                        shown,
                                        controls.clone(),
                                        selected,
                                        baseline,
                                    )
                                }
                            },
                        ))
                        .child(when(can_search, move || {
                            let is_paged = move || {
                                let window = if embed_image {
                                    shown.response_window.get()
                                } else {
                                    shown.request_window.get()
                                };
                                window.has_prev() || window.has_more()
                            };
                            body_search_box(search, body, hex, form, shown.pane_width, is_paged)
                        }))
                        .child(when(
                            move || doc.with(Option::is_some),
                            move || doc_preview_chip(preview_flag),
                        ))
                        .child(pretty_switch(
                            shown,
                            preview_flag,
                            controls,
                            selected,
                            baseline,
                        )),
                ),
        )
        .child(when(
            move || !card_folded(folded, bit) && !note.get().is_empty(),
            move || {
                view()
                    .px_3()
                    .pt_3()
                    .pb(move || if has_body_below() { px(0.0) } else { px(12.0) })
                    .child(
                        view()
                            .px_3()
                            .py_2()
                            .rounded(px(7.0))
                            .bg(status_warn_bg())
                            .border_1()
                            .border_color(status_warn_fg())
                            .child(
                                text(move || note.get())
                                    .text_size(px(12.0))
                                    .text_color(status_warn_fg()),
                            ),
                    )
            },
        ))
        .child(when(
            move || !card_folded(folded, bit) && doc_on(),
            move || {
                let divided = !note.get().is_empty();
                view()
                    .w_full()
                    .h(move || px(editor_box_height(&body.get()).max(140.0)))
                    .bg(bg_code())
                    .overflow_hidden()
                    .border_t(move || if divided { px(1.0) } else { px(0.0) })
                    .border_t_color(border_subtle())
                    .child(
                        scroll()
                            .size_full()
                            .pl_3p5()
                            .pr_5()
                            .py_2p5()
                            .child(doc_body(doc)),
                    )
            },
        ))
        .child(when(
            move || !card_folded(folded, bit) && !doc_on() && image_on(),
            move || {
                view()
                    .px_3()
                    .py_3()
                    .gap_1p5()
                    .items_center()
                    .justify_center()
                    .child(
                        view()
                            .w(move || px(shown.preview_w.get()))
                            .h(move || px(shown.preview_h.get()))
                            .min_w(px(64.0))
                            .min_h(px(64.0))
                            .flex_shrink_0()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(border_strong())
                            .bg(bg_code())
                            .overflow_hidden()
                            .child(
                                image(move || shown.preview.get())
                                    .w_full()
                                    .h_full()
                                    .object_fit(ResizeMode::ScaleDown),
                            ),
                    )
                    .child(when(
                        move || !shown.image_caption.get().is_empty(),
                        move || muted_line(move || shown.image_caption.get()),
                    ))
            },
        ))
        .child(when(
            move || {
                !card_folded(folded, bit)
                    && !doc_on()
                    && form.with(Vec::is_empty)
                    && !body.get().is_empty()
                    && code.get().is_empty()
            },
            move || {
                let divided = image_on() || !note.get().is_empty();
                let height_body = body;
                let height_hex = hex;
                let height_search = search;
                view()
                    .w_full()
                    .min_w(move || px(hex_area_width(shown.pane_width.get())))
                    .on_layout(move |bounds: &Bounds<Points>| {
                        let pane = bounds.size.width.0 + HEX_AREA_CHROME;
                        if hex_cols_for_pane(pane)
                            != hex_cols_for_pane(shown.pane_width.get_untracked())
                        {
                            shown.pane_width.set(pane);
                        }
                    })
                    .bg(bg_code())
                    .h(move || {
                        let text = filtered_plain_body(
                            &height_body.get(),
                            &height_hex.with(|bytes| bytes.clone()),
                            shown.pane_width.get(),
                            &height_search.get(),
                        );
                        px(body_box_height(body_line_count(&text)))
                    })
                    .overflow_hidden()
                    .border_t(move || if divided { px(1.0) } else { px(0.0) })
                    .border_t_color(border_subtle())
                    .child(
                        scroll().size_full().child(
                            mono(move || {
                                let bytes = hex.with(|bytes| bytes.clone());
                                filtered_plain_body(
                                    &body.get(),
                                    &bytes,
                                    shown.pane_width.get(),
                                    &search.get(),
                                )
                            })
                            .pl_3p5()
                            .pr_5()
                            .py_2()
                            .selectable(true)
                            .on_press_up(dismiss_keyboard)
                            .text_size(px(12.0))
                            .text_color(fg_primary()),
                        ),
                    )
            },
        ))
        .child(when(
            move || {
                !card_folded(folded, bit)
                    && !doc_on()
                    && form.with(Vec::is_empty)
                    && !body.get().is_empty()
                    && !code.get().is_empty()
            },
            move || {
                let divided = image_on() || !note.get().is_empty();
                view()
                    .border_t(move || if divided { px(1.0) } else { px(0.0) })
                    .border_t_color(border_subtle())
                    .child(code_body(body, code, search))
            },
        ))
        .child(when(
            move || !card_folded(folded, bit) && !doc_on() && !form.with(Vec::is_empty),
            move || {
                each(
                    move || filtered_form_lines(&form.get(), &search.get()),
                    |line| line.key,
                    move |line, index| {
                        view()
                            .flex_row()
                            .gap_3()
                            .px_3p5()
                            .py_2()
                            .bg(move || {
                                if index.get() % 2 == 1 {
                                    bg_code()
                                } else {
                                    bg_card()
                                }
                            })
                            .border_t(move || if index.get() > 0 { px(1.0) } else { px(0.0) })
                            .border_t_color(border_subtle())
                            .on_press_up(dismiss_keyboard)
                            .child(
                                mono(move || line.with(|line| line.name.clone()))
                                    .w(move || px(kv_key_width(pane_width.get())))
                                    .flex_shrink_0()
                                    .selectable(true)
                                    .number_of_lines(1)
                                    .text_overflow(EllipsizeMode::Tail)
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(fg_muted()),
                            )
                            .child(kv_value_cell(line))
                    },
                )
            },
        ))
}

fn body_window_nav(
    response: bool,
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
) -> impl IntoElement {
    let back_controls = controls.clone();
    let next_controls = controls;
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(when(
            move || {
                let window = if response {
                    shown.response_window.get()
                } else {
                    shown.request_window.get()
                };
                window.has_prev()
            },
            move || {
                let controls = back_controls.clone();
                detail_button(tr("往前", "Previous"), move || {
                    nudge_body_window(shown, &controls, selected, baseline, response, false)
                })
            },
        ))
        .child(when(
            move || {
                let window = if response {
                    shown.response_window.get()
                } else {
                    shown.request_window.get()
                };
                window.has_more()
            },
            move || {
                let controls = next_controls.clone();
                detail_button(tr("往后", "Next"), move || {
                    nudge_body_window(shown, &controls, selected, baseline, response, true)
                })
            },
        ))
}

fn nudge_body_window(
    shown: Shown,
    controls: &Controls,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    response: bool,
    forward: bool,
) {
    let window = if response {
        shown.response_window.get()
    } else {
        shown.request_window.get()
    };
    if (forward && !window.has_more()) || (!forward && !window.has_prev()) {
        return;
    }
    let next = if forward {
        window.end
    } else {
        window.offset.saturating_sub(BODY_DISPLAY_CAP)
    };
    if response {
        shown.response_offset.set(next);
    } else {
        shown.request_offset.set(next);
    }
    publish_detail(
        &shown,
        &controls.hub,
        selected.get(),
        ui_lang(ui_langs()),
        baseline.get(),
    );
}

fn reset_body_offsets(shown: Shown) {
    if shown.request_offset.get_untracked() != 0 {
        shown.request_offset.set(0);
    }
    if shown.response_offset.get_untracked() != 0 {
        shown.response_offset.set(0);
    }
}

fn filtered_plain_body(body: &str, hex: &[u8], pane_width: f32, query: &str) -> String {
    let source = if hex.is_empty() {
        body.to_string()
    } else {
        let cols = hex_cols_for_pane(pane_width);
        hex_display_rows(hex, cols)
    };
    if query.trim().is_empty() {
        source
    } else {
        present::filter_body_lines(&source, query).1
    }
}

fn filtered_form_lines(lines: &[HeaderLine], query: &str) -> Vec<HeaderLine> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return lines.to_vec();
    }
    lines
        .iter()
        .filter(|line| {
            line.name.to_ascii_lowercase().contains(&q)
                || line.value.to_ascii_lowercase().contains(&q)
                || line
                    .hint
                    .as_ref()
                    .is_some_and(|h| h.to_ascii_lowercase().contains(&q))
        })
        .cloned()
        .collect()
}

fn body_search_hit_count(
    body: Signal<String>,
    hex: Signal<Vec<u8>>,
    form: Signal<Vec<HeaderLine>>,
    pane_width: Signal<f32>,
    query: &str,
) -> usize {
    let q = query.trim();
    if q.is_empty() {
        return 0;
    }
    if !form.with(Vec::is_empty) {
        return filtered_form_lines(&form.get(), q).len();
    }
    let bytes = hex.with(|bytes| bytes.clone());
    let source = if bytes.is_empty() {
        body.get()
    } else {
        hex_display_rows(&bytes, hex_cols_for_pane(pane_width.get()))
    };
    present::filter_body_lines(&source, q).0
}

fn body_search_box(
    search: Signal<String>,
    body: Signal<String>,
    hex: Signal<Vec<u8>>,
    form: Signal<Vec<HeaderLine>>,
    pane_width: Signal<f32>,
    is_paged: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(
            input()
                .value(search)
                .placeholder(tr("搜索…", "Search…"))
                .placeholder_color(move || color(fg_muted()))
                .w(px(120.0))
                .h(px(22.0))
                .px_2()
                .rounded(px(6.0))
                .bg(bg_code())
                .border_1()
                .border_color(move || {
                    if search.get().trim().is_empty() {
                        border_control()
                    } else {
                        accent_blue_border()
                    }
                })
                .focus(|style| style.border_color(accent_blue()))
                .text_size(px(11.5))
                .text_color(fg_primary())
                .on_change(move |text: &str| search.set(text.to_string())),
        )
        .child(when(
            move || !search.get().trim().is_empty(),
            move || {
                view()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(when(is_paged, || {
                        view()
                            .h(px(22.0))
                            .px_1p5()
                            .rounded(px(6.0))
                            .bg(status_warn_bg())
                            .border_1()
                            .border_color(status_warn_fg())
                            .flex_row()
                            .items_center()
                            .tooltip(label_tip(
                                "仅在当前 256 KB 分页内匹配与过滤",
                                "Matches within the current 256 KB page only",
                            ))
                            .child(
                                text(tr("当前页命中", "Page matches"))
                                    .number_of_lines(1)
                                    .text_size(px(10.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(status_warn_fg()),
                            )
                    }))
                    .child(
                        view()
                            .h(px(22.0))
                            .px_1p5()
                            .rounded(px(6.0))
                            .bg(accent_blue_soft())
                            .hover(|style| style.bg(accent_blue_soft_hover()))
                            .active(|style| style.opacity(0.8))
                            .border_1()
                            .border_color(accent_blue_border())
                            .flex_row()
                            .items_center()
                            .cursor(Cursor::Pointer)
                            .tooltip(label_tip("点击清空搜索", "Click to clear search"))
                            .on_press(move || search.set(String::new()))
                            .child(
                                mono(move || {
                                    format!(
                                        "{} ×",
                                        body_search_hit_count(
                                            body,
                                            hex,
                                            form,
                                            pane_width,
                                            &search.get()
                                        )
                                    )
                                })
                                .tabular_nums()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(accent_blue_text()),
                            ),
                    )
            },
        ))
}

const BODY_LINE_H: f32 = 18.0;
const BODY_LINES: usize = 10;
const BODY_CHROME: f32 = 16.0;

fn body_line_count(text: &str) -> usize {
    text.lines().count().max(1)
}

/// One line of body grows the card. Past [`BODY_LINES`] the card stays put
/// and the text scrolls inside it.
fn body_box_height(lines: usize) -> f32 {
    let shown = lines.clamp(1, BODY_LINES) as f32;
    shown * BODY_LINE_H + BODY_CHROME
}

fn editor_box_height(text: &str) -> f32 {
    body_box_height(body_line_count(text))
}

fn code_body(
    body: Signal<String>,
    code: Signal<String>,
    search: Signal<String>,
) -> impl IntoElement {
    let shown_text = move || {
        let raw = body.get();
        let q = search.get();
        if q.trim().is_empty() {
            raw
        } else {
            present::filter_body_lines(&raw, &q).1
        }
    };
    view()
        .w_full()
        .h(move || px(editor_box_height(&shown_text())))
        .bg(bg_code())
        .child(
            editor()
                .value(move || SharedString::from(shown_text()))
                .read_only(true)
                .language(move || {
                    if search.get().trim().is_empty() {
                        SharedString::from(code.get())
                    } else {
                        SharedString::from("plaintext")
                    }
                })
                .line_numbers(true)
                .folding(true)
                .soft_wrap(true)
                .font_size(Points(12.0))
                .line_height(Points(18.0))
                .scroll_beyond_last_line(false)
                .size_full(),
        )
}

fn doc_body(doc: Signal<Option<nsh_base::text_view::Document>>) -> impl IntoElement {
    nsh_base::text_view::text_view()
        .document(move || doc.get())
        .selectable(true)
        .render_image(|img: &nsh_base::text_view::Image| {
            let label = if img.alt.is_empty() {
                format!("[img: {}]", img.src)
            } else {
                format!("[img: {} ({})]", img.alt, img.src)
            };
            mono(label).text_size(px(11.5)).text_color(fg_muted())
        })
}

fn doc_preview_chip(preview: Signal<bool>) -> impl IntoElement {
    view()
        .h(px(22.0))
        .px_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || {
            if preview.get() {
                accent_blue()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if preview.get() {
                accent_blue()
            } else {
                bg_card()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if preview.get() {
                    accent_blue_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .flex_row()
        .items_center()
        .cursor(Cursor::Pointer)
        .on_press(move || {
            preview.set(!preview.get());
        })
        .child(
            text(tr("预览", "Preview"))
                .number_of_lines(1)
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if preview.get() {
                        AdaptiveColor::rgb(0xffffff, 0xffffff)
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

fn pretty_switch(
    shown: Shown,
    preview: Signal<bool>,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
) -> impl IntoElement {
    view()
        .flex_row()
        .flex_shrink_0()
        .h(px(22.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(border_control())
        .overflow_hidden()
        .child(pretty_chip(
            shown,
            preview,
            controls.clone(),
            selected,
            baseline,
            true,
            "美化",
            "Pretty",
        ))
        .child(pretty_chip(
            shown, preview, controls, selected, baseline, false, "原文", "Raw",
        ))
}

fn pretty_chip(
    shown: Shown,
    preview: Signal<bool>,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    on: bool,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    let langs = ui_langs();
    let is_active = move || !preview.get() && shown.pretty.get() == on;
    view()
        .h(px(22.0))
        .px_2()
        .flex_row()
        .items_center()
        .border_r(move || if on { px(1.0) } else { px(0.0) })
        .border_r_color(border_control())
        .bg(move || {
            if is_active() {
                accent_blue()
            } else {
                bg_card()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if is_active() {
                    accent_blue_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .on_press(move || {
            let was_preview = preview.get();
            if was_preview {
                preview.set(false);
            }
            if !was_preview && shown.pretty.get() == on {
                return;
            }
            shown.pretty.set(on);
            reset_body_offsets(shown);
            publish_detail(
                &shown,
                &controls.hub,
                selected.get(),
                ui_lang(langs),
                baseline.get(),
            );
        })
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if is_active() {
                        AdaptiveColor::rgb(0xffffff, 0xffffff)
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

fn detail_actions(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
    compose: Compose,
    filter: Signal<String>,
    rules_text: Signal<String>,
    baseline: Signal<Option<u64>>,
) -> impl IntoElement {
    let copy_url = controls.clone();
    let copy_curl = controls.clone();
    let copy_fetch = controls.clone();
    let copy_python = controls.clone();
    let copy_rust = controls.clone();
    let copy_go = controls.clone();
    let copy_types = controls.clone();
    let copy_rust_struct = controls.clone();
    let copy_request = controls.clone();
    let copy_response = controls.clone();
    let save = controls.clone();
    let focus_controls = controls.clone();
    let mute_controls = controls.clone();
    let pin_controls = controls.clone();
    let base_controls = controls.clone();
    let replay_controls = controls.clone();
    let repeat_controls = controls.clone();
    let edit_controls = controls.clone();
    let send_controls = controls.clone();
    let break_controls = controls.clone();
    let map_local_controls = controls.clone();
    let pretty_controls = controls;
    let pinned = shown.pinned;
    let pretty = shown.pretty;
    let baseline_mark = baseline;
    let selected_mark = selected;
    let can_local = move || shown.relay.get().is_empty();
    let is_req_break = move || {
        !compose.response.get()
            && paused
                .get()
                .is_some_and(|(id, side)| selected.get() == Some(id) && side == BreakSide::Request)
    };
    let open = signal(false);
    let items = view()
        .w_full()
        .p_1()
        .gap_0p5()
        .child(action_item(tr("复制 URL", "Copy URL"), move || {
            copy_part(&copy_url, selected.get(), CopyPart::Url);
        }))
        .child(action_item("cURL", move || {
            copy_part(&copy_curl, selected.get(), CopyPart::Curl);
        }))
        .child(action_item("fetch", move || {
            copy_snippet(&copy_fetch, selected.get(), Snippet::Fetch);
        }))
        .child(action_item("Python", move || {
            copy_snippet(&copy_python, selected.get(), Snippet::Python);
        }))
        .child(action_item("Rust (reqwest)", move || {
            copy_snippet(&copy_rust, selected.get(), Snippet::RustReqwest);
        }))
        .child(action_item("Go (net/http)", move || {
            copy_snippet(&copy_go, selected.get(), Snippet::GoHttp);
        }))
        .child(action_item(tr("TS 类型", "TS types"), move || {
            copy_snippet(&copy_types, selected.get(), Snippet::Types);
        }))
        .child(action_item(
            tr("Rust 结构体", "Rust struct"),
            move || {
                copy_snippet(&copy_rust_struct, selected.get(), Snippet::RustStruct);
            },
        ))
        .child(action_item(tr("复制请求", "Copy request"), move || {
            copy_part(&copy_request, selected.get(), CopyPart::Request);
        }))
        .child(action_item(
            tr("复制响应", "Copy response"),
            move || {
                copy_part(&copy_response, selected.get(), CopyPart::Response);
            },
        ));
    #[cfg(not(target_arch = "wasm32"))]
    let items = items.child(action_item(
        tr("另存响应…", "Save response…"),
        move || {
            export_response(save.clone(), selected.get());
        },
    ));
    #[cfg(target_arch = "wasm32")]
    let items = {
        let save_web = save.clone();
        let has_res_body = move || {
            !shown.response_body.get().is_empty()
                || !shown.response_hex.with(Vec::is_empty)
                || !shown.preview.get().is_empty()
                || selected
                    .get()
                    .and_then(|id| selected_flow(&save, Some(id)))
                    .is_some_and(|flow| !flow.response_body.is_empty())
        };
        items.child(action_item_enabled(
            tr("另存响应…", "Save response…"),
            has_res_body,
            move || {
                export_response_web(save_web.clone(), selected.get());
            },
        ))
    };
    let items = items
        .child(menu_rule())
        .child(action_item(tr("只看", "Focus"), move || {
            focus_host(&focus_controls, selected.get(), filter);
        }))
        .child(action_item(tr("屏蔽", "Mute"), move || {
            mute_host(&mute_controls, selected.get(), filter);
        }))
        .child(menu_rule())
        .child(marked_action(
            move || pinned.get(),
            {
                let langs = ui_langs();
                move || -> SharedString {
                    if pinned.get() {
                        t(ui_lang(langs), "取消星标", "Unstar").into()
                    } else {
                        t(ui_lang(langs), "星标", "Star").into()
                    }
                }
            },
            move || toggle_pin(&pin_controls, selected.get()),
        ))
        .child(marked_action(
            move || {
                baseline_mark
                    .get()
                    .is_some_and(|id| selected_mark.get() == Some(id))
            },
            {
                let langs = ui_langs();
                let selected_label = selected;
                let baseline_label = baseline;
                move || -> SharedString {
                    if baseline_label
                        .get()
                        .is_some_and(|id| selected_label.get() == Some(id))
                    {
                        t(ui_lang(langs), "取消基准", "Clear baseline").into()
                    } else {
                        t(ui_lang(langs), "基准", "Baseline").into()
                    }
                }
            },
            {
                let langs = ui_langs();
                move || {
                    set_baseline(
                        &base_controls,
                        selected.get(),
                        baseline,
                        &shown,
                        ui_lang(langs),
                    );
                }
            },
        ))
        .child(action_item(tr("备注…", "Note…"), move || {
            shown.repeating.set(false);
            shown.editing_comment.set(true);
        }))
        .child(marked_action(
            move || pretty.get(),
            tr("美化", "Pretty"),
            {
                let langs = ui_langs();
                move || {
                    shown.req_doc_preview.set(false);
                    shown.res_doc_preview.set(false);
                    shown.pretty.set(!shown.pretty.get());
                    reset_body_offsets(shown);
                    publish_detail(
                        &shown,
                        &pretty_controls.hub,
                        selected.get(),
                        ui_lang(langs),
                        baseline.get(),
                    );
                }
            },
        ))
        .child(menu_rule())
        .child(action_item_enabled(
            tr("重放", "Replay"),
            can_local,
            move || {
                replay_flow(&replay_controls, selected.get(), ReplayOverride::default());
            },
        ))
        .child(action_item_enabled(
            tr("批量重放…", "Repeat…"),
            can_local,
            move || {
                if refuse_relay(&repeat_controls, selected.get()) {
                    return;
                }
                shown.editing_comment.set(false);
                shown.repeating.set(true);
            },
        ))
        .child(action_item_enabled(
            {
                let langs = ui_langs();
                move || -> SharedString {
                    edit_action_label(ui_lang(langs), selected.get(), paused.get()).into()
                }
            },
            can_local,
            move || {
                begin_compose(&edit_controls, selected.get(), compose);
            },
        ))
        .child(action_item_enabled(
            tr("填入断点", "Add break"),
            can_local,
            move || {
                fill_break_rule(&break_controls, selected.get(), rules_text);
            },
        ))
        .child(action_item_enabled(
            {
                let langs = ui_langs();
                move || -> SharedString {
                    #[cfg(not(target_arch = "wasm32"))]
                    let saves_file = !shown.response_body.get().is_empty()
                        || !shown.response_hex.with(Vec::is_empty);
                    #[cfg(target_arch = "wasm32")]
                    let saves_file = false;
                    if saves_file {
                        t(ui_lang(langs), "本地映射…", "Map local…").into()
                    } else {
                        t(ui_lang(langs), "本地映射", "Map local").into()
                    }
                }
            },
            can_local,
            move || {
                map_local_from_flow(map_local_controls.clone(), selected.get(), rules_text);
            },
        ))
        .child(when(
            move || compose.composing.get() && !compose.response.get(),
            move || {
                let controls = send_controls.clone();
                let langs = ui_langs();
                action_item_enabled(
                    move || -> SharedString {
                        if is_req_break() {
                            t(ui_lang(langs), "放行", "Continue").into()
                        } else {
                            t(ui_lang(langs), "发送", "Send").into()
                        }
                    },
                    can_local,
                    move || {
                        if is_req_break()
                            || controls.hub.paused().is_some_and(|(id, side)| {
                                selected.get() == Some(id) && side == BreakSide::Request
                            })
                        {
                            continue_break(&controls, compose, selected, paused);
                            return;
                        }
                        replay_flow(
                            &controls,
                            selected.get(),
                            ReplayOverride {
                                method: Some(compose.method.get()),
                                url: Some(compose.url.get()),
                                headers: Some(parse_header_text(&compose.headers.get())),
                                body: Some(compose.body.get().into_bytes()),
                            },
                        );
                    },
                )
            },
        ));
    menu()
        .on_open_change(move |event: &nsh_base::MenuEvent| {
            if let nsh_base::MenuEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(8.0))
        .trigger(detail_more_trigger(open))
        .child(
            view()
                .w(px(208.0))
                .max_w(move || px((window().width.0 - 16.0).max(160.0)))
                .max_h(move || px((window().height.0 - 96.0).max(160.0)))
                .rounded(px(10.0))
                .border_1()
                .border_color(border_strong())
                .bg(bg_card())
                .shadow_md()
                .overflow_hidden()
                .child(
                    scroll()
                        .w_full()
                        .max_h(move || px((window().height.0 - 98.0).max(158.0)))
                        .child(items),
                ),
        )
}

fn detail_more_trigger(open: Signal<bool>) -> impl IntoElement {
    view()
        .h(px(26.0))
        .px_2()
        .gap_1()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || {
            if open.get() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if open.get() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if open.get() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.75))
        .child(
            text(tr("更多", "More"))
                .number_of_lines(1)
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if open.get() {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
        )
        .child(glyph(
            || Glyph::Chevron,
            move || {
                color(if open.get() {
                    accent_blue_text()
                } else {
                    fg_secondary()
                })
            },
        ))
}

fn action_item(
    label: impl IntoReactive<SharedString>,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    action_item_enabled(label, || true, on_select)
}

fn action_item_enabled(
    label: impl IntoReactive<SharedString>,
    enabled: impl Fn() -> bool + 'static + Clone,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    let disabled_check = enabled.clone();
    let hover_check = enabled.clone();
    let active_check = enabled.clone();
    let fg_check = enabled;
    menu_item()
        .disabled(move || !disabled_check())
        .disabled_style(|style| style.opacity(0.45).cursor(Cursor::Default))
        .on_select(on_select)
        .h(px(30.0))
        .px_2p5()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .hover(|style| {
            style.bg(move || {
                if hover_check() {
                    bg_control_hover()
                } else {
                    AdaptiveColor::TRANSPARENT
                }
            })
        })
        .active(|style| style.opacity(move || if active_check() { 0.75_f32 } else { 1.0_f32 }))
        .cursor(Cursor::Pointer)
        .child(control_text(label).text_color(move || {
            if fg_check() {
                fg_secondary()
            } else {
                fg_muted()
            }
        }))
}

fn marked_action(
    marked: impl Fn() -> bool + 'static + Clone,
    label: impl IntoReactive<SharedString>,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    marked_action_inner(marked, label, true, on_select)
}

fn marked_radio_stay(
    marked: impl Fn() -> bool + 'static + Clone,
    label: impl IntoReactive<SharedString>,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    marked_action_inner(marked, label, false, on_select)
}

fn marked_action_inner(
    marked: impl Fn() -> bool + 'static + Clone,
    label: impl IntoReactive<SharedString>,
    close_on_select: bool,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    let dot = marked.clone();
    let dot_fill = marked.clone();
    let label_on = marked;
    menu_item()
        .close_on_select(close_on_select)
        .on_select(on_select)
        .h(px(30.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.opacity(0.75))
        .cursor(Cursor::Pointer)
        .child(
            view()
                .w(px(8.0))
                .h(px(8.0))
                .rounded_full()
                .border_1()
                .border_color(move || if dot() { accent_blue() } else { fg_muted() })
                .bg(move || if dot_fill() { accent_blue() } else { bg_card() }),
        )
        .child(control_text(label).text_color(move || {
            if label_on() {
                accent_blue_text()
            } else {
                fg_secondary()
            }
        }))
}

fn detail_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .h(px(26.0))
        .px_2()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(border_control())
        .bg(bg_control())
        .hover(|style| style.bg(bg_control_hover()))
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.75))
        .on_press(on_press)
        .child(
            text(label)
                .number_of_lines(1)
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg_secondary()),
        )
}

fn section_title(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg_secondary())
}

fn muted_line(label: impl IntoReactive<SharedString>) -> Text {
    text(label).text_size(px(12.0)).text_color(fg_muted())
}

fn footer(
    listen: Signal<String>,
    running: Signal<bool>,
    memory: Signal<String>,
    dropped: Signal<u64>,
    port_text: Signal<String>,
    lan_url: Signal<String>,
    lan_choice: Signal<String>,
    lan_addrs: Signal<Vec<LanAddr>>,
    qr_path: Signal<String>,
    listen_all: Signal<bool>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let port_controls = controls.clone();
    let lan_controls = controls;
    view()
        .h(px(32.0))
        .pl(px(LIST_INSET))
        .pr_3()
        .gap_3()
        .flex_row()
        .items_center()
        .bg(bg_chrome())
        .border_t_1()
        .border_t_color(border_strong())
        .child(
            view()
                .flex_1()
                .flex_row()
                .items_center()
                .gap_2()
                .min_w(px(0.0))
                .overflow_hidden()
                .child(
                    view()
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded_full()
                        .flex_shrink_0()
                        .bg(move || {
                            if running.get() {
                                status_ok_fg()
                            } else {
                                fg_muted()
                            }
                        }),
                )
                .child(
                    when(
                        move || listen.get().is_empty(),
                        move || {
                            text(tr("监听尚未开始", "Not listening"))
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Tail)
                                .text_size(px(11.5))
                                .text_color(fg_muted())
                        },
                    )
                    .otherwise(move || {
                        view()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .child(
                                text(tr("监听", "Listening"))
                                    .flex_shrink_0()
                                    .number_of_lines(1)
                                    .text_size(px(11.5))
                                    .text_color(fg_muted()),
                            )
                            .child(
                                mono(move || listen.get())
                                    .min_w(px(0.0))
                                    .tabular_nums()
                                    .number_of_lines(1)
                                    .text_overflow(EllipsizeMode::Tail)
                                    .text_size(px(11.5))
                                    .text_color(fg_muted()),
                            )
                    }),
                ),
        )
        .child(when(
            move || !lan_url.get().is_empty(),
            move || cert_qr(listen, lan_url, lan_choice, lan_addrs, qr_path),
        ))
        .child(when(
            move || !running.get(),
            move || port_field(port_text, port_controls.clone()),
        ))
        .child(when(
            move || !running.get(),
            move || lan_toggle(listen_all, lan_controls.clone()),
        ))
        .child(
            view()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_1()
                .child(
                    mono(move || memory.get())
                        .tabular_nums()
                        .text_size(px(11.5))
                        .text_color(fg_muted()),
                )
                .child(
                    text(tr("· 已丢弃", "· dropped"))
                        .text_size(px(11.5))
                        .text_color(fg_muted()),
                )
                .child(
                    mono(move || dropped.get().to_string())
                        .tabular_nums()
                        .text_size(px(11.5))
                        .text_color(fg_muted()),
                ),
        )
}

fn cert_qr(
    listen: Signal<String>,
    lan_url: Signal<String>,
    lan_choice: Signal<String>,
    lan_addrs: Signal<Vec<LanAddr>>,
    qr_path: Signal<String>,
) -> impl IntoElement {
    let open = signal(false);
    popover()
        .placement(Placement::Top)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(8.0))
        .on_open_change(move |event: &nsh_base::PopoverEvent| {
            if let nsh_base::PopoverEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .trigger(qr_trigger(lan_choice, open))
        .child(qr_panel(listen, lan_url, lan_choice, lan_addrs, qr_path))
}

fn qr_trigger(choice: Signal<String>, open: Signal<bool>) -> impl IntoElement {
    view()
        .h(px(22.0))
        .px_2()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_1()
        .rounded(px(6.0))
        .border_1()
        .border_color(accent_blue_border())
        .bg(move || {
            if open.get() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if open.get() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.75))
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .bg(accent_blue()),
        )
        .child(
            text({
                let langs = ui_langs();
                move || -> SharedString {
                    format!("{} {}", t(ui_lang(langs), "扫码", "Scan"), choice.get()).into()
                }
            })
            .tabular_nums()
            .number_of_lines(1)
            .text_size(px(11.5))
            .text_color(move || {
                if open.get() {
                    accent_blue_text()
                } else {
                    fg_secondary()
                }
            }),
        )
}

fn qr_panel(
    listen: Signal<String>,
    lan_url: Signal<String>,
    lan_choice: Signal<String>,
    lan_addrs: Signal<Vec<LanAddr>>,
    qr_path: Signal<String>,
) -> impl IntoElement {
    view()
        .w(move || px((window().width.0 - 20.0).clamp(280.0, 612.0)))
        .max_h(move || px((window().height.0 - 50.0).max(200.0)))
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
        .overflow_hidden()
        .child(
            scroll()
                .w_full()
                .max_h(move || px((window().height.0 - 52.0).max(198.0)))
                .child(
                    view()
                        .w_full()
                        .p_3()
                        .gap_2()
                        .items_center()
                        .child(
                            text(tr("手机安装页", "Phone install page"))
                                .text_size(px(12.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(fg_secondary()),
                        )
                        .child(
                            view()
                                .w_full()
                                .p_2()
                                .rounded(px(6.0))
                                .bg(bg_code())
                                .border_1()
                                .border_color(border_strong())
                                .child(
                                    mono(move || lan_url.get())
                                        .text_size(px(12.0))
                                        .text_color(fg_primary()),
                                ),
                        )
                        .child(
                            view()
                                .w_full()
                                .flex_row()
                                .flex_wrap()
                                .items_start()
                                .gap_3()
                                .child(qr_code_side(
                                    listen, lan_url, lan_choice, lan_addrs, qr_path,
                                ))
                                .child(qr_phone_steps()),
                        ),
                ),
        )
}

fn qr_code_side(
    listen: Signal<String>,
    lan_url: Signal<String>,
    lan_choice: Signal<String>,
    lan_addrs: Signal<Vec<LanAddr>>,
    qr_path: Signal<String>,
) -> impl IntoElement {
    view()
        .w(px(236.0))
        .max_w(move || px((window().width.0 - 44.0).clamp(200.0, 236.0)))
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .child(
            view().w_full().child(
                text({
                    let langs = ui_langs();
                    move || -> SharedString {
                        let lang = ui_lang(langs);
                        let ip = lan_choice.get();
                        let name = lan_addrs.with(|addrs| {
                            addrs
                                .iter()
                                .find(|addr| addr.ip.to_string() == ip)
                                .map(|addr| addr.name.clone())
                                .unwrap_or_default()
                        });
                        if lan_addrs.with(Vec::len) > 1 {
                            t(
                                lang,
                                "点下面换一张网卡。手机要和这个地址在同一网络。",
                                "Pick a network below. The phone must be on that network.",
                            )
                            .into()
                        } else if name.is_empty() {
                            t(
                                lang,
                                "手机要能访问这个地址。",
                                "The phone must be able to reach this address.",
                            )
                            .into()
                        } else if lang == Lang::Zh {
                            format!("网卡 {name}。手机要和它在同一网络。").into()
                        } else {
                            format!("Interface {name}. The phone must be on that network.").into()
                        }
                    }
                })
                .text_size(px(12.0))
                .text_color(fg_muted()),
            ),
        )
        .child(when(
            move || lan_addrs.with(Vec::len) > 1,
            move || {
                view().w_full().gap_1().child(each(
                    move || lan_addrs.get(),
                    |addr: &LanAddr| addr.ip.to_string(),
                    move |addr, _index| {
                        lan_choice_row(addr, listen, lan_addrs, lan_choice, lan_url, qr_path)
                    },
                ))
            },
        ))
        .child(when(
            move || !qr_path.get().is_empty(),
            move || {
                view()
                    .p_2()
                    .rounded(px(8.0))
                    .bg(AdaptiveColor::rgb(0xffffff, 0xffffff))
                    .child(image(move || qr_path.get()).w(px(168.0)).h(px(168.0)))
            },
        ))
        .child(when(
            move || qr_path.get().is_empty(),
            move || muted_line(tr("二维码没有生成", "QR code was not generated")),
        ))
}

/// Steps shown beside the QR. Same path as the phone install page, plus the
/// Wi-Fi proxy the page only mentions in one sentence.
fn qr_phone_steps() -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(240.0))
        .gap_3()
        .child(
            view()
                .w_full()
                .gap_1p5()
                .child(qr_os_heading("iOS"))
                .child(
                    view()
                        .w_full()
                        .gap_2()
                        .child(qr_step(
                            "1.",
                            tr(
                                "扫码打开安装页，点「下载根证书」。系统会提示安装描述文件。",
                                "Scan to open the page and tap Download the root certificate. The system offers to install a profile.",
                            ),
                        ))
                        .child(qr_step(
                            "2.",
                            tr(
                                "打开「设置」→「已下载描述文件」，安装 zniff CA。",
                                "Open Settings → Profile Downloaded and install zniff CA.",
                            ),
                        ))
                        .child(qr_step(
                            "3.",
                            tr(
                                "再到「设置」→「通用」→「关于本机」→「证书信任设置」，打开 zniff CA 的完全信任。",
                                "Then open Settings → General → About → Certificate Trust Settings and enable full trust for zniff CA.",
                            ),
                        ))
                        .child(qr_step(
                            "4.",
                            tr(
                                "「设置」→「无线局域网」→ 当前网络右侧 ⓘ →「配置代理」→「手动」。服务器和端口填上面的地址。",
                                "Settings → Wi-Fi → the ⓘ beside this network → Configure Proxy → Manual. Use the server and port above.",
                            ),
                        )),
                ),
        )
        .child(
            view()
                .w_full()
                .gap_1p5()
                .child(qr_os_heading("Android"))
                .child(
                    view()
                        .w_full()
                        .gap_2()
                        .child(qr_step(
                            "1.",
                            tr(
                                "扫码打开安装页，点下载并保存 zniff-ca.crt。",
                                "Scan to open the page, then download and save zniff-ca.crt.",
                            ),
                        ))
                        .child(qr_step(
                            "2.",
                            tr(
                                "打开「设置」→「安全」→「加密与凭据」→「安装证书」→「CA 证书」。",
                                "Open Settings → Security → Encryption & credentials → Install a certificate → CA certificate.",
                            ),
                        ))
                        .child(qr_step(
                            "3.",
                            tr(
                                "打开当前 WLAN 的详情，或「修改网络」→「高级选项」。代理选「手动」，主机名和端口填上面的地址。",
                                "Open this WLAN's details, or Modify network → Advanced. Set Proxy to Manual and use the hostname and port above.",
                            ),
                        )),
                ),
        )
}

fn qr_step(
    index: &'static str,
    body: impl IntoReactive<SharedString> + 'static,
) -> impl IntoElement {
    view()
        .w_full()
        .flex_row()
        .items_start()
        .gap_1p5()
        .child(
            view().w(px(18.0)).flex_shrink_0().child(
                text(index)
                    .text_size(px(12.0))
                    .line_height(Points(18.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(fg_secondary()),
            ),
        )
        .child(
            view().flex_1().min_w(px(0.0)).child(
                text(body)
                    .text_size(px(12.0))
                    .line_height(Points(18.0))
                    .text_color(fg_muted()),
            ),
        )
}

fn qr_os_heading(title: &'static str) -> Text {
    text(title)
        .text_size(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg_primary())
}

fn lan_choice_row(
    addr: ReadSignal<LanAddr>,
    listen: Signal<String>,
    lan_addrs: Signal<Vec<LanAddr>>,
    lan_choice: Signal<String>,
    lan_url: Signal<String>,
    qr_path: Signal<String>,
) -> impl IntoElement {
    let picked = move || lan_choice.get() == addr.with(|item| item.ip.to_string());
    let picked_bg = picked;
    let picked_dot = picked;
    view()
        .h(px(28.0))
        .px_2()
        .w_full()
        .flex_row()
        .items_center()
        .gap_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || {
            if picked() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if picked_bg() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if picked() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .on_press(move || {
            let ip = addr.with(|item| item.ip.to_string());
            publish_lan(&listen.get(), &ip, lan_addrs, lan_choice, lan_url, qr_path);
        })
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .flex_shrink_0()
                .bg(move || {
                    if picked_dot() {
                        accent_blue()
                    } else {
                        fg_muted()
                    }
                }),
        )
        .child(
            mono(move || -> SharedString {
                let item = addr.get();
                if item.name.is_empty() {
                    item.ip.to_string().into()
                } else {
                    format!("{}  {}", item.name, item.ip).into()
                }
            })
            .number_of_lines(1)
            .text_size(px(12.0))
            .text_color(fg_primary()),
        )
}

fn port_field(port_text: Signal<String>, controls: Arc<Controls>) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_1p5()
        .flex_shrink_0()
        .child(
            text(tr("端口", "Port"))
                .text_size(px(11.5))
                .text_color(fg_muted()),
        )
        .child(
            input()
                .value(port_text)
                .placeholder(config::default_listen_port(config::is_dev_mode()).to_string())
                .placeholder_color(move || color(fg_muted()))
                .w(px(72.0))
                .h(px(22.0))
                .px_2()
                .rounded(px(6.0))
                .bg(bg_code())
                .border_1()
                .border_color(border_control())
                .focus(|style| style.border_color(accent_blue()))
                .text_size(px(12.0))
                .text_color(fg_primary())
                .on_change(move |text: &str| {
                    port_text.set(text.to_string());
                    if let Some(port) = present::parse_port(text) {
                        set_port(&controls.listen_port, port);
                        if let Err(err) = store_listen_port(port) {
                            set_status(
                                &controls,
                                StatusMsg::PrefUnsaved {
                                    name: PrefName::Port,
                                    err: err.to_string(),
                                },
                            );
                        }
                    }
                }),
        )
}

fn control_text(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .number_of_lines(1)
        .text_size(px(CONTROL_TEXT))
        .font_weight(FontWeight::MEDIUM)
}

#[allow(clippy::too_many_arguments)]
fn toolbar_menu(
    ca_status: Signal<ca::CaStatus>,
    proxy_on: Signal<bool>,
    install: Rc<impl Fn() + 'static>,
    remove: Rc<impl Fn() + 'static>,
    toggle_proxy: impl Fn() + 'static,
    export_har: impl Fn() + 'static,
    copy_env: impl Fn() + 'static,
    clear: impl Fn() + 'static,
    clear_unpinned: impl Fn() + 'static,
    import_controls: Arc<Controls>,
    rows: Signal<Vec<Arc<RowLine>>>,
    flow_total: Signal<usize>,
    filter: Signal<String>,
    pretty: Signal<bool>,
    shown_phase: Signal<Phase>,
    share_id: Signal<String>,
    compose: Compose,
    seq_builder: SeqBuilder,
    selected: Signal<Option<u64>>,
    update_state: Signal<UpdateState>,
) -> impl IntoElement {
    let open = signal(false);
    menu()
        .on_open_change(move |event: &nsh_base::MenuEvent| {
            if let nsh_base::MenuEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(8.0))
        .trigger(more_trigger(ca_status, proxy_on, update_state, open))
        .child(more_menu(
            ca_status,
            proxy_on,
            install,
            remove,
            toggle_proxy,
            export_har,
            copy_env,
            clear,
            clear_unpinned,
            import_controls,
            rows,
            flow_total,
            filter,
            pretty,
            shown_phase,
            share_id,
            compose,
            seq_builder,
            selected,
            update_state,
        ))
}

/// Session size for the toolbar and the Clear item. A filter shows
/// `matched / total`; an empty filter shows the total alone.
fn flow_count_text(filtered: usize, total: usize, filtering: bool) -> String {
    if filtering {
        format!("{filtered} / {total}")
    } else {
        total.to_string()
    }
}

fn flow_count_now(
    rows: Signal<Vec<Arc<RowLine>>>,
    flow_total: Signal<usize>,
    filter: Signal<String>,
) -> String {
    flow_count_text(
        rows.with(Vec::len),
        flow_total.get(),
        !filter.get().is_empty(),
    )
}

fn flow_count_badge(
    rows: Signal<Vec<Arc<RowLine>>>,
    flow_total: Signal<usize>,
    filter: Signal<String>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .window_drag(false)
        .h(px(CONTROL_H))
        .px_2p5()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(CONTROL_RADIUS))
        .bg(bg_control())
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .border_1()
        .border_color(border_control())
        .on_press(on_press)
        .child(
            mono(move || SharedString::from(flow_count_now(rows, flow_total, filter)))
                .tabular_nums()
                .number_of_lines(1)
                .text_size(px(CONTROL_TEXT))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_secondary()),
        )
}

fn more_trigger(
    ca_status: Signal<ca::CaStatus>,
    proxy_on: Signal<bool>,
    update_state: Signal<UpdateState>,
    open: Signal<bool>,
) -> impl IntoElement {
    let dot_status = ca_status;
    let dot_proxy = proxy_on;
    let dot_update = update_state;
    let is_tinted = move || open.get() || proxy_on.get();
    view()
        .window_drag(false)
        .h(px(CONTROL_H))
        .px_2p5()
        .gap_1p5()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(move || {
            if is_tinted() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if is_tinted() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if is_tinted() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.75))
        .child(when(
            move || {
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = (
                        dot_proxy.get(),
                        dot_status.get(),
                        dot_update.get().nudge.is_some(),
                    );
                    false
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    dot_proxy.get()
                        || dot_status.get() != ca::CaStatus::Trusted
                        || dot_update.get().nudge.is_some()
                }
            },
            move || {
                view().w(px(6.0)).h(px(6.0)).rounded_full().bg(move || {
                    if dot_status.get() != ca::CaStatus::Trusted {
                        status_warn_fg()
                    } else {
                        accent_blue()
                    }
                })
            },
        ))
        .child(control_text(tr("更多", "More")).text_color(move || {
            if is_tinted() {
                accent_blue_text()
            } else {
                fg_primary()
            }
        }))
        .child(glyph(
            || Glyph::Chevron,
            move || {
                color(if is_tinted() {
                    accent_blue_text()
                } else {
                    fg_secondary()
                })
            },
        ))
}

#[allow(clippy::too_many_arguments)]
#[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
fn more_menu(
    ca_status: Signal<ca::CaStatus>,
    proxy_on: Signal<bool>,
    install: Rc<impl Fn() + 'static>,
    remove: Rc<impl Fn() + 'static>,
    toggle_proxy: impl Fn() + 'static,
    export_har: impl Fn() + 'static,
    copy_env: impl Fn() + 'static,
    clear: impl Fn() + 'static,
    clear_unpinned: impl Fn() + 'static,
    import_controls: Arc<Controls>,
    rows: Signal<Vec<Arc<RowLine>>>,
    flow_total: Signal<usize>,
    filter: Signal<String>,
    pretty: Signal<bool>,
    shown_phase: Signal<Phase>,
    share_id: Signal<String>,
    compose: Compose,
    seq_builder: SeqBuilder,
    selected: Signal<Option<u64>>,
    update_state: Signal<UpdateState>,
) -> impl IntoElement {
    let export_har = Rc::new(export_har);
    let export_all = Rc::clone(&export_har);
    let export_filtered = Rc::clone(&export_har);
    let items = view()
        .w_full()
        .p_1()
        .gap_0p5()
        .child(menu_action_meta(
            Glyph::Trash,
            tr("清空", "Clear"),
            move || SharedString::from(flow_count_now(rows, flow_total, filter)),
            clear,
        ))
        .child(menu_action(
            Glyph::Trash,
            tr("清空未星标", "Clear unstarred"),
            clear_unpinned,
        ))
        .child(
            when(
                move || !filter.get().is_empty(),
                move || {
                    let export_filtered = Rc::clone(&export_filtered);
                    menu_action_meta(
                        Glyph::Download,
                        tr("导出", "Export"),
                        move || SharedString::from(rows.with(Vec::len).to_string()),
                        move || export_filtered(),
                    )
                },
            )
            .otherwise(move || {
                let export_all = Rc::clone(&export_all);
                menu_action(Glyph::Download, tr("导出", "Export"), move || {
                    export_all()
                })
            }),
        )
        .child(menu_action(
            Glyph::Download,
            tr("导出 HTML", "Export HTML"),
            {
                let controls = import_controls.clone();
                let langs = ui_langs();
                move || {
                    export_html_file(controls.clone(), &filter.get(), ui_lang(langs) == Lang::Zh)
                }
            },
        ))
        .child(menu_action_meta(
            Glyph::Shield,
            tr("导出/分享脱敏", "Redact exports & shares"),
            move || {
                if seq_builder.redact_export.get() {
                    SharedString::from("✓")
                } else {
                    SharedString::default()
                }
            },
            {
                let controls = import_controls.clone();
                move || {
                    let next = !seq_builder.redact_export.get();
                    seq_builder.redact_export.set(next);
                    *controls
                        .redact_export
                        .lock()
                        .unwrap_or_else(|err| err.into_inner()) = next;
                    let _ = save_config(|cfg| cfg.redact_export = next);
                }
            },
        ))
        .child(menu_action(
            Glyph::Prompt,
            tr("脱敏预览与设置", "Redaction preview"),
            {
                let controls = import_controls.clone();
                move || {
                    begin_redact_preview(
                        &controls,
                        rows,
                        seq_builder,
                        compose,
                        selected,
                        shown_phase,
                    );
                }
            },
        ))
        .child(menu_action_enabled(
            Glyph::List,
            tr("生成时序图", "Sequence diagram"),
            move || {
                let len = rows.with(Vec::len);
                if len > 0 {
                    SharedString::from(len.to_string())
                } else {
                    SharedString::default()
                }
            },
            move || !rows.with(Vec::is_empty),
            {
                let controls = import_controls.clone();
                move || {
                    begin_sequence_diagram(
                        &controls,
                        rows,
                        seq_builder,
                        compose,
                        selected,
                        shown_phase,
                    );
                }
            },
        ))
        .child(menu_action(
            Glyph::Prompt,
            tr("分享 15 分钟", "Share 15 min"),
            {
                let controls = import_controls.clone();
                move || begin_share(&controls, 15, &filter.get(), share_id)
            },
        ))
        .child(menu_action(
            Glyph::Prompt,
            tr("分享 1 小时", "Share 1 hour"),
            {
                let controls = import_controls.clone();
                move || begin_share(&controls, 60, &filter.get(), share_id)
            },
        ))
        .child(menu_action(
            Glyph::Prompt,
            tr("分享 8 小时", "Share 8 hours"),
            {
                let controls = import_controls.clone();
                move || begin_share(&controls, 480, &filter.get(), share_id)
            },
        ))
        .child({
            let stop_controls = import_controls.clone();
            when(
                move || !share_id.get().is_empty(),
                move || {
                    let controls = stop_controls.clone();
                    menu_action(
                        Glyph::Trash,
                        tr("停止分享", "Stop sharing"),
                        move || {
                            end_share(&controls, share_id);
                        },
                    )
                },
            )
        });
    let items = desktop_child!(
        items,
        menu_action(Glyph::List, tr("导入 HAR", "Import HAR"), {
            let controls = import_controls.clone();
            move || prompt_import_har(controls.clone())
        })
    );
    let items = items.child(menu_action(
        Glyph::Prompt,
        tr("构造请求", "New request"),
        move || begin_new_request(compose, selected),
    ));
    let items = desktop_child!(
        items,
        menu_action(
            Glyph::Prompt,
            tr("复制代理环境", "Copy proxy env"),
            copy_env,
        )
    );
    let items = desktop_child!(
        items,
        menu_action(Glyph::Prompt, tr("复制 MCP 配置", "Copy MCP config"), {
            let controls = import_controls.clone();
            move || copy_mcp_config(&controls)
        },)
    );
    let items = desktop_child!(items, menu_rule());
    let items = desktop_child!(items, proxy_menu_item(proxy_on, toggle_proxy));
    let items = desktop_child!(items, menu_rule());
    let items = desktop_child!(items, cert_menu_status(ca_status));
    let items = desktop_child!(items, {
        let install_untrusted = install.clone();
        when(
            move || ca_status.get() == ca::CaStatus::InstalledUntrusted,
            move || {
                let install = install_untrusted.clone();
                menu_action(
                    Glyph::Badge,
                    tr("信任证书", "Trust certificate"),
                    move || install.as_ref()(),
                )
            },
        )
    });
    let items = desktop_child!(
        items,
        when(
            move || ca_status.get().is_installed(),
            move || {
                let remove = remove.clone();
                menu_action(
                    Glyph::Badge,
                    tr("移除证书", "Remove certificate"),
                    move || remove.as_ref()(),
                )
            },
        )
        .otherwise(move || {
            let install = install.clone();
            menu_action(
                Glyph::Badge,
                tr("安装证书", "Install certificate"),
                move || install.as_ref()(),
            )
        })
    );
    let items = desktop_child!(items, platform_ca_extra_menu_item(&import_controls));
    let items = items
        .child(menu_rule())
        .child(more_check_action(
            move || pretty.get(),
            tr("默认美化", "Pretty by default"),
            {
                let controls = import_controls.clone();
                move || {
                    let next = !pretty.get();
                    pretty.set(next);
                    if let Err(err) = save_config(|config| config.default_pretty = next) {
                        set_status(
                            &controls,
                            StatusMsg::PrefUnsaved {
                                name: PrefName::Pretty,
                                err: err.to_string(),
                            },
                        );
                    }
                }
            },
        ))
        .child(lang_choice(None, "跟随系统", "Follow system"))
        .child(lang_choice(Some(Lang::Zh), "中文", "中文"))
        .child(lang_choice(Some(Lang::En), "English", "English"));
    let items = desktop_child!(items, menu_rule());
    let items = desktop_child!(items, update_menu_section(update_state, import_controls));
    view()
        .w(px(280.0))
        .max_w(move || px((window().width.0 - 16.0).max(180.0)))
        .max_h(move || px((window().height.0 - 68.0).max(180.0)))
        .overflow_hidden()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
        .child(
            scroll()
                .w_full()
                .max_h(move || px((window().height.0 - 70.0).max(178.0)))
                .child(items),
        )
}

#[cfg(not(target_arch = "wasm32"))]
fn toolbar_update_pill(
    update_state: Signal<UpdateState>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    when(
        move || update_state.get().has_toolbar_pill(),
        move || {
            let controls_press = controls.clone();
            let langs = ui_langs();
            view()
                .window_drag(false)
                .h(px(CONTROL_H))
                .px_2p5()
                .gap_1p5()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .rounded(px(CONTROL_RADIUS))
                .border_1()
                .border_color(accent_blue_border())
                .bg(accent_blue_soft())
                .hover(|style| {
                    style.bg(move || {
                        if update_state.get().is_busy() {
                            accent_blue_soft()
                        } else {
                            accent_blue_soft_hover()
                        }
                    })
                })
                .cursor(Cursor::Pointer)
                .active(|style| style.opacity(0.8))
                .on_press(move || {
                    let st = update_state.get();
                    match st.phase {
                        UpdatePhase::ReadyToRestart { .. } => {
                            relaunch_and_exit(&controls_press);
                        }
                        UpdatePhase::OpenedInstaller { .. } => {
                            quit_for_manual_install(&controls_press);
                        }
                        UpdatePhase::Idle => {
                            if let Some(version) = st.nudge {
                                start_upgrade(controls_press.clone(), version);
                            }
                        }
                        UpdatePhase::Checking
                        | UpdatePhase::Downloading { .. }
                        | UpdatePhase::Installing { .. } => {}
                    }
                })
                .child(glyph(|| Glyph::Download, || color(accent_blue_text())))
                .child(
                    control_text(move || {
                        let lang = ui_lang(langs);
                        let st = update_state.get();
                        let label = match &st.phase {
                            UpdatePhase::Downloading {
                                version,
                                received,
                                total,
                            } => {
                                if *total > 0 {
                                    let pct = ((*received as f64 / *total as f64) * 100.0)
                                        .round()
                                        .clamp(0.0, 100.0)
                                        as u32;
                                    format!("{} {version} · {pct}%", t(lang, "下载", "Downloading"))
                                } else {
                                    format!(
                                        "{} {version} · {}",
                                        t(lang, "下载", "Downloading"),
                                        format_bytes(*received as usize)
                                    )
                                }
                            }
                            UpdatePhase::Installing { version } => {
                                format!("{} {version}…", t(lang, "安装", "Installing"))
                            }
                            UpdatePhase::ReadyToRestart { version } => {
                                format!("{} {version}", t(lang, "重启完成", "Restart for"))
                            }
                            UpdatePhase::OpenedInstaller { .. } => {
                                t(lang, "退出完成安装", "Quit to install").to_string()
                            }
                            UpdatePhase::Idle | UpdatePhase::Checking => {
                                let ver = st.nudge.as_deref().unwrap_or_default();
                                format!("{} {ver}", t(lang, "升级", "Upgrade"))
                            }
                        };
                        SharedString::from(label)
                    })
                    .text_color(accent_blue_text()),
                )
        },
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn update_menu_section(
    update_state: Signal<UpdateState>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let controls_upgrade = controls.clone();
    let controls_skip = controls.clone();
    let controls_restart = controls.clone();
    let controls_quit = controls.clone();
    let controls_unskip = controls.clone();
    let controls_check = controls;
    view()
        .gap_0p5()
        .child(when(
            move || {
                let st = update_state.get();
                matches!(st.phase, UpdatePhase::Idle) && st.nudge.is_some()
            },
            move || {
                let controls_up = controls_upgrade.clone();
                let controls_sk = controls_skip.clone();
                let langs = ui_langs();
                view()
                    .gap_0p5()
                    .child(menu_action_meta(
                        Glyph::Download,
                        move || {
                            let ver = update_state.get().nudge.unwrap_or_default();
                            SharedString::from(format!(
                                "{} {ver}",
                                t(ui_lang(langs), "升级到", "Upgrade to")
                            ))
                        },
                        move || {
                            let ver = update_state.get().nudge.unwrap_or_default();
                            SharedString::from(format!("v{} → {ver}", updater::version()))
                        },
                        move || {
                            if let Some(ver) = update_state.get().nudge {
                                start_upgrade(controls_up.clone(), ver);
                            }
                        },
                    ))
                    .child(menu_action_meta(
                        Glyph::Stop,
                        tr("跳过此版本", "Skip this version"),
                        move || SharedString::from(update_state.get().nudge.unwrap_or_default()),
                        move || {
                            if let Some(ver) = update_state.get().nudge {
                                skip_update(&controls_sk, &ver);
                            }
                        },
                    ))
            },
        ))
        .child(when(
            move || matches!(update_state.get().phase, UpdatePhase::Downloading { .. }),
            move || {
                menu_action_enabled(
                    Glyph::Download,
                    tr("正在下载更新…", "Downloading update…"),
                    move || match update_state.get().phase {
                        UpdatePhase::Downloading {
                            received, total, ..
                        } => {
                            if total > 0 {
                                SharedString::from(format!(
                                    "{} / {}",
                                    format_bytes(received as usize),
                                    format_bytes(total as usize)
                                ))
                            } else {
                                SharedString::from(format_bytes(received as usize))
                            }
                        }
                        _ => SharedString::default(),
                    },
                    || false,
                    || {},
                )
            },
        ))
        .child(when(
            move || matches!(update_state.get().phase, UpdatePhase::Installing { .. }),
            move || {
                menu_action_enabled(
                    Glyph::Download,
                    tr("正在安装更新…", "Installing update…"),
                    move || match update_state.get().phase {
                        UpdatePhase::Installing { version } => SharedString::from(version),
                        _ => SharedString::default(),
                    },
                    || false,
                    || {},
                )
            },
        ))
        .child(when(
            move || matches!(update_state.get().phase, UpdatePhase::ReadyToRestart { .. }),
            move || {
                let controls = controls_restart.clone();
                menu_action_meta(
                    Glyph::Play,
                    tr("立即重启完成更新", "Restart to finish update"),
                    move || match update_state.get().phase {
                        UpdatePhase::ReadyToRestart { version } => SharedString::from(version),
                        _ => SharedString::default(),
                    },
                    move || relaunch_and_exit(&controls),
                )
            },
        ))
        .child(when(
            move || {
                matches!(
                    update_state.get().phase,
                    UpdatePhase::OpenedInstaller { .. }
                )
            },
            move || {
                let controls = controls_quit.clone();
                menu_action_meta(
                    Glyph::Stop,
                    tr("退出以完成安装", "Quit to finish install"),
                    move || match update_state.get().phase {
                        UpdatePhase::OpenedInstaller { version } => SharedString::from(version),
                        _ => SharedString::default(),
                    },
                    move || quit_for_manual_install(&controls),
                )
            },
        ))
        .child(when(
            move || {
                let st = update_state.get();
                matches!(st.phase, UpdatePhase::Idle) && st.nudge.is_none() && st.ignored.is_some()
            },
            move || {
                let controls = controls_unskip.clone();
                menu_action_meta(
                    Glyph::Download,
                    tr("恢复版本提醒", "Unskip update"),
                    move || SharedString::from(update_state.get().ignored.unwrap_or_default()),
                    move || unskip_update(&controls),
                )
            },
        ))
        .child(menu_action_enabled(
            Glyph::Download,
            tr("检查更新", "Check for updates"),
            {
                let langs = ui_langs();
                move || {
                    let st = update_state.get();
                    if matches!(st.phase, UpdatePhase::Checking) {
                        SharedString::from(t(ui_lang(langs), "检查中…", "Checking…"))
                    } else {
                        SharedString::from(format!("v{}", updater::version()))
                    }
                }
            },
            move || !update_state.get().is_busy(),
            move || check_update_manual(controls_check.clone()),
        ))
}

fn menu_action(
    icon: Glyph,
    label: impl IntoReactive<SharedString>,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    menu_item()
        .on_select(on_select)
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.opacity(0.75))
        .cursor(Cursor::Pointer)
        .child(glyph(move || icon, || color(fg_secondary())))
        .child(control_text(label).text_color(fg_secondary()))
}

fn menu_action_meta(
    icon: Glyph,
    label: impl IntoReactive<SharedString>,
    meta: impl Fn() -> SharedString + 'static,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    menu_item()
        .on_select(on_select)
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.opacity(0.75))
        .cursor(Cursor::Pointer)
        .child(glyph(move || icon, || color(fg_secondary())))
        .child(control_text(label).flex_1().text_color(fg_secondary()))
        .child(
            mono(meta)
                .tabular_nums()
                .flex_shrink_0()
                .number_of_lines(1)
                .text_size(px(11.0))
                .text_color(fg_muted()),
        )
}

fn menu_action_enabled(
    icon: Glyph,
    label: impl IntoReactive<SharedString>,
    meta: impl Fn() -> SharedString + 'static,
    enabled: impl Fn() -> bool + 'static + Clone,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    let enabled_off = enabled.clone();
    let enabled_run = enabled.clone();
    let enabled_hover = enabled.clone();
    let enabled_active = enabled.clone();
    let enabled_icon = enabled.clone();
    let enabled_text = enabled;
    menu_item()
        .disabled(move || !enabled_off())
        .disabled_style(|style| style.opacity(0.45).cursor(Cursor::Default))
        .on_select(move || {
            if enabled_run() {
                on_select();
            }
        })
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .hover(|style| {
            style.bg(move || {
                if enabled_hover() {
                    bg_control_hover()
                } else {
                    AdaptiveColor::TRANSPARENT
                }
            })
        })
        .active(|style| style.opacity(move || if enabled_active() { 0.75_f32 } else { 0.45_f32 }))
        .cursor(Cursor::Pointer)
        .child(glyph(
            move || icon,
            move || {
                color(if enabled_icon() {
                    fg_secondary()
                } else {
                    fg_muted()
                })
            },
        ))
        .child(control_text(label).flex_1().text_color(move || {
            if enabled_text() {
                fg_secondary()
            } else {
                fg_muted()
            }
        }))
        .child(
            mono(meta)
                .tabular_nums()
                .flex_shrink_0()
                .number_of_lines(1)
                .text_size(px(11.0))
                .text_color(fg_muted()),
        )
}

fn more_check_action(
    marked: impl Fn() -> bool + 'static + Clone,
    label: impl IntoReactive<SharedString>,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    let dot_border = marked.clone();
    let dot_bg = marked.clone();
    menu_item()
        .on_select(on_select)
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.opacity(0.75))
        .cursor(Cursor::Pointer)
        .child(
            view()
                .w(px(13.0))
                .h(px(13.0))
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .justify_center()
                .child(
                    view()
                        .w(px(10.0))
                        .h(px(10.0))
                        .rounded(px(2.5))
                        .border_1()
                        .border_color(move || {
                            if dot_border() {
                                accent_blue()
                            } else {
                                border_control()
                            }
                        })
                        .bg(move || if dot_bg() { accent_blue() } else { bg_card() }),
                ),
        )
        .child(control_text(label).text_color(move || {
            if marked() {
                accent_blue_text()
            } else {
                fg_secondary()
            }
        }))
}

fn lang_choice(choice: Option<Lang>, zh: &'static str, en: &'static str) -> impl IntoElement {
    let langs = ui_langs();
    let mark = langs;
    menu_item()
        .on_select(move || {
            langs.set(choice);
            let _ = store_lang_pref(choice);
        })
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.opacity(0.75))
        .cursor(Cursor::Pointer)
        .child(
            canvas(move || {
                if mark.get() == choice {
                    Painter::new()
                        .fill_color(color(accent_blue()))
                        .fill_svg("M6.5 6.5 m-3.5 0 a3.5 3.5 0 1 0 7 0 a3.5 3.5 0 1 0 -7 0")
                } else {
                    Painter::new()
                        .line_width(1.25)
                        .stroke_color(color(fg_muted()))
                        .stroke_svg("M6.5 6.5 m-3.5 0 a3.5 3.5 0 1 0 7 0 a3.5 3.5 0 1 0 -7 0")
                }
            })
            .w(px(13.0))
            .h(px(13.0))
            .flex_shrink_0(),
        )
        .child(control_text(tr(zh, en)).text_color(move || {
            if mark.get() == choice {
                accent_blue_text()
            } else {
                fg_secondary()
            }
        }))
}

fn proxy_menu_item(on: Signal<bool>, toggle: impl Fn() + 'static) -> impl IntoElement {
    let bg_on = on;
    let hover_on = on;
    let dot_on = on;
    let label_on = on;
    let state_on = on;
    menu_item()
        .on_select(toggle)
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .bg(move || {
            if bg_on.get() {
                accent_blue_soft()
            } else {
                bg_card()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if hover_on.get() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.75))
        .cursor(Cursor::Pointer)
        .child(glyph(
            || Glyph::Shield,
            move || {
                color(if dot_on.get() {
                    accent_blue_text()
                } else {
                    fg_secondary()
                })
            },
        ))
        .child(
            control_text(tr("系统代理", "System proxy"))
                .flex_1()
                .text_color(move || {
                    if label_on.get() {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(move || {
                    if dot_on.get() {
                        accent_blue()
                    } else {
                        fg_muted()
                    }
                }))
                .child(
                    control_text({
                        let langs = ui_langs();
                        move || -> SharedString {
                            if state_on.get() {
                                t(ui_lang(langs), "开", "On").into()
                            } else {
                                t(ui_lang(langs), "关", "Off").into()
                            }
                        }
                    })
                    .text_color(move || {
                        if state_on.get() {
                            accent_blue_text()
                        } else {
                            fg_muted()
                        }
                    }),
                ),
        )
}

fn cert_menu_status(ca_status: Signal<ca::CaStatus>) -> impl IntoElement {
    let dot_status = ca_status;
    let color_status = ca_status;
    view()
        .h(px(28.0))
        .px_2p5()
        .gap_2()
        .flex_row()
        .items_center()
        .child(glyph(|| Glyph::Badge, || color(fg_muted())))
        .child(
            control_text({
                let langs = ui_langs();
                move || -> SharedString {
                    match ca_status.get() {
                        ca::CaStatus::Trusted => {
                            t(ui_lang(langs), "证书：已信任", "CA: trusted").into()
                        }
                        ca::CaStatus::InstalledUntrusted => {
                            t(ui_lang(langs), "证书：未开启信任", "CA: not trusted").into()
                        }
                        ca::CaStatus::NotInstalled => {
                            t(ui_lang(langs), "证书：未安装", "CA: not installed").into()
                        }
                    }
                }
            })
            .flex_1()
            .text_color(move || match color_status.get() {
                ca::CaStatus::InstalledUntrusted => status_warn_fg(),
                ca::CaStatus::Trusted | ca::CaStatus::NotInstalled => fg_muted(),
            }),
        )
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .flex_shrink_0()
                .bg(move || match dot_status.get() {
                    ca::CaStatus::Trusted => status_ok_fg(),
                    ca::CaStatus::InstalledUntrusted | ca::CaStatus::NotInstalled => {
                        status_warn_fg()
                    }
                }),
        )
}

fn menu_rule() -> impl IntoElement {
    view().h(px(1.0)).mx(px(6.0)).bg(border_strong())
}

fn primary_button(
    label: impl IntoReactive<SharedString>,
    danger: impl Fn() -> bool + 'static + Clone,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let danger_bg = danger.clone();
    let danger_hover = danger.clone();
    let icon_state = danger;
    view()
        .window_drag(false)
        .h(px(CONTROL_H))
        .px_3p5()
        .gap_1p5()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .rounded(px(CONTROL_RADIUS))
        .bg(move || {
            if danger_bg() {
                danger_button_bg()
            } else {
                accent_blue()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if danger_hover() {
                    danger_button_hover()
                } else {
                    accent_blue_hover()
                }
            })
        })
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.8))
        .on_press(on_press)
        .child(glyph(
            move || {
                if icon_state() {
                    Glyph::Stop
                } else {
                    Glyph::Play
                }
            },
            white,
        ))
        .child(
            control_text(label)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(white()),
        )
}

fn mono(content: impl IntoReactive<SharedString>) -> Text {
    text(content).font_family(move || theme().font_mono.clone().unwrap_or_else(|| "Menlo".into()))
}

fn list_status(status: Option<u16>, failed: bool) -> String {
    match status {
        Some(code) => code.to_string(),
        None if failed => "ERR".to_string(),
        None => "—".to_string(),
    }
}

fn list_path(path: &str, error: Option<&str>, failed: bool, lang: Lang) -> String {
    if failed {
        localize_error(lang, error.unwrap_or(""))
    } else {
        path.split('?').next().unwrap_or(path).to_string()
    }
}

fn list_url(url: &str, path: &str, error: Option<&str>, failed: bool, lang: Lang) -> String {
    if failed {
        localize_error(lang, error.unwrap_or(""))
    } else if url.is_empty() {
        path.to_string()
    } else {
        url.to_string()
    }
}

struct CachedRow {
    rev: u64,
    prev: Option<SystemTime>,
    line: Arc<RowLine>,
}

struct RowCache {
    lang: Lang,
    rows: HashMap<u64, CachedRow>,
}

fn same_row_arcs(current: &[Arc<RowLine>], next: &[Arc<RowLine>]) -> bool {
    current.len() == next.len()
        && current
            .iter()
            .zip(next)
            .all(|(left, right)| Arc::ptr_eq(left, right))
}

fn project_rows(
    session: &session::Session,
    matched: &[(u64, Option<SystemTime>)],
    lang: Lang,
    sort: Option<ListSort>,
    full_url: bool,
    cache: &mut RowCache,
) -> Vec<Arc<RowLine>> {
    if cache.lang != lang {
        cache.rows.clear();
        cache.lang = lang;
    }
    let mut lines = Vec::with_capacity(matched.len());
    let mut keep = HashSet::with_capacity(matched.len());
    for (id, previous) in matched {
        keep.insert(*id);
        let Some(flow) = session.get(*id) else {
            continue;
        };
        if let Some(hit) = cache.rows.get(id)
            && hit.rev == flow.rev
            && hit.prev == *previous
        {
            lines.push(Arc::clone(&hit.line));
            continue;
        }
        let failed = flow.error.is_some() && flow.status.is_none();
        let gap_nanos = previous
            .and_then(|prev| flow.started.duration_since(prev).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let elapsed_nanos = flow
            .finished_at
            .and_then(|fin| fin.duration_since(flow.started).ok())
            .map(|duration| duration.as_nanos());
        let line = Arc::new(RowLine {
            id: *id,
            method: flow.method.clone(),
            status: flow.status,
            time: present::format_gap(*previous, flow.started),
            gap_nanos,
            clock: present::format_clock(flow.started),
            elapsed: present::format_elapsed(flow.started, flow.finished_at),
            elapsed_nanos,
            elapsed_slow: flow.finished_at.is_some()
                && present::is_slow(flow.started, flow.finished_at, clock::now()),
            host: flow.host.clone(),
            client_ip: if flow.client_ip.is_empty() {
                "—".to_string()
            } else {
                flow.client_ip.clone()
            },
            path: list_path(&flow.path, flow.error.as_deref(), failed, lang),
            url: list_url(&flow.url, &flow.path, flow.error.as_deref(), failed, lang),
            comment: flow.comment.clone(),
            protocol: flow.protocol.label_in(lang).to_string(),
            size: format_bytes(flow.response_bytes as usize),
            size_bytes: flow.response_bytes,
            failed,
            pinned: flow.pinned,
            relay_short: flow
                .relay_source
                .as_deref()
                .map(session::relay_short)
                .unwrap_or_default(),
        });
        cache.rows.insert(
            *id,
            CachedRow {
                rev: flow.rev,
                prev: *previous,
                line: Arc::clone(&line),
            },
        );
        lines.push(line);
    }
    cache.rows.retain(|id, _| keep.contains(id));
    if let Some(sort) = sort {
        sort_row_lines(&mut lines, sort, full_url);
    }
    lines
}

fn sort_row_lines<T: std::borrow::Borrow<RowLine>>(rows: &mut [T], sort: ListSort, full_url: bool) {
    rows.sort_by(|a, b| {
        let a = a.borrow();
        let b = b.borrow();
        let primary = match sort.column {
            ListColumn::Id => a.id.cmp(&b.id),
            ListColumn::Method => cmp_ignore_ascii_case(&a.method, &b.method),
            ListColumn::Path => {
                cmp_ignore_ascii_case(row_target_text(a, full_url), row_target_text(b, full_url))
            }
            ListColumn::Status => status_sort_rank(a).cmp(&status_sort_rank(b)),
            ListColumn::Gap => a.gap_nanos.cmp(&b.gap_nanos),
            ListColumn::Elapsed => match (a.elapsed_nanos, b.elapsed_nanos) {
                (Some(da), Some(db)) => da.cmp(&db),
                (Some(_), None) => return std::cmp::Ordering::Less,
                (None, Some(_)) => return std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            },
            ListColumn::Size => a.size_bytes.cmp(&b.size_bytes),
            ListColumn::Protocol => cmp_ignore_ascii_case(&a.protocol, &b.protocol),
            ListColumn::Host => cmp_ignore_ascii_case(&a.host, &b.host),
            ListColumn::ClientIp => match (a.client_ip == "—", b.client_ip == "—") {
                (false, false) => cmp_ignore_ascii_case(&a.client_ip, &b.client_ip),
                (false, true) => return std::cmp::Ordering::Less,
                (true, false) => return std::cmp::Ordering::Greater,
                (true, true) => std::cmp::Ordering::Equal,
            },
        };
        let ordered = if sort.desc {
            primary.reverse()
        } else {
            primary
        };
        ordered.then_with(|| a.id.cmp(&b.id))
    });
}

fn cmp_ignore_ascii_case(a: &str, b: &str) -> std::cmp::Ordering {
    a.bytes()
        .map(|byte| byte.to_ascii_lowercase())
        .cmp(b.bytes().map(|byte| byte.to_ascii_lowercase()))
        .then_with(|| a.cmp(b))
}

fn status_sort_rank(row: &RowLine) -> (u8, u16) {
    match (row.status, row.failed) {
        (Some(code), _) => (0, code),
        (None, true) => (1, u16::MAX),
        (None, false) => (2, u16::MAX),
    }
}

fn publish_detail(shown: &Shown, hub: &Hub, id: Option<u64>, lang: Lang, baseline: Option<u64>) {
    let Some(id) = id else {
        clear_shown(*shown);
        return;
    };
    let snapshot = hub.with_session(|session| session.hydrated(id));
    let Some(flow) = snapshot else {
        clear_shown(*shown);
        shown.phase.set(Phase::Gone);
        return;
    };
    let (diff_old, diff_new) = match baseline {
        Some(base) if base != id => hub
            .with_session(|session| session.hydrated(base))
            .map(|base| {
                (
                    present::flow_snapshot(&base, lang),
                    present::flow_snapshot(&flow, lang),
                )
            })
            .unwrap_or_default(),
        _ => (String::new(), String::new()),
    };
    let diff = if diff_old.is_empty() {
        String::new()
    } else {
        "1".to_string()
    };
    let messages = message_lines(&flow, lang);
    let pretty = shown.pretty.get();
    let request_type = present::content_type(&flow.request_headers);
    let response_type = present::content_type(&flow.response_headers);
    let request = display_body(
        &flow.request_body,
        flow.request_truncated,
        flow.request_expired,
        flow.request_encoding_error.as_deref(),
        request_type,
        pretty,
        lang,
        shown.request_offset.get(),
    );
    let response = display_body(
        &flow.response_body,
        flow.response_truncated,
        flow.response_expired,
        flow.response_encoding_error.as_deref(),
        response_type,
        pretty,
        lang,
        shown.response_offset.get(),
    );
    let response_hex = response.hex;
    let request_window = request.window;
    let response_window = response.window;
    let preview = image_preview_path(id, &flow);
    let response = present::image_body(
        pretty,
        !preview.is_empty(),
        response.text,
        response.note,
        flow.response_body.len(),
        lang,
    );
    let ttfb = flow
        .response_at
        .and_then(|at| at.duration_since(flow.started).ok());
    let total = flow
        .finished_at
        .and_then(|at| at.duration_since(flow.started).ok());
    shown.metrics.set(present::format_timing(
        flow.started,
        ttfb,
        total,
        flow.request_bytes,
        flow.response_bytes,
        lang,
    ));
    shown.timing_bar.set(present::flow_timing_breakdown(&flow));
    shown.query.set(
        present::query_pairs(&flow.url)
            .into_iter()
            .enumerate()
            .map(|(index, (name, value))| {
                let hint = present::annotate_value(&name, &value);
                HeaderLine {
                    key: index as u32,
                    name,
                    value,
                    hint,
                }
            })
            .collect(),
    );
    shown.method.set(flow.method);
    shown.url.set(flow.url);
    shown.protocol.set(flow.protocol.label_in(lang).to_string());
    let mut badges = flow.rule_hit.clone();
    if let Some(op_label) = present::graphql_operation_label(&flow.path, &flow.request_body) {
        let tag = format!("gql:{op_label}");
        if badges.is_empty() {
            badges = tag;
        } else {
            badges.push(' ');
            badges.push_str(&tag);
        }
    }
    shown.rules.set(badges);
    shown.status.set(flow.status);
    shown.error.set(flow.error.clone().unwrap_or_default());
    shown
        .request_headers
        .set(header_lines(&flow.request_headers, pretty));
    shown
        .response_headers
        .set(header_lines(&flow.response_headers, pretty));
    let form = if pretty {
        present::multipart_pairs(request_type, &flow.request_body)
            .or_else(|| {
                std::str::from_utf8(&flow.request_body)
                    .ok()
                    .filter(|text| present::is_form(request_type, text))
                    .map(present::form_pairs)
            })
            .map(|pairs| {
                pairs
                    .into_iter()
                    .enumerate()
                    .map(|(index, (name, value))| {
                        let hint = present::annotate_value(&name, &value);
                        HeaderLine {
                            key: index as u32,
                            name,
                            value,
                            hint,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    shown.request_form.set(form);
    shown
        .request_language
        .set(if shown.request_form.with(Vec::is_empty) {
            present::editor_language(request_type, &request.text)
                .unwrap_or("")
                .to_string()
        } else {
            String::new()
        });
    shown.response_language.set(
        present::editor_language(response_type, &response.text)
            .unwrap_or("")
            .to_string(),
    );
    shown.request_doc.set(present::parse_text_view_doc(
        request_type,
        &flow.request_body,
    ));
    shown.response_doc.set(present::parse_text_view_doc(
        response_type,
        &flow.response_body,
    ));
    shown
        .request_body
        .set(if shown.request_form.with(Vec::is_empty) {
            request.text
        } else {
            String::new()
        });
    shown
        .request_hex
        .set(if shown.request_form.with(Vec::is_empty) {
            request.hex
        } else {
            Vec::new()
        });
    shown.response_body.set(response.text);
    shown
        .response_hex
        .set(response_hex_for_view(pretty, &preview, response_hex));
    shown.request_note.set(request.note);
    shown.response_note.set(response.note);
    if shown.request_window.get_untracked() != request_window {
        shown.request_window.set(request_window);
    }
    if shown.response_window.get_untracked() != response_window {
        shown.response_window.set(response_window);
    }
    if shown.request_offset.get_untracked() != request_window.offset {
        shown.request_offset.set(request_window.offset);
    }
    if shown.response_offset.get_untracked() != response_window.offset {
        shown.response_offset.set(response_window.offset);
    }
    let (box_w, box_h) = present::image_pixel_size(&flow.response_body)
        .map(|(w, h)| present::preview_box(w, h))
        .unwrap_or((160, 160));
    shown.preview.set(preview);
    shown.preview_w.set(box_w as f32);
    shown.preview_h.set(box_h as f32);
    shown.image_caption.set(response.caption);
    shown.messages.set(messages);
    shown.diff.set(diff);
    shown.diff_old.set(diff_old);
    shown.diff_new.set(diff_new);
    shown.pinned.set(flow.pinned);
    shown.relay.set(
        flow.relay_source
            .as_deref()
            .map(session::relay_short)
            .unwrap_or_default(),
    );
    if !shown.editing_comment.get() {
        shown.comment.set(flow.comment);
    }
    shown.phase.set(Phase::Ready);
}

fn message_lines(flow: &crate::session::Flow, lang: Lang) -> Vec<HeaderLine> {
    if !flow.messages.is_empty() {
        let mut lines = flow
            .messages
            .iter()
            .enumerate()
            .map(|(index, message)| {
                let who = direction_name(lang, matches!(message.direction, Direction::Request));
                HeaderLine {
                    key: index as u32,
                    name: format!("{who} {}", ws_kind(lang, &message.kind)),
                    value: message.text.clone(),
                    hint: None,
                }
            })
            .collect::<Vec<_>>();
        if flow.messages_truncated {
            let key = lines.len() as u32;
            lines.push(HeaderLine {
                key,
                name: "…".into(),
                value: t(lang, "消息已截断", "Messages truncated").into(),
                hint: None,
            });
        }
        return lines;
    }
    let kind = present::content_type(&flow.response_headers).unwrap_or("");
    if !kind.to_ascii_lowercase().contains("event-stream") {
        return Vec::new();
    }
    let Ok(text) = std::str::from_utf8(&flow.response_body) else {
        return Vec::new();
    };
    // Same cap as WebSocket messages: each event is a view in the message card.
    const SSE_CAP: usize = 200;
    let events = present::sse_events(text);
    let truncated = events.len() > SSE_CAP;
    let mut lines: Vec<HeaderLine> = events
        .into_iter()
        .take(SSE_CAP)
        .enumerate()
        .map(|(index, event)| HeaderLine {
            key: index as u32,
            name: event.event,
            value: event.data,
            hint: None,
        })
        .collect();
    if truncated {
        let key = lines.len() as u32;
        lines.push(HeaderLine {
            key,
            name: "…".into(),
            value: t(lang, "消息已截断", "Messages truncated").into(),
            hint: None,
        });
    }
    lines
}

fn header_lines(headers: &[(String, String)], pretty: bool) -> Vec<HeaderLine> {
    let sorted;
    let headers = if pretty {
        sorted = {
            let mut copy = headers.to_vec();
            present::sort_headers_pretty(&mut copy);
            copy
        };
        sorted.as_slice()
    } else {
        headers
    };
    let mut lines = Vec::new();
    for (name, value) in headers {
        if pretty && let Some(rows) = present::pretty_cookie_rows(name, value) {
            for (row_name, shown) in rows {
                let (raw_val, hint) = match shown.split_once('\n') {
                    Some((v, h)) => (v.to_string(), Some(h.to_string())),
                    None => (shown, None),
                };
                let key = lines.len() as u32;
                lines.push(HeaderLine {
                    key,
                    name: row_name,
                    value: raw_val,
                    hint,
                });
            }
            continue;
        }
        let key = lines.len() as u32;
        lines.push(HeaderLine {
            key,
            name: name.clone(),
            value: value.clone(),
            hint: present::annotate_value(name, value),
        });
    }
    lines
}

/// One page of a UTF-8 body in the detail pane. The record can be up to 1 MB.
const BODY_DISPLAY_CAP: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TextWindow {
    offset: usize,
    end: usize,
    total: usize,
}

impl TextWindow {
    fn has_prev(self) -> bool {
        self.offset > 0
    }

    fn has_more(self) -> bool {
        self.end < self.total
    }
}

struct BodyText {
    text: String,
    note: String,
    /// Set for a binary body. The view breaks it into 16 or 32 byte rows.
    hex: Vec<u8>,
    window: TextWindow,
}

fn display_body(
    body: &[u8],
    truncated: bool,
    expired: Option<usize>,
    encoding_error: Option<&str>,
    content_type: Option<&str>,
    pretty: bool,
    lang: Lang,
    offset: usize,
) -> BodyText {
    let mut presented = body_text(body, truncated, expired, encoding_error, lang, offset);
    if pretty {
        if let Some(decoded) = present::pretty_binary_body(content_type, body) {
            presented.hex.clear();
            presented.note = present::without_hex_cap_note(&presented.note);
            presented.text = decoded;
            presented.window = TextWindow {
                offset: 0,
                end: body.len(),
                total: body.len(),
            };
            return presented;
        }
        if std::str::from_utf8(body).is_ok() {
            presented.hex.clear();
        }
        apply_pretty(&mut presented, body, content_type, lang, offset);
    }
    presented
}

fn apply_pretty(
    presented: &mut BodyText,
    body: &[u8],
    content_type: Option<&str>,
    lang: Lang,
    offset: usize,
) {
    let Ok(text) = std::str::from_utf8(body) else {
        return;
    };
    let pretty = present::pretty_body(content_type, text);
    let (window, slice) = window_text(&pretty, offset);
    presented.text = slice.to_string();
    let extra = if window.has_prev() || window.has_more() {
        Some(window_note(lang, window))
    } else {
        None
    };
    replace_window_note(&mut presented.note, extra);
    presented.window = window;
}

fn window_text(text: &str, offset: usize) -> (TextWindow, &str) {
    let total = text.len();
    if total == 0 {
        return (TextWindow::default(), "");
    }
    let mut start = offset.min(total);
    if start == total {
        start = total.saturating_sub(BODY_DISPLAY_CAP);
    }
    if !text.is_char_boundary(start) {
        start = text.floor_char_boundary(start);
    }
    if start >= total {
        start = text.floor_char_boundary(
            total
                .saturating_sub(1)
                .min(total.saturating_sub(BODY_DISPLAY_CAP)),
        );
    }
    let mut end = start.saturating_add(BODY_DISPLAY_CAP).min(total);
    if end < total && !text.is_char_boundary(end) {
        end = text.floor_char_boundary(end);
    }
    if end <= start {
        end = text[start..]
            .chars()
            .next()
            .map(|ch| start + ch.len_utf8())
            .unwrap_or(total);
    }
    (
        TextWindow {
            offset: start,
            end,
            total,
        },
        &text[start..end],
    )
}

fn window_note(lang: Lang, window: TextWindow) -> String {
    if window.offset == 0 {
        let rest = window.total - window.end;
        return match lang {
            Lang::Zh => format!("界面只显示前 256KB，记录里还有 {rest} 字节。"),
            Lang::En => format!("Showing only the first 256 KB; {rest} more bytes are stored."),
        };
    }
    let start = window.offset + 1;
    match lang {
        Lang::Zh => format!(
            "界面显示第 {start}-{} 字节，共 {} 字节。",
            window.end, window.total
        ),
        Lang::En => format!("Showing bytes {start}-{} of {}.", window.end, window.total),
    }
}

fn is_window_note_line(line: &str) -> bool {
    line.starts_with("界面只显示前 ")
        || line.starts_with("界面显示第 ")
        || line.starts_with("Showing only the first ")
        || line.starts_with("Showing bytes ")
}

fn replace_window_note(note: &mut String, extra: Option<String>) {
    let mut kept = Vec::new();
    for line in note.lines() {
        if !line.is_empty() && !is_window_note_line(line) {
            kept.push(line.to_string());
        }
    }
    if let Some(extra) = extra {
        kept.push(extra);
    }
    *note = kept.join("\n");
}

fn body_text(
    body: &[u8],
    truncated: bool,
    expired: Option<usize>,
    encoding_error: Option<&str>,
    lang: Lang,
    offset: usize,
) -> BodyText {
    let mut note = String::new();
    let mut hex = Vec::new();
    if let Some(error) = encoding_error {
        note.push_str(t(
            lang,
            "解压失败，以下是原始字节。",
            "Decompression failed; showing the raw bytes. ",
        ));
        note.push_str(error);
    }
    if let Some(expired_bytes) = expired {
        if !note.is_empty() {
            note.push('\n');
        }
        let size = format_bytes(expired_bytes);
        note.push_str(&match lang {
            Lang::Zh => format!("大体积 body（{size}）已随临时分片过期或轮转回收。"),
            Lang::En => format!(
                "Large body ({size}) was reclaimed after temporary segment rotation or expiry."
            ),
        });
    }
    let mut window = TextWindow {
        offset: 0,
        end: body.len(),
        total: body.len(),
    };
    let text = if body.is_empty() {
        String::new()
    } else if let Ok(text) = std::str::from_utf8(body) {
        let (slice_window, slice) = window_text(text, offset);
        window = slice_window;
        if window.has_prev() || window.has_more() {
            if !note.is_empty() {
                note.push('\n');
            }
            note.push_str(&window_note(lang, window));
        }
        slice.to_string()
    } else {
        const HEX_CAP: usize = 4096;
        let shown = body.len().min(HEX_CAP);
        hex = body[..shown].to_vec();
        let out = hex_display_rows(&hex, 16);
        if body.len() > shown {
            if !note.is_empty() {
                note.push('\n');
            }
            let total = body.len();
            note.push_str(&match lang {
                Lang::Zh => format!("十六进制只显示前 {shown} 字节，共 {total} 字节。"),
                Lang::En => format!("Hex shows the first {shown} of {total} bytes."),
            });
        }
        out
    };
    if truncated {
        if !note.is_empty() {
            note.push('\n');
        }
        note.push_str(t(
            lang,
            "body 已超过 1 MB，记录被截断，转发仍继续。",
            "The body passed 1 MB. The record is truncated; forwarding continued.",
        ));
    }
    BodyText {
        text,
        note,
        hex,
        window,
    }
}

#[cfg(target_arch = "wasm32")]
fn open_controls() -> Arc<Controls> {
    let hub = Hub::new(Arc::new(Ca::unavailable())).expect("hub");
    Arc::new(Controls {
        hub: Arc::new(hub),
        capture: Mutex::new(None),
        status: Mutex::new(StatusMsg::Idle),
        running: Mutex::new(false),
        listen: Mutex::new(String::new()),
        listen_port: Mutex::new(load_listen_port()),
        ca_status: Mutex::new(ca::CaStatus::NotInstalled),
        use_system_proxy: Mutex::new(false),
        proxy_applied: Mutex::new(false),
        ops: Mutex::new(()),
        listen_all: Mutex::new(false),
        update_state: Mutex::new(UpdateState::default()),
        redact_export: Mutex::new(false),
        redact_strict: Mutex::new(false),
        redact_fields: Mutex::new(String::new()),
        relay_url: Mutex::new(String::new()),
        relay_token: Mutex::new(String::new()),
        toast_seq: std::sync::atomic::AtomicU64::new(0),
        toast_queue: Mutex::new(Vec::new()),
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn open_controls() -> Arc<Controls> {
    let saved = saved_config();
    match Ca::open_default().and_then(|ca| Hub::new(Arc::new(ca))) {
        Ok(hub) => {
            apply_saved_prefs(&hub);
            let ca_status = hub.ca.trust_status();
            Arc::new(Controls {
                hub: Arc::new(hub),
                capture: Mutex::new(None),
                status: Mutex::new(StatusMsg::Idle),
                running: Mutex::new(false),
                listen: Mutex::new(String::new()),
                listen_port: Mutex::new(load_listen_port()),
                ca_status: Mutex::new(ca_status),
                use_system_proxy: Mutex::new(load_use_system_proxy()),
                proxy_applied: Mutex::new(false),
                ops: Mutex::new(()),
                listen_all: Mutex::new(load_listen_all()),
                update_state: Mutex::new(UpdateState::from_disk()),
                redact_export: Mutex::new(saved.redact_export),
                redact_strict: Mutex::new(saved.redact_strict),
                redact_fields: Mutex::new(saved.redact_fields.clone()),
                relay_url: Mutex::new(saved.relay_url.clone()),
                relay_token: Mutex::new(saved.relay_token.clone()),
                relay_push: Mutex::new(None),
                toast_seq: std::sync::atomic::AtomicU64::new(0),
                toast_queue: Mutex::new(Vec::new()),
            })
        }
        Err(err) => {
            // A hub is required for the view. If the CA cannot be created,
            // build one in a temp directory so the window can still explain
            // the failure.
            let dir = std::env::temp_dir().join("zniff-fallback");
            let ca = Arc::new(Ca::open(&dir).expect("临时证书目录"));
            let hub = Hub::new(ca).expect("hub");
            apply_saved_prefs(&hub);
            Arc::new(Controls {
                hub: Arc::new(hub),
                capture: Mutex::new(None),
                status: Mutex::new(StatusMsg::CaUnavailable(format!("{err:#}"))),
                running: Mutex::new(false),
                listen: Mutex::new(String::new()),
                listen_port: Mutex::new(load_listen_port()),
                ca_status: Mutex::new(ca::CaStatus::NotInstalled),
                use_system_proxy: Mutex::new(load_use_system_proxy()),
                proxy_applied: Mutex::new(false),
                ops: Mutex::new(()),
                listen_all: Mutex::new(load_listen_all()),
                update_state: Mutex::new(UpdateState::from_disk()),
                redact_export: Mutex::new(saved.redact_export),
                redact_strict: Mutex::new(saved.redact_strict),
                redact_fields: Mutex::new(saved.redact_fields),
                relay_url: Mutex::new(saved.relay_url),
                relay_token: Mutex::new(saved.relay_token),
                relay_push: Mutex::new(None),
                toast_seq: std::sync::atomic::AtomicU64::new(0),
                toast_queue: Mutex::new(Vec::new()),
            })
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn redact_options(controls: &Controls) -> share::RedactOptions {
    let enabled = lock_bool(&controls.redact_export);
    let strict = lock_bool(&controls.redact_strict);
    let extra = controls
        .redact_fields
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    share::RedactOptions::new(enabled, &extra).with_strict(strict)
}

fn lock_update_state(controls: &Controls) -> UpdateState {
    controls
        .update_state
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

fn with_update_state(controls: &Controls, f: impl FnOnce(&mut UpdateState)) {
    let mut guard = controls
        .update_state
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    f(&mut guard);
}

#[cfg(not(target_arch = "wasm32"))]
fn auto_check_update(controls: Arc<Controls>) {
    if lock_update_state(&controls).is_busy() {
        return;
    }
    std::thread::spawn(move || {
        let outcome = updater::check();
        updater::record_outcome(clock::now(), &outcome);
        with_update_state(&controls, |st| {
            st.nudge = updater::nudge();
            st.ignored = updater::ignored();
        });
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn check_update_manual(controls: Arc<Controls>) {
    {
        let mut st = controls
            .update_state
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if st.is_busy() {
            return;
        }
        st.phase = UpdatePhase::Checking;
    }
    set_status(&controls, StatusMsg::UpdateChecking);
    std::thread::spawn(move || {
        let outcome = updater::check_with_skipped();
        updater::record_outcome(clock::now(), &outcome);
        match &outcome {
            updater::UpdateCheck::UpToDate => {
                set_status(
                    &controls,
                    StatusMsg::UpdateUpToDate(updater::version().to_string()),
                );
            }
            updater::UpdateCheck::Newer { version, .. } => {
                updater::unignore();
                set_status(&controls, StatusMsg::UpdateAvailable(version.clone()));
            }
            updater::UpdateCheck::Failed(err) => {
                set_status(&controls, StatusMsg::UpdateCheckFailed(err.clone()));
            }
        }
        with_update_state(&controls, |st| {
            st.phase = UpdatePhase::Idle;
            st.nudge = updater::nudge();
            st.ignored = updater::ignored();
        });
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn skip_update(controls: &Controls, version: &str) {
    updater::ignore(version);
    with_update_state(controls, |st| {
        st.nudge = updater::nudge();
        st.ignored = updater::ignored();
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn unskip_update(controls: &Controls) {
    updater::unignore();
    with_update_state(controls, |st| {
        st.nudge = updater::nudge();
        st.ignored = updater::ignored();
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn start_upgrade(controls: Arc<Controls>, version: String) {
    {
        let mut st = controls
            .update_state
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if st.is_busy() {
            return;
        }
        st.phase = UpdatePhase::Downloading {
            version: version.clone(),
            received: 0,
            total: 0,
        };
    }
    set_status(
        &controls,
        StatusMsg::UpdateDownloading {
            version: version.clone(),
            progress: "0 B".into(),
        },
    );
    std::thread::spawn(move || {
        let controls_progress = controls.clone();
        let ver_progress = version.clone();
        let mut last_step = u64::MAX;
        let downloaded = updater::download(&version, move |received, total| {
            let step = received / (256 * 1024);
            if step != last_step || (total > 0 && received == total) {
                last_step = step;
                with_update_state(&controls_progress, |st| {
                    st.phase = UpdatePhase::Downloading {
                        version: ver_progress.clone(),
                        received,
                        total,
                    };
                });
                let progress = if total > 0 {
                    format!(
                        "{} / {}",
                        format_bytes(received as usize),
                        format_bytes(total as usize)
                    )
                } else {
                    format_bytes(received as usize)
                };
                set_status(
                    &controls_progress,
                    StatusMsg::UpdateDownloading {
                        version: ver_progress.clone(),
                        progress,
                    },
                );
            }
        });
        let path = match downloaded {
            Ok(path) => path,
            Err(err) => {
                with_update_state(&controls, |st| {
                    st.phase = UpdatePhase::Idle;
                });
                set_status(&controls, StatusMsg::UpdateFailed { version, err });
                return;
            }
        };
        with_update_state(&controls, |st| {
            st.phase = UpdatePhase::Installing {
                version: version.clone(),
            };
        });
        set_status(&controls, StatusMsg::UpdateInstalling(version.clone()));
        match updater::install(&path) {
            Ok(updater::Delivery::Replaced) => {
                with_update_state(&controls, |st| {
                    st.phase = UpdatePhase::ReadyToRestart {
                        version: version.clone(),
                    };
                });
                set_status(&controls, StatusMsg::UpdateRestarting(version));
                relaunch_and_exit(&controls);
            }
            Ok(updater::Delivery::OpenedForDrag) => {
                with_update_state(&controls, |st| {
                    st.phase = UpdatePhase::OpenedInstaller {
                        version: version.clone(),
                    };
                });
                set_status(&controls, StatusMsg::UpdateOpenedInstaller(version));
            }
            Err(err) => {
                with_update_state(&controls, |st| {
                    st.phase = UpdatePhase::Idle;
                });
                set_status(&controls, StatusMsg::UpdateFailed { version, err });
            }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn shutdown_capture_for_exit(controls: &Controls) {
    let _ops = lock_ops(controls);
    if let Some(capture) = controls
        .capture
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
    {
        capture.stop();
    }
    if let Some(handle) = controls
        .relay_push
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
    {
        handle.stop_and_wait(Duration::from_secs(3));
    }
    relay::shutdown_relay_push();
    controls.hub.end_share();
    if lock_bool(&controls.proxy_applied) {
        set_bool(&controls.proxy_applied, false);
        let _ = sysproxy::disable();
    } else {
        let _ = sysproxy::restore_if_dirty();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn relaunch_and_exit(controls: &Controls) {
    shutdown_capture_for_exit(controls);
    updater::relaunch();
    std::process::exit(0);
}

#[cfg(not(target_arch = "wasm32"))]
fn quit_for_manual_install(controls: &Controls) {
    shutdown_capture_for_exit(controls);
    std::process::exit(0);
}

#[cfg(not(target_arch = "wasm32"))]
fn install_ca(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let ca = controls.hub.ca.clone();
        match ca::install(&ca) {
            Ok(_) => set_ca_status(&controls, ca::trust_status(&ca)),
            Err(err) => {
                set_ca_status(&controls, ca::trust_status(&ca));
                set_status(&controls, StatusMsg::InstallFailed(format!("{err:#}")));
            }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn remove_ca(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let ca = controls.hub.ca.clone();
        match ca::remove(&ca) {
            Ok(_) => set_ca_status(&controls, ca::trust_status(&ca)),
            Err(err) => {
                set_ca_status(&controls, ca::trust_status(&ca));
                set_status(&controls, StatusMsg::RemoveFailed(format!("{err:#}")));
            }
        }
    });
}

#[cfg(target_os = "macos")]
fn platform_ca_extra_menu_item(controls: &Arc<Controls>) -> impl IntoElement {
    let controls = controls.clone();
    menu_action(
        Glyph::Badge,
        tr("安装证书到 iOS 模拟器", "Install on iOS Simulator"),
        move || install_ca_ios_sim(controls.clone()),
    )
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_ca_extra_menu_item(controls: &Arc<Controls>) -> impl IntoElement {
    let controls = controls.clone();
    menu_action(
        Glyph::Prompt,
        tr("复制系统证书命令", "Copy system CA command"),
        move || copy_linux_ca_cmd(&controls),
    )
}

#[cfg(not(unix))]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn platform_ca_extra_menu_item(_controls: &Arc<Controls>) -> impl IntoElement {
    view().w(px(0.0)).h(px(0.0))
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn install_ca_ios_sim(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let ca = controls.hub.ca.clone();
        let port = current_listen_port(&controls);
        match ca::install_ios_simulators(&ca, port) {
            Ok(ca::IosSimOutcome::Installed { count, port }) => {
                set_status(&controls, StatusMsg::IosSimInstalled { count, port });
            }
            Ok(ca::IosSimOutcome::NoneBooted) => {
                set_status(&controls, StatusMsg::IosSimNoneBooted);
            }
            Err(err) => {
                set_status(&controls, StatusMsg::IosSimFailed(format!("{err:#}")));
            }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
fn copy_linux_ca_cmd(controls: &Controls) {
    let cmd = ca::linux_system_ca_command(&controls.hub.ca.cert_path());
    set_clipboard(cmd.clone());
    set_status(controls, StatusMsg::CopiedLinuxCaCmd(cmd));
}

fn set_ca_status(controls: &Controls, status: ca::CaStatus) {
    *controls
        .ca_status
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = status;
}

fn lock_ca_status(controls: &Controls) -> ca::CaStatus {
    *controls
        .ca_status
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

fn set_status(controls: &Controls, message: StatusMsg) {
    if message.toast_kind().is_some() {
        let id = controls
            .toast_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        controls
            .toast_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push((id, message.clone()));
    }
    *controls
        .status
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = message;
}

fn lock_status(controls: &Controls) -> StatusMsg {
    controls
        .status
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

#[cfg(target_arch = "wasm32")]
fn install_ca(_controls: Arc<Controls>) {}

#[cfg(target_arch = "wasm32")]
fn remove_ca(_controls: Arc<Controls>) {}

#[cfg(target_arch = "wasm32")]
fn toggle_system_proxy(_controls: Arc<Controls>) {}

#[cfg(not(target_arch = "wasm32"))]
fn toggle_capture(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let running = *controls
            .running
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if running {
            stop_capture(&controls);
        } else {
            start_capture(&controls);
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn toggle_capture(controls: Arc<Controls>) {
    let running = lock_bool(&controls.running);
    let listen_all = lock_bool(&controls.listen_all);
    let port = lock_port(&controls.listen_port);
    spawn(async move {
        let body = if running {
            b"{\"on\":false}".to_vec()
        } else {
            serde_json::json!({"on": true, "listen_all": listen_all, "port": port})
                .to_string()
                .into_bytes()
        };
        let request =
            HttpRequest::post("/api/capture", body).header("content-type", "application/json");
        match fetch(request).await {
            Ok(response) => {
                let value: serde_json::Value =
                    serde_json::from_str(&response.text()).unwrap_or(serde_json::json!({}));
                apply_capture_state(&controls, &value, response.ok());
            }
            Err(err) => set_status(&controls, StatusMsg::BindFailed { port, err }),
        }
    });
}

/// The browser does not capture. This loop copies the server process into the
/// same signals the desktop tick already reads.
#[cfg(target_arch = "wasm32")]
fn start_web_poll(
    controls: Arc<Controls>,
    port_text: Signal<String>,
    listen_all: Signal<bool>,
    profiles: ProfileSignals,
    effective_filter: Signal<String>,
    seq_builder: SeqBuilder,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
) {
    spawn(async move {
        let mut seeded = false;
        let mut seen_error = String::new();
        loop {
            if let Ok(response) = fetch(HttpRequest::get("/api/state")).await
                && let Ok(value) = serde_json::from_str::<serde_json::Value>(&response.text())
            {
                controls.hub.apply_state(&value);
                if let Some(on) = value
                    .get("prefs")
                    .and_then(|prefs| prefs.get("relay_accept"))
                    .and_then(serde_json::Value::as_bool)
                    && profiles.relay_accept.get_untracked() != on
                {
                    profiles.relay_accept.set(on);
                }
                if !seeded {
                    seeded = true;
                    if let Some(port) = json_port(&value) {
                        set_port(&controls.listen_port, port);
                        port_text.set(port.to_string());
                    }
                    if let Some(all) = value.get("listen_all").and_then(serde_json::Value::as_bool)
                    {
                        set_bool(&controls.listen_all, all);
                        listen_all.set(all);
                    }
                    if let Some(prefs) = value.get("prefs") {
                        seed_web_prefs(&controls, prefs, profiles, seq_builder);
                    }
                }
                let capturing = value
                    .get("capturing")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                set_bool(&controls.running, capturing);
                let listen = value
                    .get("listen")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                *controls
                    .listen
                    .lock()
                    .unwrap_or_else(|err| err.into_inner()) = listen.to_string();
                let error = value
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                if !error.is_empty() && error != seen_error {
                    seen_error = error.to_string();
                    set_status(
                        &controls,
                        StatusMsg::BindFailed {
                            port: json_port(&value).unwrap_or(0),
                            err: error.to_string(),
                        },
                    );
                } else if error.is_empty() {
                    seen_error.clear();
                }
            }
            let flows_url = {
                let f = effective_filter.get_untracked();
                let trimmed = f.trim();
                if trimmed.is_empty() {
                    "/api/flows".to_string()
                } else {
                    format!("/api/flows?filter={}", url_encode_query_component(trimmed))
                }
            };
            if let Ok(response) = fetch(HttpRequest::get(flows_url)).await
                && let Ok(value) = serde_json::from_str::<serde_json::Value>(&response.text())
            {
                controls.hub.apply_flows(&value);
            }
            let picked = selected.get_untracked();
            let base = baseline.get_untracked();
            for maybe_id in [picked, if base != picked { base } else { None }] {
                let Some(id) = maybe_id else {
                    continue;
                };
                let needs = controls
                    .hub
                    .with_session(|session| session.cached_needs_detail(id));
                if needs
                    && let Ok(response) = fetch(HttpRequest::get(format!("/api/flows/{id}"))).await
                    && response.ok()
                    && let Ok(value) = serde_json::from_str::<serde_json::Value>(&response.text())
                {
                    controls.hub.fill_detail(&value);
                }
            }
            timer(Duration::from_millis(400)).await;
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn seed_web_prefs(
    controls: &Controls,
    prefs: &serde_json::Value,
    profiles: ProfileSignals,
    seq_builder: SeqBuilder,
) {
    let mut cfg = web_config_slot()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    if let Some(text) = prefs.get("bypass").and_then(serde_json::Value::as_str) {
        cfg.bypass_hosts = text.to_string();
        profiles.bypass.set(text.to_string());
    }
    if let Some(text) = prefs.get("rules").and_then(serde_json::Value::as_str) {
        cfg.rules = text.to_string();
        profiles.rules.set(text.to_string());
    }
    if let Some(text) = prefs.get("upstream").and_then(serde_json::Value::as_str) {
        cfg.upstream_proxy = text.to_string();
        profiles.upstream.set(text.to_string());
    }
    if let Some(on) = prefs
        .get("rules_enabled")
        .and_then(serde_json::Value::as_bool)
    {
        cfg.rules_enabled = on;
        profiles.rules_on.set(on);
    }
    if let Some(on) = prefs
        .get("breaks_enabled")
        .and_then(serde_json::Value::as_bool)
    {
        cfg.breaks_enabled = on;
        profiles.breaks_on.set(on);
    }
    if let Some(on) = prefs
        .get("show_tunnels")
        .and_then(serde_json::Value::as_bool)
    {
        cfg.show_tunnel_failures = on;
        profiles.show_tunnels.set(on);
    }
    if let Some(text) = prefs.get("filter").and_then(serde_json::Value::as_str) {
        cfg.filter = text.to_string();
        profiles.filter.set(text.to_string());
    }
    if let Some(items) = prefs
        .get("saved_profiles")
        .and_then(|v| serde_json::from_value::<Vec<config::SavedProfile>>(v.clone()).ok())
    {
        cfg.saved_profiles = items.clone();
        profiles.saved.set(items);
    }
    if let Some(items) = prefs
        .get("profile_history")
        .and_then(|v| serde_json::from_value::<Vec<config::SavedProfile>>(v.clone()).ok())
    {
        cfg.profile_history = items.clone();
        profiles.history.set(items);
    }
    if let Some(active) = prefs
        .get("active_profile")
        .and_then(serde_json::Value::as_str)
    {
        cfg.active_profile = active.to_string();
        let initial_name = if !active.trim().is_empty() {
            active.to_string()
        } else {
            matched_saved_profile(
                &cfg.saved_profiles,
                &cfg.filter,
                &cfg.bypass_hosts,
                &cfg.rules,
                &cfg.upstream_proxy,
            )
            .unwrap_or_default()
        };
        profiles.name.set(initial_name);
    }
    if let Some(on) = prefs
        .get("relay_accept")
        .and_then(serde_json::Value::as_bool)
    {
        cfg.relay_accept = on;
        profiles.relay_accept.set(on);
    }
    if let Some(auth) = prefs.get("proxy_auth").and_then(serde_json::Value::as_str) {
        cfg.proxy_auth = auth.to_string();
        profiles.proxy_auth.set(auth.to_string());
    }
    if let Some(url) = prefs.get("relay_url").and_then(serde_json::Value::as_str) {
        cfg.relay_url = url.to_string();
        profiles.relay_url.set(url.to_string());
        *controls
            .relay_url
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = url.to_string();
    }
    if let Some(on) = prefs
        .get("redact_export")
        .and_then(serde_json::Value::as_bool)
    {
        cfg.redact_export = on;
        seq_builder.redact_export.set(on);
        set_bool(&controls.redact_export, on);
    }
    if let Some(on) = prefs
        .get("redact_strict")
        .and_then(serde_json::Value::as_bool)
    {
        cfg.redact_strict = on;
        seq_builder.redact_strict.set(on);
        set_bool(&controls.redact_strict, on);
    }
    if let Some(fields) = prefs
        .get("redact_fields")
        .and_then(serde_json::Value::as_str)
    {
        cfg.redact_fields = fields.to_string();
        seq_builder.redact_fields.set(fields.to_string());
        *controls
            .redact_fields
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = fields.to_string();
    }
    *web_config_slot()
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = cfg;
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn url_encode_query_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for &b in text.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(target_arch = "wasm32")]
fn json_port(value: &serde_json::Value) -> Option<u16> {
    value
        .get("listen_port")
        .and_then(serde_json::Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
}

#[cfg(target_arch = "wasm32")]
fn apply_capture_state(controls: &Controls, value: &serde_json::Value, http_ok: bool) {
    let capturing = value
        .get("capturing")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let listen = value
        .get("listen")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let port = json_port(value);
    let error = value
        .get("error")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    set_bool(&controls.running, capturing);
    *controls
        .listen
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = listen;
    if let Some(port) = port {
        set_port(&controls.listen_port, port);
    }
    if let Some(all) = value.get("listen_all").and_then(serde_json::Value::as_bool) {
        set_bool(&controls.listen_all, all);
    }
    if capturing {
        set_status(
            controls,
            StatusMsg::Listening {
                port: port.unwrap_or(0),
                via_system_proxy: false,
            },
        );
    } else if !http_ok || !error.is_empty() {
        set_status(
            controls,
            StatusMsg::BindFailed {
                port: port.unwrap_or(0),
                err: if error.is_empty() {
                    "没有开始".to_string()
                } else {
                    error.to_string()
                },
            },
        );
    } else {
        set_status(controls, StatusMsg::Stopped);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn start_capture(controls: &Controls) {
    let _ops = lock_ops(controls);
    if lock_bool(&controls.running) {
        return;
    }
    set_status(controls, StatusMsg::Starting);
    let port = lock_port(&controls.listen_port);
    let ip = if lock_bool(&controls.listen_all) {
        [0, 0, 0, 0]
    } else {
        [127, 0, 0, 1]
    };
    let addr = SocketAddr::from((ip, port));
    let capture = match Capture::spawn(controls.hub.clone(), addr) {
        Ok(capture) => capture,
        Err(err) => {
            set_status(
                controls,
                StatusMsg::BindFailed {
                    port,
                    err: format!("{err:#}"),
                },
            );
            return;
        }
    };
    let port = capture.addr.port();
    set_port(&controls.listen_port, port);
    let remembered = store_listen_port(port);
    *controls
        .listen
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = capture.addr.to_string();
    *controls
        .capture
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = Some(capture);
    set_bool(&controls.running, true);
    if lock_bool(&controls.use_system_proxy) {
        apply_system_proxy(controls);
    } else {
        set_bool(&controls.proxy_applied, false);
        set_status(
            controls,
            StatusMsg::Listening {
                port,
                via_system_proxy: false,
            },
        );
    }
    if let Err(err) = remembered {
        let base = lock_status(controls);
        set_status(
            controls,
            StatusMsg::PortUnsaved {
                base: Box::new(base),
                err: err.to_string(),
            },
        );
    }
    remember_capturing(controls, true);
}

#[cfg(not(target_arch = "wasm32"))]
fn stop_capture(controls: &Controls) {
    let _ops = lock_ops(controls);
    if let Some(capture) = controls
        .capture
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
    {
        capture.stop();
    }
    stop_relay_push(controls);
    controls.hub.end_share();
    ca::clear_ios_simulator_proxies();
    *controls
        .listen
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = String::new();
    let applied = lock_bool(&controls.proxy_applied);
    set_bool(&controls.proxy_applied, false);
    set_bool(&controls.running, false);
    if !applied {
        set_status(controls, StatusMsg::Stopped);
    } else {
        match sysproxy::disable() {
            Ok(()) => set_status(controls, StatusMsg::StoppedRestored),
            Err(err) => set_status(controls, StatusMsg::StopRestoreFailed(format!("{err:#}"))),
        }
    }
    remember_capturing(controls, false);
}

#[cfg(not(target_arch = "wasm32"))]
fn remember_capturing(controls: &Controls, on: bool) {
    if let Err(err) = save_config(|config| config.capturing = on) {
        set_status(
            controls,
            StatusMsg::PrefUnsaved {
                name: PrefName::Capture,
                err: err.to_string(),
            },
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn toggle_system_proxy(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let _ops = lock_ops(&controls);
        let on = !lock_bool(&controls.use_system_proxy);
        set_bool(&controls.use_system_proxy, on);
        let running = lock_bool(&controls.running);
        if let Err(err) = store_use_system_proxy(on)
            && !running
        {
            set_status(&controls, StatusMsg::ProxyOptionUnsaved(err.to_string()));
            return;
        }
        if !running {
            return;
        }
        if on {
            apply_system_proxy(&controls);
        } else {
            clear_system_proxy(&controls);
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn apply_system_proxy(controls: &Controls) {
    let Some(port) = capture_port(controls) else {
        set_bool(&controls.proxy_applied, false);
        return;
    };
    match sysproxy::enable(port) {
        Ok(sysproxy::EnableOutcome::Applied) => {
            set_bool(&controls.proxy_applied, true);
            set_status(
                controls,
                StatusMsg::Listening {
                    port,
                    via_system_proxy: true,
                },
            );
        }
        Ok(sysproxy::EnableOutcome::Manual { reason }) => {
            set_bool(&controls.proxy_applied, false);
            set_status(controls, StatusMsg::ProxyManual(reason));
        }
        Err(err) => {
            set_bool(&controls.proxy_applied, false);
            set_status(
                controls,
                StatusMsg::ProxyApplyFailed {
                    port,
                    err: format!("{err:#}"),
                },
            );
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn clear_system_proxy(controls: &Controls) {
    let port = capture_port(controls);
    if !lock_bool(&controls.proxy_applied) {
        if let Some(port) = port {
            set_status(
                controls,
                StatusMsg::Listening {
                    port,
                    via_system_proxy: false,
                },
            );
        }
        return;
    }
    match sysproxy::disable() {
        Ok(()) => {
            set_bool(&controls.proxy_applied, false);
            if let Some(port) = port {
                set_status(
                    controls,
                    StatusMsg::Listening {
                        port,
                        via_system_proxy: false,
                    },
                );
            }
        }
        Err(err) => {
            set_bool(&controls.use_system_proxy, true);
            let _ = store_use_system_proxy(true);
            set_status(controls, StatusMsg::ProxyRestoreFailed(format!("{err:#}")));
        }
    }
}

fn capture_port(controls: &Controls) -> Option<u16> {
    controls
        .capture
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .as_ref()
        .map(|capture| capture.addr.port())
}

fn lock_ops(controls: &Controls) -> std::sync::MutexGuard<'_, ()> {
    controls.ops.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(target_arch = "wasm32")]
fn web_config_slot() -> &'static Mutex<config::Config> {
    static SLOT: std::sync::OnceLock<Mutex<config::Config>> = std::sync::OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(config::Config::default_for(false)))
}

#[cfg(not(target_arch = "wasm32"))]
fn saved_config() -> config::Config {
    config::load_mode(&ca::data_dir(), config::is_dev_mode())
}

#[cfg(target_arch = "wasm32")]
fn saved_config() -> config::Config {
    web_config_slot()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

#[cfg(not(target_arch = "wasm32"))]
fn save_config(change: impl FnOnce(&mut config::Config)) -> std::io::Result<()> {
    config::update_mode(&ca::data_dir(), config::is_dev_mode(), change)
}

#[cfg(target_arch = "wasm32")]
fn save_config(change: impl FnOnce(&mut config::Config)) -> std::io::Result<()> {
    let (before, after) = {
        let mut guard = web_config_slot()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let before = guard.clone();
        change(&mut guard);
        let after = guard.clone();
        (before, after)
    };
    proxy::sync_web_config_diff(&before, &after);
    Ok(())
}

fn load_use_system_proxy() -> bool {
    saved_config().use_system_proxy
}

fn store_use_system_proxy(on: bool) -> std::io::Result<()> {
    save_config(|config| config.use_system_proxy = on)
}

fn load_listen_port() -> u16 {
    saved_config().listen_port
}

fn store_listen_port(port: u16) -> std::io::Result<()> {
    save_config(|config| config.listen_port = port)
}

fn current_listen_port(controls: &Controls) -> u16 {
    if let Some(capture) = controls
        .capture
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .as_ref()
    {
        return capture.addr.port();
    }
    lock_port(&controls.listen_port)
}

fn lock_port(slot: &Mutex<u16>) -> u16 {
    *slot.lock().unwrap_or_else(|err| err.into_inner())
}

fn set_port(slot: &Mutex<u16>, port: u16) {
    *slot.lock().unwrap_or_else(|err| err.into_inner()) = port;
}

#[derive(Clone, Copy)]
enum Snippet {
    Fetch,
    Python,
    RustReqwest,
    GoHttp,
    Types,
    RustStruct,
}

fn copy_proxy_env(controls: &Controls) {
    let port = lock_port(&controls.listen_port);
    let cert = controls.hub.ca.cert_path();
    let auth = controls.hub.proxy_auth_text();
    set_clipboard(present::proxy_env_export_with_auth(
        port,
        &cert.display().to_string(),
        &auth,
    ));
    set_status(controls, StatusMsg::CopiedEnv);
}

#[cfg(not(target_arch = "wasm32"))]
fn copy_mcp_config(controls: &Controls) {
    let port = lock_port(&controls.listen_port);
    set_clipboard(mcp::mcp_config_json(port));
    set_status(controls, StatusMsg::CopiedMcpConfig(port));
}

fn copy_snippet(controls: &Controls, id: Option<u64>, kind: Snippet) {
    let Some(flow) = selected_flow(controls, id) else {
        return;
    };
    match kind {
        Snippet::Fetch => {
            set_clipboard(present::fetch_snippet(&flow));
            set_status(controls, StatusMsg::CopiedFetch);
        }
        Snippet::Python => {
            set_clipboard(present::python_requests(&flow));
            set_status(controls, StatusMsg::CopiedPython);
        }
        Snippet::RustReqwest => {
            set_clipboard(present::rust_reqwest(&flow));
            set_status(controls, StatusMsg::CopiedRustReqwest);
        }
        Snippet::GoHttp => {
            set_clipboard(present::go_http(&flow));
            set_status(controls, StatusMsg::CopiedGoHttp);
        }
        Snippet::Types => match std::str::from_utf8(&flow.response_body)
            .ok()
            .and_then(present::typescript_interface)
        {
            Some(text) => {
                set_clipboard(text);
                set_status(controls, StatusMsg::CopiedInterface);
            }
            None => set_status(controls, StatusMsg::InterfaceUnavailable),
        },
        Snippet::RustStruct => match std::str::from_utf8(&flow.response_body)
            .ok()
            .and_then(present::rust_serde_struct)
        {
            Some(text) => {
                set_clipboard(text);
                set_status(controls, StatusMsg::CopiedRustStruct);
            }
            None => set_status(controls, StatusMsg::InterfaceUnavailable),
        },
    }
}

fn focus_host(controls: &Controls, id: Option<u64>, filter: Signal<String>) {
    let Some(flow) = selected_flow(controls, id) else {
        return;
    };
    let host = flow.host.trim();
    if host.is_empty() {
        return;
    }
    let current = filter.get();
    let existing = split_ui_or_items(&extract_host_or(&current));
    if existing.len() == 1
        && existing[0].eq_ignore_ascii_case(host)
        && current.trim().starts_with("host:")
    {
        return;
    }
    let next_hosts = if existing.is_empty() {
        host.to_string()
    } else if existing.iter().any(|item| item.eq_ignore_ascii_case(host)) {
        existing.join(" | ")
    } else {
        let mut combined = existing;
        combined.push(host.to_string());
        combined.join(" | ")
    };
    filter.set(set_host_or(&current, &next_hosts));
}

fn mute_host(controls: &Controls, id: Option<u64>, filter: Signal<String>) {
    let Some(flow) = selected_flow(controls, id) else {
        return;
    };
    let host = flow.host.trim().to_ascii_lowercase();
    if host.is_empty() || host.split_whitespace().nth(1).is_some() {
        return;
    }
    let current = filter.get();
    let mut existing = split_ui_or_items(&extract_exclude_or(&current));
    if existing.iter().any(|item| item.eq_ignore_ascii_case(&host)) {
        return;
    }
    existing.push(host);
    filter.set(set_exclude_or(&current, &existing.join(" | ")));
}

fn toggle_pin(controls: &Controls, id: Option<u64>) {
    let Some(flow) = selected_flow(controls, id) else {
        return;
    };
    let next = !flow.pinned;
    if controls.hub.set_pinned(flow.id, next) {
        return;
    }
    set_status(
        controls,
        if next {
            StatusMsg::PinLimit
        } else {
            StatusMsg::FlowGone
        },
    );
}

fn set_baseline(
    controls: &Controls,
    id: Option<u64>,
    baseline: Signal<Option<u64>>,
    shown: &Shown,
    lang: Lang,
) {
    let Some(id) = id else {
        set_status(controls, StatusMsg::NeedSelection);
        return;
    };
    if selected_flow(controls, Some(id)).is_none() {
        return;
    }
    if baseline.get() == Some(id) {
        baseline.set(None);
        set_status(controls, StatusMsg::BaselineCleared);
        publish_detail(shown, &controls.hub, Some(id), lang, None);
        return;
    }
    baseline.set(Some(id));
    set_status(controls, StatusMsg::BaselineSet);
    publish_detail(shown, &controls.hub, Some(id), lang, Some(id));
}

fn copy_part(controls: &Controls, id: Option<u64>, part: CopyPart) {
    let Some(flow) = selected_flow(controls, id) else {
        return;
    };
    let (text, note) = match part {
        CopyPart::Url => (flow.url, StatusMsg::CopiedUrl),
        CopyPart::Curl => {
            let binary =
                !flow.request_body.is_empty() && std::str::from_utf8(&flow.request_body).is_err();
            (
                present::curl_command(&flow),
                StatusMsg::CopiedCurl { binary },
            )
        }
        CopyPart::Request => bytes_for_copy(&flow.request_body, Side::Request),
        CopyPart::Response => bytes_for_copy(&flow.response_body, Side::Response),
    };
    set_clipboard(text);
    set_status(controls, note);
}

fn bytes_for_copy(body: &[u8], side: Side) -> (String, StatusMsg) {
    if body.is_empty() {
        return (String::new(), StatusMsg::EmptyCopy(side));
    }
    if let Ok(text) = std::str::from_utf8(body) {
        (text.to_string(), StatusMsg::CopiedBody { side, hex: false })
    } else {
        (hex_dump(body), StatusMsg::CopiedBody { side, hex: true })
    }
}

/// `0001  ` plus `per_row` bytes. A 32-byte row leaves one extra space after
/// the 16th byte. Copy uses `hex_dump`, which has no row numbers.
fn hex_display_rows(body: &[u8], per_row: usize) -> String {
    let per_row = per_row.max(1);
    let mut out = String::with_capacity(body.len() * 4);
    for (index, byte) in body.iter().enumerate() {
        if index % per_row == 0 {
            if index > 0 {
                out.push('\n');
            }
            let row = index / per_row + 1;
            out.push_str(&format!("{row:04}  "));
        } else if per_row > 16 && index % 16 == 0 {
            out.push(' ');
        }
        out.push_str(&format!("{byte:02x} "));
    }
    out
}

/// 12px Menlo is about 7.25px wide. A row is `0001  ` plus `xx ` per byte,
/// and a 32-byte row adds one space between the two halves.
fn hex_line_width(bytes: usize) -> f32 {
    const CHAR: f32 = 7.25;
    let gaps = if bytes > 16 {
        (bytes / 16 - 1) as f32
    } else {
        0.0
    };
    (6.0 + 3.0 * bytes as f32 + gaps) * CHAR
}

/// Use 32 only when that line fits; otherwise stay at 16.
fn hex_row_bytes(width: f32) -> usize {
    if width + 0.01 >= hex_line_width(32) {
        32
    } else {
        16
    }
}

/// `ready_detail` padding (`pl_3p5` + `pr_5`) and the card border, between the pane and the hex area.
const HEX_AREA_CHROME: f32 = 14.0 + 20.0 + 2.0;
/// Horizontal padding of the hex text (`pl_3p5` + `pr_5`).
const HEX_TEXT_PAD: f32 = 14.0 + 20.0;

/// Width the hex area can use inside a detail pane of `pane` points.
fn hex_area_width(pane: f32) -> f32 {
    (pane - HEX_AREA_CHROME).max(0.0)
}

/// Columns once the pane has been shown. Zero stays at 16.
fn hex_cols_for_pane(pane: f32) -> usize {
    if pane <= 0.0 {
        16
    } else {
        hex_row_bytes(hex_area_width(pane) - HEX_TEXT_PAD)
    }
}

/// Keeps raw hex bytes when switching an image response to "Raw" (`pretty == false`),
/// so the hex view can reflow between 16 and 32 bytes per row based on pane width.
fn response_hex_for_view(pretty: bool, preview: &str, response_hex: Vec<u8>) -> Vec<u8> {
    if pretty && !preview.is_empty() {
        Vec::new()
    } else {
        response_hex
    }
}

fn hex_dump(body: &[u8]) -> String {
    let mut out = String::with_capacity(body.len() * 3);
    for (index, byte) in body.iter().enumerate() {
        if index > 0 && index % 16 == 0 {
            out.push('\n');
        }
        out.push_str(&format!("{byte:02x} "));
    }
    out
}

fn spawn_detached<T: 'static>(future: impl std::future::Future<Output = T> + 'static) {
    Owner::detached().run(|| {
        spawn(future);
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn default_save_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok();
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            home.as_deref()
                .map(|h| std::path::PathBuf::from(h).join(".config"))
        });
    let user_dirs =
        config_home.and_then(|dir| std::fs::read_to_string(dir.join("user-dirs.dirs")).ok());
    let cwd = std::env::current_dir().ok();
    default_save_dir_from(home.as_deref(), user_dirs.as_deref(), cwd.as_deref())
}

#[cfg(any(not(target_arch = "wasm32"), test))]
fn default_save_dir_from(
    home: Option<&str>,
    user_dirs_text: Option<&str>,
    cwd: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    if let Some(home) = home {
        if let Some(text) = user_dirs_text
            && let Some(xdg) = xdg_download_dir(home, text)
            && xdg.is_dir()
        {
            return Some(xdg);
        }
        let downloads = std::path::PathBuf::from(home).join("Downloads");
        if downloads.is_dir() {
            return Some(downloads);
        }
        let home_dir = std::path::PathBuf::from(home);
        if home_dir.is_dir() {
            return Some(home_dir);
        }
    }
    cwd.filter(|dir| dir.is_dir())
        .map(std::path::Path::to_path_buf)
}

#[cfg(not(target_arch = "wasm32"))]
fn save_dialog_with_default_dir() -> nsh_core::file_dialog::SaveDialog {
    let dialog = nsh_core::file_dialog::SaveDialog::new();
    match default_save_dir() {
        Some(dir) => dialog.directory(dir),
        None => dialog,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn open_dialog_with_default_dir() -> nsh_core::file_dialog::OpenDialog {
    let dialog = nsh_core::file_dialog::OpenDialog::files();
    match default_save_dir() {
        Some(dir) => dialog.directory(dir),
        None => dialog,
    }
}

#[cfg(any(not(target_arch = "wasm32"), test))]
fn write_export_file(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)
}

#[cfg(not(target_arch = "wasm32"))]
fn export_response(controls: Arc<Controls>, id: Option<u64>) {
    let Some(flow) = selected_flow(&controls, id) else {
        return;
    };
    if flow.response_body.is_empty() {
        set_status(&controls, StatusMsg::EmptyResponse);
        return;
    };
    let ext = present::content_type(&flow.response_headers)
        .and_then(present::image_extension)
        .unwrap_or_else(|| body_extension(&flow.response_headers, &flow.response_body));
    let dialog = save_dialog_with_default_dir()
        .file_name(format!("{}.{}", flow.id, ext))
        .filter(nsh_core::file_dialog::FileFilter::new(
            ext.to_ascii_uppercase(),
            [ext],
        ));
    let body = flow.response_body;
    spawn_detached(async move {
        match pick_save_path(dialog).await {
            Ok(Some(path)) => {
                if let Err(err) = write_export_file(&path, &body) {
                    set_status(&controls, StatusMsg::SaveFailed(err.to_string()));
                    return;
                }
                let text = path.display().to_string();
                set_clipboard(text.clone());
                set_status(&controls, StatusMsg::Saved(text));
            }
            Ok(None) => set_status(&controls, StatusMsg::SaveCancelled),
            Err(err) => set_status(&controls, StatusMsg::SaveFailed(err)),
        }
    });
}

/// `XDG_DOWNLOAD_DIR="$HOME/Downloads"` from `user-dirs.dirs`.
#[cfg(any(not(target_arch = "wasm32"), test))]
fn xdg_download_dir(home: &str, text: &str) -> Option<std::path::PathBuf> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "XDG_DOWNLOAD_DIR" {
            continue;
        }
        let mut value = value.trim();
        if value.len() >= 2 {
            let bytes = value.as_bytes();
            let quoted = (bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
                || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'');
            if quoted {
                value = &value[1..value.len() - 1];
            }
        }
        if value.is_empty() {
            return None;
        }
        return Some(std::path::PathBuf::from(value.replace("$HOME", home)));
    }
    None
}

#[cfg(any(not(target_arch = "wasm32"), test))]
fn body_extension(headers: &[(String, String)], body: &[u8]) -> &'static str {
    if let Some(kind) = present::content_type(headers) {
        let kind = kind.to_ascii_lowercase();
        if kind.contains("json") {
            return "json";
        }
        if kind.contains("html") {
            return "html";
        }
        if kind.contains("xml") {
            return "xml";
        }
        if kind.contains("text/") || kind.contains("javascript") {
            return "txt";
        }
    }
    if std::str::from_utf8(body).is_ok() {
        "txt"
    } else {
        "bin"
    }
}

fn selected_flow(controls: &Controls, id: Option<u64>) -> Option<crate::session::Flow> {
    let id = id?;
    let flow = controls.hub.with_session(|session| session.hydrated(id));
    if flow.is_none() {
        set_status(controls, StatusMsg::FlowGone);
    }
    flow
}

fn image_preview_path(id: u64, flow: &crate::session::Flow) -> String {
    if flow.finished_at.is_none() || flow.response_body.is_empty() || flow.response_truncated {
        return String::new();
    }
    let Some(ext) =
        present::content_type(&flow.response_headers).and_then(present::image_extension)
    else {
        return String::new();
    };
    if ext == "svg" {
        return String::new();
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = id;
        if ext == "ico" {
            if let Some(png) = present::ico_to_png(&flow.response_body) {
                return format!("data:image/png;base64,{}", session::base64_encode(&png));
            }
            let Some(sniffed) = present::sniff_raster_extension(&flow.response_body) else {
                return String::new();
            };
            let mime = match sniffed {
                "png" => "image/png",
                "jpg" => "image/jpeg",
                "gif" => "image/gif",
                "webp" => "image/webp",
                _ => return String::new(),
            };
            return format!(
                "data:{mime};base64,{}",
                session::base64_encode(&flow.response_body)
            );
        }
        let mime = match ext {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "bmp" => "image/bmp",
            _ => "application/octet-stream",
        };
        format!(
            "data:{mime};base64,{}",
            session::base64_encode(&flow.response_body)
        )
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let crc = present::crc32(&flow.response_body);
        let dir = ca::data_dir().join("preview");
        if ext == "ico" {
            let png_path = dir.join(format!("{id}-{crc:08x}.png"));
            if let Ok(meta) = std::fs::metadata(&png_path)
                && meta.len() > 0
            {
                return png_path.to_string_lossy().into_owned();
            }
            if let Some(png) = present::ico_to_png(&flow.response_body) {
                if std::fs::create_dir_all(&dir).is_err()
                    || std::fs::write(&png_path, &png).is_err()
                {
                    return String::new();
                }
                return png_path.to_string_lossy().into_owned();
            }
            let Some(sniffed) = present::sniff_raster_extension(&flow.response_body) else {
                return String::new();
            };
            let path = dir.join(format!("{id}-{crc:08x}.{sniffed}"));
            if let Ok(meta) = std::fs::metadata(&path)
                && meta.len() == flow.response_body.len() as u64
            {
                return path.to_string_lossy().into_owned();
            }
            if std::fs::create_dir_all(&dir).is_err()
                || std::fs::write(&path, &flow.response_body).is_err()
            {
                return String::new();
            }
            return path.to_string_lossy().into_owned();
        }
        let path = dir.join(format!("{id}-{crc:08x}.{ext}"));
        if let Ok(meta) = std::fs::metadata(&path)
            && meta.len() == flow.response_body.len() as u64
        {
            return path.to_string_lossy().into_owned();
        }
        if std::fs::create_dir_all(&dir).is_err()
            || std::fs::write(&path, &flow.response_body).is_err()
        {
            return String::new();
        }
        path.to_string_lossy().into_owned()
    }
}

fn lock_bool(slot: &Mutex<bool>) -> bool {
    *slot.lock().unwrap_or_else(|err| err.into_inner())
}

fn set_bool(slot: &Mutex<bool>, value: bool) {
    *slot.lock().unwrap_or_else(|err| err.into_inner()) = value;
}

fn lock_string(slot: &Mutex<String>) -> String {
    slot.lock().unwrap_or_else(|err| err.into_inner()).clone()
}

fn break_bar(
    paused: Signal<Option<(u64, BreakSide)>>,
    paused_count: Signal<usize>,
    paused_queue: Signal<Vec<(u64, BreakSide)>>,
    compose: Compose,
    selected: Signal<Option<u64>>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let release = controls.clone();
    let release_all = controls.clone();
    let chip_controls = controls.clone();
    let drop_controls = controls;
    view()
        .min_h(px(40.0))
        .pl(px(LIST_INSET))
        .pr_3()
        .py_1p5()
        .gap_1p5()
        .bg(status_warn_bg())
        .border_b_1()
        .border_b_color(status_warn_fg())
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(
                    text({
                        let langs = ui_langs();
                        move || -> SharedString {
                            let count = paused_count.get();
                            let queue_suffix = if count > 1 {
                                format!(" (1 / {count})")
                            } else {
                                String::new()
                            };
                            match paused.get() {
                                Some((id, BreakSide::Response)) => format!(
                                    "{} #{id} {}{queue_suffix}",
                                    t(ui_lang(langs), "流量", "Flow"),
                                    t(ui_lang(langs), "停在响应断点", "is paused on the response")
                                )
                                .into(),
                                Some((id, BreakSide::Request)) => format!(
                                    "{} #{id} {}{queue_suffix}",
                                    t(ui_lang(langs), "流量", "Flow"),
                                    t(ui_lang(langs), "停在请求断点", "is paused on the request")
                                )
                                .into(),
                                None => "".into(),
                            }
                        }
                    })
                    .flex_1()
                    .min_w(px(160.0))
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail)
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(status_warn_fg()),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .child(
                            view()
                                .h(px(26.0))
                                .px_2p5()
                                .rounded(px(6.0))
                                .bg(accent_blue())
                                .hover(|style| style.bg(accent_blue_hover()))
                                .active(|style| style.opacity(0.85))
                                .cursor(Cursor::Pointer)
                                .flex_row()
                                .items_center()
                                .on_press(move || {
                                    continue_break(&release, compose, selected, paused);
                                })
                                .child(
                                    text(tr("放行", "Continue"))
                                        .number_of_lines(1)
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(white()),
                                ),
                        )
                        .child(when(
                            move || paused_count.get() > 1,
                            move || {
                                let controls = release_all.clone();
                                detail_button(tr("全部放行", "Continue all"), move || {
                                    if !break_all_allowed() {
                                        return;
                                    }
                                    if compose.composing.get() {
                                        reset_compose(compose);
                                    }
                                    controls.hub.resolve_all_breaks();
                                })
                            },
                        ))
                        .child(
                            view()
                                .h(px(26.0))
                                .px_2p5()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(status_err_fg())
                                .bg(status_err_bg())
                                .hover(|style| style.bg(status_err_bg_hover()))
                                .active(|style| style.opacity(0.8))
                                .cursor(Cursor::Pointer)
                                .flex_row()
                                .items_center()
                                .on_press(move || {
                                    let paused_now =
                                        drop_controls.hub.paused().or_else(|| paused.get());
                                    if !break_action_allowed(paused_now.map(|(id, _)| id)) {
                                        return;
                                    }
                                    if compose.composing.get() {
                                        reset_compose(compose);
                                    }
                                    let _ = drop_controls.hub.resolve_break(BreakDecision {
                                        drop: true,
                                        headers: None,
                                        body: None,
                                        status: None,
                                    });
                                })
                                .child(
                                    text(tr("丢弃", "Drop"))
                                        .number_of_lines(1)
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(status_err_fg()),
                                ),
                        ),
                ),
        )
        .child(when(
            move || paused_queue.with(|queue| queue.len() > 1),
            move || {
                let controls = chip_controls.clone();
                view().flex_row().flex_wrap().gap_1().child(each(
                    move || paused_queue.get(),
                    |item| item.0,
                    move |item, index| {
                        let controls = controls.clone();
                        let is_head = move || index.get() == 0;
                        view()
                            .h(px(22.0))
                            .px_2()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(move || {
                                if is_head() {
                                    accent_blue_border()
                                } else {
                                    border_control()
                                }
                            })
                            .bg(move || {
                                if is_head() {
                                    accent_blue_soft()
                                } else {
                                    bg_control()
                                }
                            })
                            .hover(|style| {
                                style.bg(move || {
                                    if is_head() {
                                        accent_blue_soft_hover()
                                    } else {
                                        bg_control_hover()
                                    }
                                })
                            })
                            .cursor(Cursor::Pointer)
                            .active(|style| style.opacity(0.75))
                            .flex_row()
                            .items_center()
                            .on_press(move || {
                                let id = item.with(|pair| pair.0);
                                focus_queued_break(&controls, id, selected, compose);
                            })
                            .child(
                                text({
                                    let langs = ui_langs();
                                    move || -> SharedString {
                                        let (id, side) = item.with(|pair| *pair);
                                        let which = match side {
                                            BreakSide::Request => {
                                                t(ui_lang(langs), "请求", "Request")
                                            }
                                            BreakSide::Response => {
                                                t(ui_lang(langs), "响应", "Response")
                                            }
                                        };
                                        format!("#{id} {which}").into()
                                    }
                                })
                                .tabular_nums()
                                .number_of_lines(1)
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || {
                                    if is_head() {
                                        accent_blue_text()
                                    } else {
                                        fg_secondary()
                                    }
                                }),
                            )
                    },
                ))
            },
        ))
}

fn focus_queued_break(
    controls: &Controls,
    id: u64,
    selected: Signal<Option<u64>>,
    compose: Compose,
) {
    let already =
        selected.get() == Some(id) && controls.hub.paused().is_some_and(|(cur, _)| cur == id);
    if !controls.hub.promote_break(id) {
        return;
    }
    if !already && compose.composing.get() {
        reset_compose(compose);
    }
    selected.set(Some(id));
}

#[cfg(not(target_arch = "wasm32"))]
static LAST_BREAK_RESOLVE: Mutex<Option<(u64, Instant)>> = Mutex::new(None);
#[cfg(not(target_arch = "wasm32"))]
static LAST_BREAK_ALL: Mutex<Option<Instant>> = Mutex::new(None);
#[cfg(target_arch = "wasm32")]
static LAST_BREAK_RESOLVE: Mutex<Option<(u64, SystemTime)>> = Mutex::new(None);
#[cfg(target_arch = "wasm32")]
static LAST_BREAK_ALL: Mutex<Option<SystemTime>> = Mutex::new(None);

fn break_action_allowed(flow_id: Option<u64>) -> bool {
    // `Instant::now` panics on wasm32. The debounce there uses the wall clock.
    #[cfg(not(target_arch = "wasm32"))]
    let now = Instant::now();
    #[cfg(target_arch = "wasm32")]
    let now = clock::now();
    let mut lock = LAST_BREAK_RESOLVE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(id) = flow_id {
        if let Some((last_id, last_time)) = *lock
            && last_id == id
            && debounce_elapsed(now, last_time) < Duration::from_millis(300)
        {
            return false;
        }
        *lock = Some((id, now));
    }
    true
}

fn break_all_allowed() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    let now = Instant::now();
    #[cfg(target_arch = "wasm32")]
    let now = clock::now();
    let mut lock = LAST_BREAK_ALL.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(last_time) = *lock
        && debounce_elapsed(now, last_time) < Duration::from_millis(300)
    {
        return false;
    }
    *lock = Some(now);
    true
}

#[cfg(not(target_arch = "wasm32"))]
fn debounce_elapsed(now: Instant, earlier: Instant) -> Duration {
    now.duration_since(earlier)
}

#[cfg(target_arch = "wasm32")]
fn debounce_elapsed(now: SystemTime, earlier: SystemTime) -> Duration {
    now.duration_since(earlier).unwrap_or_default()
}

fn continue_break(
    controls: &Controls,
    compose: Compose,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
) {
    let current_paused = controls.hub.paused().or_else(|| paused.get());
    if !break_action_allowed(current_paused.map(|(id, _)| id)) {
        return;
    }
    let edited = compose.composing.get()
        && current_paused.is_some_and(|(id, side)| {
            (selected.get() == Some(id) || selected.get().is_none())
                && compose.response.get() == matches!(side, BreakSide::Response)
        });
    let decision = if edited {
        BreakDecision {
            drop: false,
            headers: Some(parse_header_text(&compose.headers.get())),
            body: Some(compose.body.get().into_bytes()),
            status: edited_status(compose),
        }
    } else {
        BreakDecision {
            drop: false,
            headers: None,
            body: None,
            status: None,
        }
    };
    if compose.composing.get() {
        reset_compose(compose);
    }
    if !controls.hub.resolve_break(decision) {
        set_status(controls, StatusMsg::BreakDone);
    }
}

fn submit_compose(
    controls: &Arc<Controls>,
    compose: Compose,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
) {
    if compose.response.get() {
        let raw_status = compose.status.get();
        if !raw_status.trim().is_empty() && parse_status_code(&raw_status).is_none() {
            return;
        }
        let paused_sig = signal(controls.hub.paused());
        continue_break(controls, compose, selected, paused_sig);
        return;
    }
    let at_req_break = paused
        .get()
        .is_some_and(|(id, side)| selected.get() == Some(id) && side == BreakSide::Request)
        || controls
            .hub
            .paused()
            .is_some_and(|(id, side)| selected.get() == Some(id) && side == BreakSide::Request);
    if at_req_break {
        continue_break(controls, compose, selected, paused);
        return;
    }
    replay_flow(
        controls,
        selected.get(),
        ReplayOverride {
            method: Some(compose.method.get()),
            url: Some(compose.url.get()),
            headers: Some(parse_header_text(&compose.headers.get())),
            body: Some(compose.body.get().into_bytes()),
        },
    );
}

fn compose_box(
    compose: Compose,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    paused: Signal<Option<(u64, BreakSide)>>,
) -> impl IntoElement {
    let controls_continue = controls.clone();
    let controls_touch_status = controls.clone();
    let controls_touch_method = controls.clone();
    let controls_touch_url = controls.clone();
    let controls_submit_url = controls.clone();
    let controls_touch_headers = controls.clone();
    let controls_touch_body = controls.clone();
    let controls_submit_status = controls.clone();
    let status_invalid = move || {
        compose.response.get()
            && !compose.status.get().trim().is_empty()
            && parse_status_code(&compose.status.get()).is_none()
    };
    let is_req_break = move || {
        !compose.response.get()
            && paused
                .get()
                .is_some_and(|(id, side)| selected.get() == Some(id) && side == BreakSide::Request)
    };
    view()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .overflow_hidden()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .px_3p5()
                .py_2()
                .border_b_1()
                .border_b_color(border_subtle())
                .child(section_title({
                    let langs = ui_langs();
                    move || -> SharedString {
                        if compose.response.get() {
                            t(ui_lang(langs), "编辑响应", "Edit response").into()
                        } else if is_req_break() {
                            t(ui_lang(langs), "编辑请求", "Edit request").into()
                        } else if selected.get().is_none() {
                            t(ui_lang(langs), "构造请求", "New request").into()
                        } else {
                            t(ui_lang(langs), "编辑重发", "Edit and resend").into()
                        }
                    }
                }))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .child(
                            view()
                                .h(px(26.0))
                                .px_2p5()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(border_control())
                                .bg(bg_control())
                                .hover(|style| style.bg(bg_control_hover()))
                                .active(|style| style.opacity(0.75))
                                .cursor(Cursor::Pointer)
                                .flex_row()
                                .items_center()
                                .on_press(move || reset_compose(compose))
                                .child(
                                    text(tr("取消", "Cancel"))
                                        .number_of_lines(1)
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(fg_secondary()),
                                ),
                        )
                        .child(when(
                            move || compose.response.get(),
                            move || {
                                let controls = controls_continue.clone();
                                view()
                                    .h(px(26.0))
                                    .px_2p5()
                                    .rounded(px(6.0))
                                    .bg(move || {
                                        if status_invalid() {
                                            bg_control()
                                        } else {
                                            accent_blue()
                                        }
                                    })
                                    .hover(|style| {
                                        style.bg(move || {
                                            if status_invalid() {
                                                bg_control()
                                            } else {
                                                accent_blue_hover()
                                            }
                                        })
                                    })
                                    .active(|style| {
                                        style.opacity(move || {
                                            if status_invalid() { 1.0_f32 } else { 0.85_f32 }
                                        })
                                    })
                                    .cursor(Cursor::Pointer)
                                    .disabled(status_invalid)
                                    .disabled_style(|style| style.cursor(Cursor::Default))
                                    .flex_row()
                                    .items_center()
                                    .tooltip(label_tip("Cmd/Ctrl+Enter 放行", "Cmd/Ctrl+Enter to continue"))
                                    .on_press(move || {
                                        submit_compose(&controls, compose, selected, paused);
                                    })
                                    .child(
                                        text(tr("放行", "Continue"))
                                            .number_of_lines(1)
                                            .text_size(px(12.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || {
                                                if status_invalid() {
                                                    fg_muted()
                                                } else {
                                                    AdaptiveColor::rgb(0xffffff, 0xffffff)
                                                }
                                            }),
                                    )
                            },
                        ))
                        .child(when(
                            move || !compose.response.get(),
                            move || {
                                let controls = controls.clone();
                                let langs = ui_langs();
                                view()
                                    .h(px(26.0))
                                    .px_2p5()
                                    .rounded(px(6.0))
                                    .bg(accent_blue())
                                    .hover(|style| style.bg(accent_blue_hover()))
                                    .active(|style| style.opacity(0.85))
                                    .cursor(Cursor::Pointer)
                                    .flex_row()
                                    .items_center()
                                    .tooltip(label_tip("Cmd/Ctrl+Enter 发送", "Cmd/Ctrl+Enter to send"))
                                    .on_press(move || {
                                        submit_compose(&controls, compose, selected, paused);
                                    })
                                    .child(
                                        text(move || -> SharedString {
                                            if is_req_break() {
                                                t(ui_lang(langs), "放行", "Continue").into()
                                            } else {
                                                t(ui_lang(langs), "发送", "Send").into()
                                            }
                                        })
                                        .number_of_lines(1)
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(white()),
                                    )
                            },
                        )),
                ),
        )
        .child(
            view()
                .p_3p5()
                .gap_2()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .child(when(
                            move || compose.response.get(),
                            move || {
                                let controls_touch = controls_touch_status.clone();
                                let controls_submit = controls_submit_status.clone();
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        text(tr("状态码", "Status code"))
                                            .number_of_lines(1)
                                            .text_size(px(12.0))
                                            .text_color(fg_secondary()),
                                    )
                                    .child(
                                        view().w(px(88.0)).flex_shrink_0().child(
                                            compose_input_on(
                                                compose.status,
                                                "200",
                                                30.0,
                                                false,
                                                move |text| {
                                                    controls_touch.hub.touch_break();
                                                    compose.status.set(text.to_string());
                                                },
                                            )
                                            .border_color(move || {
                                                if status_invalid() {
                                                    status_err_fg()
                                                } else {
                                                    border_control()
                                                }
                                            })
                                            .focus(move |style| {
                                                style.border_color(move || {
                                                    if status_invalid() {
                                                        status_err_fg()
                                                    } else {
                                                        accent_blue()
                                                    }
                                                })
                                            })
                                            .on_submit(move |text: &str| {
                                                compose.status.set(text.to_string());
                                                if status_invalid() {
                                                    return;
                                                }
                                                let paused_sig =
                                                    signal(controls_submit.hub.paused());
                                                continue_break(
                                                    &controls_submit,
                                                    compose,
                                                    selected,
                                                    paused_sig,
                                                );
                                            }),
                                        ),
                                    )
                            },
                        ))
                        .child(when(
                            move || !compose.response.get(),
                            move || {
                                let controls_method = controls_touch_method.clone();
                                let controls_method_pick = controls_touch_method.clone();
                                let controls_url = controls_touch_url.clone();
                                let controls_url_submit = controls_submit_url.clone();
                                when(
                                    is_req_break,
                                    move || {
                                        view()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .h(px(30.0))
                                            .flex_row()
                                            .items_center()
                                            .gap_2()
                                            .px_2p5()
                                            .rounded(px(7.0))
                                            .border_1()
                                            .border_color(border_subtle())
                                            .bg(bg_code())
                                            .child(
                                                view()
                                                    .px_2()
                                                    .py_0p5()
                                                    .flex_shrink_0()
                                                    .rounded(px(5.0))
                                                    .bg(move || method_bg(&compose.method.get()))
                                                    .child(
                                                        mono(move || compose.method.get())
                                                            .text_size(px(11.5))
                                                            .font_weight(FontWeight::BOLD)
                                                            .text_color(move || {
                                                                method_fg(&compose.method.get())
                                                            }),
                                                    ),
                                            )
                                            .child(
                                                mono(move || compose.url.get())
                                                    .flex_1()
                                                    .min_w(px(0.0))
                                                    .number_of_lines(1)
                                                    .text_overflow(EllipsizeMode::Middle)
                                                    .text_size(px(12.0))
                                                    .text_color(fg_secondary()),
                                            )
                                    },
                                )
                                .otherwise(move || {
                                    let controls_method = controls_method.clone();
                                    let controls_method_pick = controls_method_pick.clone();
                                    let controls_url = controls_url.clone();
                                    let controls_url_submit = controls_url_submit.clone();
                                    let method_open = signal(false);
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .flex_row()
                                        .gap_1p5()
                                        .child(
                                            view()
                                                .flex_row()
                                                .items_center()
                                                .gap_1()
                                                .flex_shrink_0()
                                                .child(view().w(px(76.0)).child(compose_input_on(
                                                    compose.method,
                                                    "POST",
                                                    30.0,
                                                    false,
                                                    move |text| {
                                                        controls_method.hub.touch_break();
                                                        compose.method.set(text.to_string());
                                                    },
                                                )))
                                                .child(
                                                    menu()
                                                        .on_open_change(
                                                            move |event: &nsh_base::MenuEvent| {
                                                                if let nsh_base::MenuEvent::OpenChange(
                                                                    next,
                                                                    _,
                                                                ) = event
                                                                {
                                                                    method_open.set(*next);
                                                                }
                                                            },
                                                        )
                                                        .placement(Placement::Bottom)
                                                        .align(Align::Start)
                                                        .offset(px(4.0))
                                                        .margin(px(8.0))
                                                        .trigger(
                                                            view()
                                                                .w(px(26.0))
                                                                .h(px(30.0))
                                                                .rounded(px(7.0))
                                                                .border_1()
                                                                .border_color(move || {
                                                                    if method_open.get() {
                                                                        accent_blue_border()
                                                                    } else {
                                                                        border_control()
                                                                    }
                                                                })
                                                                .bg(move || {
                                                                    if method_open.get() {
                                                                        accent_blue_soft()
                                                                    } else {
                                                                        bg_control()
                                                                    }
                                                                })
                                                                .hover(|style| {
                                                                    style.bg(move || {
                                                                        if method_open.get() {
                                                                            accent_blue_soft_hover()
                                                                        } else {
                                                                            bg_control_hover()
                                                                        }
                                                                    })
                                                                })
                                                                .active(|style| style.opacity(0.8))
                                                                .cursor(Cursor::Pointer)
                                                                .items_center()
                                                                .justify_center()
                                                                .label(tr(
                                                                    "选择 HTTP 方法",
                                                                    "Pick HTTP method",
                                                                ))
                                                                .tooltip(label_tip(
                                                                    "选择常用 HTTP 方法",
                                                                    "Pick HTTP method",
                                                                ))
                                                                .child(glyph(
                                                                    || Glyph::Chevron,
                                                                    move || {
                                                                        color(if method_open.get() {
                                                                            accent_blue_text()
                                                                        } else {
                                                                            fg_secondary()
                                                                        })
                                                                    },
                                                                )),
                                                        )
                                                        .child(
                                                            view()
                                                                .w(px(116.0))
                                                                .max_w(move || {
                                                                    px((window().width.0 - 16.0)
                                                                        .max(96.0))
                                                                })
                                                                .max_h(move || {
                                                                    px((window().height.0 - 24.0)
                                                                        .max(120.0))
                                                                })
                                                                .rounded(px(9.0))
                                                                .border_1()
                                                                .border_color(border_strong())
                                                                .bg(bg_card())
                                                                .shadow_md()
                                                                .overflow_hidden()
                                                                .child(
                                                                    scroll()
                                                                        .w_full()
                                                                        .max_h(move || {
                                                                            px((window().height.0
                                                                                - 26.0)
                                                                                .max(118.0))
                                                                        })
                                                                        .child(
                                                                            view()
                                                                                .w_full()
                                                                                .flex_col()
                                                                                .p_1()
                                                                                .gap_0p5()
                                                                                .children(
                                                                                    [
                                                                                        "GET",
                                                                                        "POST",
                                                                                        "PUT",
                                                                                        "PATCH",
                                                                                        "DELETE",
                                                                                        "HEAD",
                                                                                        "OPTIONS",
                                                                                    ]
                                                                                    .into_iter()
                                                                                    .map(move |m| {
                                                                                        let controls_pick =
                                                                                            controls_method_pick
                                                                                                .clone();
                                                                                        marked_action(
                                                                                            move || {
                                                                                                compose
                                                                                                    .method
                                                                                                    .get()
                                                                                                    .trim()
                                                                                                    .eq_ignore_ascii_case(
                                                                                                        m,
                                                                                                    )
                                                                                            },
                                                                                            m,
                                                                                            move || {
                                                                                                controls_pick
                                                                                                    .hub
                                                                                                    .touch_break();
                                                                                                compose
                                                                                                    .method
                                                                                                    .set(m.to_string());
                                                                                            },
                                                                                        )
                                                                                    }),
                                                                                ),
                                                                        ),
                                                                ),
                                                        ),
                                                ),
                                        )
                                        .child(view().flex_1().min_w(px(0.0)).child(
                                            compose_input_on(
                                                compose.url,
                                                tr(
                                                    "https://example.com/path，或粘贴 curl",
                                                    "https://example.com/path, or paste curl",
                                                ),
                                                30.0,
                                                false,
                                                move |text| {
                                                    controls_url.hub.touch_break();
                                                    apply_url_edit(compose, text);
                                                },
                                            )
                                            .on_submit(move |text: &str| {
                                                controls_url_submit.hub.touch_break();
                                                if looks_like_curl(text) {
                                                    apply_url_edit(compose, text);
                                                    return;
                                                }
                                                apply_url_edit(compose, text);
                                                submit_compose(
                                                    &controls_url_submit,
                                                    compose,
                                                    selected,
                                                    paused,
                                                );
                                            }),
                                        ))
                                })
                            },
                        )),
                )
                .child(compose_editor(
                    compose.headers,
                    tr("一行一个 Name: value", "One Name: value per line"),
                    120.0,
                    "plaintext",
                    controls_touch_headers,
                ))
                .child(compose_body_editor(
                    compose.body,
                    compose.headers,
                    tr("body", "body"),
                    160.0,
                    controls_touch_body,
                )),
        )
}

fn compose_body_editor(
    value: Signal<String>,
    headers: Signal<String>,
    placeholder: impl Fn() -> String + 'static,
    height: f32,
    controls: Arc<Controls>,
) -> impl IntoElement {
    view()
        .w_full()
        .h(px(height))
        .rounded(px(7.0))
        .border_1()
        .border_color(border_control())
        .bg(bg_code())
        .overflow_hidden()
        .child(
            editor()
                .value(move || SharedString::from(value.get()))
                .language(move || {
                    let parsed = parse_header_text(&headers.get());
                    let ct = present::content_type(&parsed);
                    SharedString::from(
                        present::editor_language(ct, &value.get()).unwrap_or("plaintext"),
                    )
                })
                .placeholder(move || SharedString::from(placeholder()))
                .line_numbers(true)
                .folding(true)
                .soft_wrap(true)
                .font_size(Points(12.0))
                .line_height(Points(18.0))
                .scroll_beyond_last_line(false)
                .on_change(move |text: &SharedString| {
                    controls.hub.touch_break();
                    value.set(text.to_string());
                })
                .size_full(),
        )
}

fn compose_editor(
    value: Signal<String>,
    placeholder: impl Fn() -> String + 'static,
    height: f32,
    language: &'static str,
    controls: Arc<Controls>,
) -> impl IntoElement {
    view()
        .w_full()
        .h(px(height))
        .rounded(px(7.0))
        .border_1()
        .border_color(border_control())
        .bg(bg_code())
        .overflow_hidden()
        .child(
            editor()
                .value(move || SharedString::from(value.get()))
                .language(language)
                .placeholder(move || SharedString::from(placeholder()))
                .line_numbers(true)
                .folding(true)
                .soft_wrap(true)
                .font_size(Points(12.0))
                .line_height(Points(18.0))
                .scroll_beyond_last_line(false)
                .on_change(move |text: &SharedString| {
                    controls.hub.touch_break();
                    value.set(text.to_string());
                })
                .size_full(),
        )
}

fn compose_input_on(
    value: Signal<String>,
    placeholder: impl IntoReactive<String> + 'static,
    height: f32,
    multiline: bool,
    on_change: impl Fn(&str) + 'static,
) -> Input {
    input()
        .value(value)
        .placeholder(placeholder)
        .placeholder_color(move || color(fg_muted()))
        .w_full()
        .h(px(height))
        .px_2p5()
        .rounded(px(7.0))
        .bg(bg_code())
        .border_1()
        .border_color(border_control())
        .focus(|style| style.border_color(accent_blue()))
        .text_size(px(12.0))
        .text_color(fg_primary())
        .multiline(multiline)
        .on_change(on_change)
}

fn apply_url_edit(compose: Compose, text: &str) {
    if looks_like_curl(text)
        && let Some(parsed) = present::parse_curl(text)
    {
        compose.method.set(parsed.method);
        compose.url.set(parsed.url);
        compose.headers.set(header_text(&parsed.headers));
        compose.body.set(parsed.body);
        return;
    }
    compose.url.set(text.to_string());
}

fn looks_like_curl(text: &str) -> bool {
    let text = text.trim_start();
    text == "curl"
        || text.starts_with("curl ")
        || text.starts_with("curl\t")
        || text.starts_with("curl\n")
        || text.starts_with("curl\r")
}

fn parse_status_code(text: &str) -> Option<u16> {
    let code = text.split_whitespace().next()?.parse::<u16>().ok()?;
    (100..600).contains(&code).then_some(code)
}

fn edited_status(compose: Compose) -> Option<u16> {
    if !compose.response.get() {
        return None;
    }
    parse_status_code(&compose.status.get())
}

fn lan_toggle(on: Signal<bool>, controls: Arc<Controls>) -> impl IntoElement {
    view()
        .h(px(22.0))
        .px_2()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_1()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || {
            if on.get() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(bg_control())
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .on_press(move || {
            let next = !on.get();
            on.set(next);
            set_bool(&controls.listen_all, next);
            if let Err(err) = store_listen_all(next) {
                set_status(
                    &controls,
                    StatusMsg::PrefUnsaved {
                        name: PrefName::Lan,
                        err: err.to_string(),
                    },
                );
            }
        })
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .bg(move || if on.get() { status_ok_fg() } else { fg_muted() }),
        )
        .child(
            text({
                let langs = ui_langs();
                move || -> SharedString {
                    if on.get() {
                        t(ui_lang(langs), "局域网 开", "LAN on").into()
                    } else {
                        t(ui_lang(langs), "局域网 关", "LAN off").into()
                    }
                }
            })
            .text_size(px(11.5))
            .text_color(fg_secondary()),
        )
}

#[cfg(not(target_arch = "wasm32"))]
fn prompt_import_har(controls: Arc<Controls>) {
    spawn_detached(async move {
        let dialog = open_dialog_with_default_dir()
            .multiple(true)
            .filter(nsh_core::file_dialog::FileFilter::new("HAR", ["har"]));
        match pick_paths(dialog).await {
            Ok(Some(paths)) => {
                for path in paths {
                    import_har_file(&controls, &path.to_string_lossy());
                }
            }
            Ok(None) => {}
            Err(err) => set_status(&controls, StatusMsg::ImportFailed(err)),
        }
    });
}

fn begin_new_request(compose: Compose, selected: Signal<Option<u64>>) {
    selected.set(None);
    compose.response.set(false);
    compose.method.set("GET".to_string());
    compose.url.set(String::new());
    compose.headers.set(String::new());
    compose.body.set(String::new());
    compose.status.set(String::new());
    compose.composing.set(true);
}

fn begin_sequence_diagram(
    controls: &Controls,
    rows: Signal<Vec<Arc<RowLine>>>,
    seq_builder: SeqBuilder,
    compose: Compose,
    selected: Signal<Option<u64>>,
    shown_phase: Signal<Phase>,
) {
    let current_rows = rows.get();
    if current_rows.is_empty() {
        return;
    }
    let mut candidates: Vec<SeqCandidate> = controls.hub.with_session(|session| {
        current_rows
            .into_iter()
            .map(|row| {
                if let Some(f) = session.get(row.id) {
                    let failed = f.error.is_some() && f.status.is_none();
                    SeqCandidate {
                        id: row.id,
                        method: f.method.clone(),
                        host: present::resolve_flow_host(f),
                        path: present::resolve_flow_target(f, true),
                        status: f.status,
                        failed,
                        elapsed: present::format_elapsed(f.started, f.finished_at),
                        comment: f.comment.clone(),
                        checked: true,
                    }
                } else {
                    SeqCandidate {
                        id: row.id,
                        method: row.method.clone(),
                        host: row.host.clone(),
                        path: row.path.clone(),
                        status: row.status,
                        failed: row.failed,
                        elapsed: row.elapsed.clone(),
                        comment: row.comment.clone(),
                        checked: true,
                    }
                }
            })
            .collect()
    });
    candidates.sort_by_key(|c| c.id);
    reset_compose(compose);
    selected.set(None);
    shown_phase.set(Phase::Empty);
    seq_builder.redact_open.set(false);
    seq_builder.items.set(candidates);
    seq_builder.hover_bar.set(None);
    refresh_seq_diagram(controls, seq_builder);
    seq_builder.open.set(true);
}

fn begin_redact_preview(
    _controls: &Controls,
    rows: Signal<Vec<Arc<RowLine>>>,
    seq_builder: SeqBuilder,
    compose: Compose,
    selected: Signal<Option<u64>>,
    shown_phase: Signal<Phase>,
) {
    let pick = selected
        .get()
        .or_else(|| rows.with(|list| list.last().map(|r| r.id)));
    seq_builder.redact_flow_id.set(pick);
    reset_compose(compose);
    selected.set(None);
    shown_phase.set(Phase::Empty);
    seq_builder.open.set(false);
    seq_builder.redact_open.set(true);
}

fn redact_preview_panel(
    seq_builder: SeqBuilder,
    controls: Arc<Controls>,
    rows: Signal<Vec<Arc<RowLine>>>,
    filter: Signal<String>,
) -> impl IntoElement {
    let toggle_controls = controls.clone();
    let strict_controls = controls.clone();
    let fields_controls = controls.clone();
    let export_har_controls = controls.clone();
    let export_html_controls = controls.clone();
    let preview_controls = controls;
    let langs = ui_langs();

    let preview_pair = move || {
        let zh = ui_lang(langs) == Lang::Zh;
        let flow_id = seq_builder
            .redact_flow_id
            .get()
            .or_else(|| rows.with(|list| list.last().map(|r| r.id)));
        let extra = seq_builder.redact_fields.get();
        let opts =
            share::RedactOptions::new(true, &extra).with_strict(seq_builder.redact_strict.get());
        let Some(id) = flow_id else {
            return (
                None,
                String::new(),
                String::new(),
                share::RedactReport::default(),
            );
        };
        let fallback_id = rows.with(|list| list.last().map(|r| r.id));
        let Some(flow) = preview_controls.hub.with_session(|session| {
            session.hydrated(id).or_else(|| {
                fallback_id
                    .filter(|&last_id| last_id != id)
                    .and_then(|last_id| session.hydrated(last_id))
            })
        }) else {
            return (
                None,
                String::new(),
                String::new(),
                share::RedactReport::default(),
            );
        };
        let (before, after, report) = share::format_redact_preview(&flow, &opts, zh);
        (
            Some((flow.id, flow.method, flow.url)),
            before,
            after,
            report,
        )
    };

    view()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .overflow_hidden()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .px_3p5()
                .py_2()
                .border_b_1()
                .border_b_color(border_subtle())
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(section_title(tr(
                            "导出与分享脱敏设置",
                            "Export & share redaction",
                        )))
                        .child(
                            view()
                                .h(px(24.0))
                                .px_2p5()
                                .rounded(px(5.0))
                                .border_1()
                                .border_color(move || {
                                    if seq_builder.redact_export.get() {
                                        status_ok_fg()
                                    } else {
                                        border_control()
                                    }
                                })
                                .bg(move || {
                                    if seq_builder.redact_export.get() {
                                        status_ok_bg()
                                    } else {
                                        bg_control()
                                    }
                                })
                                .hover(|style| {
                                    style.bg(move || {
                                        if seq_builder.redact_export.get() {
                                            status_ok_bg_hover()
                                        } else {
                                            bg_control_hover()
                                        }
                                    })
                                })
                                .active(|style| style.opacity(0.8))
                                .flex_row()
                                .items_center()
                                .cursor(Cursor::Pointer)
                                .on_press(move || {
                                    let next = !seq_builder.redact_export.get();
                                    seq_builder.redact_export.set(next);
                                    *toggle_controls
                                        .redact_export
                                        .lock()
                                        .unwrap_or_else(|err| err.into_inner()) = next;
                                    let _ = save_config(|cfg| cfg.redact_export = next);
                                })
                                .child(
                                    text(move || -> SharedString {
                                        let lang = ui_lang(langs);
                                        if seq_builder.redact_export.get() {
                                            t(lang, "脱敏：已开启", "Redaction: ON").into()
                                        } else {
                                            t(
                                                lang,
                                                "脱敏：已关闭（点击开启）",
                                                "Redaction: OFF (click to enable)",
                                            )
                                            .into()
                                        }
                                    })
                                    .number_of_lines(1)
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || {
                                        if seq_builder.redact_export.get() {
                                            status_ok_fg()
                                        } else {
                                            fg_secondary()
                                        }
                                    }),
                                ),
                        )
                        .child(
                            view()
                                .h(px(24.0))
                                .px_2p5()
                                .rounded(px(5.0))
                                .border_1()
                                .border_color(move || {
                                    if seq_builder.redact_strict.get() {
                                        accent_blue_border()
                                    } else {
                                        border_control()
                                    }
                                })
                                .bg(move || {
                                    if seq_builder.redact_strict.get() {
                                        accent_blue_soft()
                                    } else {
                                        bg_control()
                                    }
                                })
                                .hover(|style| {
                                    style.bg(move || {
                                        if seq_builder.redact_strict.get() {
                                            accent_blue_soft_hover()
                                        } else {
                                            bg_control_hover()
                                        }
                                    })
                                })
                                .active(|style| style.opacity(0.8))
                                .flex_row()
                                .items_center()
                                .cursor(Cursor::Pointer)
                                .label(tr("严格遮罩", "Strict mask"))
                                .tooltip(label_tip(
                                    "开启后将无法按字段脱敏的非结构化或二进制正文替换为占位文本",
                                    "Replace unstructured or binary bodies with a placeholder when redacting",
                                ))
                                .on_press(move || {
                                    let next = !seq_builder.redact_strict.get();
                                    seq_builder.redact_strict.set(next);
                                    *strict_controls
                                        .redact_strict
                                        .lock()
                                        .unwrap_or_else(|err| err.into_inner()) = next;
                                    let _ = save_config(|cfg| cfg.redact_strict = next);
                                })
                                .child(
                                    text(tr("严格遮罩", "Strict mask"))
                                        .number_of_lines(1)
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || {
                                            if seq_builder.redact_strict.get() {
                                                accent_blue_text()
                                            } else {
                                                fg_secondary()
                                            }
                                        }),
                                ),
                        ),
                )
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_1p5()
                        .child(seq_action_button(
                            tr("导出 HAR", "Export HAR"),
                            || true,
                            move || {
                                export_har_file(export_har_controls.clone(), &filter.get());
                            },
                        ))
                        .child(seq_action_button(
                            tr("导出 HTML", "Export HTML"),
                            || true,
                            move || {
                                export_html_file(
                                    export_html_controls.clone(),
                                    &filter.get(),
                                    ui_lang(langs) == Lang::Zh,
                                );
                            },
                        ))
                        .child(seq_action_button(tr("关闭", "Close"), || true, move || {
                            seq_builder.redact_open.set(false);
                        })),
                ),
        )
        .child(
            view()
                .p_3p5()
                .gap_2p5()
                .child(
                    text(tr(
                        "默认脱敏：Authorization、Proxy-Authorization、Cookie、Set-Cookie、X-Api-Key，以及 URL 查询参数、JSON 和表单中的 password、token、access_token、secret、api_key、session_id、jwt 等字段。仅作用于导出的 HAR/HTML 与实时分享副本，不改动抓包记录或重放数据。",
                        "Defaults: Authorization, Proxy-Authorization, Cookie, Set-Cookie, X-Api-Key, plus password, token, access_token, secret, api_key, session_id, jwt in query/JSON/form. Sanitizes exported/shared copies only without modifying captured flows or replay data.",
                    ))
                    .text_size(px(11.5))
                    .text_color(fg_muted()),
                )
                .child(
                    view()
                        .gap_1()
                        .child(
                            text(tr(
                                "附加敏感字段或请求头名（逗号或空格分隔）",
                                "Extra sensitive field or header names (comma or space separated)",
                            ))
                            .text_size(px(11.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg_secondary()),
                        )
                        .child(
                            input()
                                .value(seq_builder.redact_fields)
                                .placeholder(tr(
                                    "例如: x-custom-token, phone, id_card",
                                    "e.g. x-custom-token, phone, card_number",
                                ))
                                .placeholder_color(move || color(fg_muted()))
                                .h(px(30.0))
                                .px_2p5()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(border_control())
                                .focus(|style| style.border_color(accent_blue()))
                                .bg(bg_code())
                                .text_size(px(12.0))
                                .text_color(fg_primary())
                                .on_change(move |val: &str| {
                                    let text_val = val.to_string();
                                    seq_builder.redact_fields.set(text_val.clone());
                                    *fields_controls
                                        .redact_fields
                                        .lock()
                                        .unwrap_or_else(|err| err.into_inner()) = text_val.clone();
                                    let _ = save_config(|cfg| cfg.redact_fields = text_val);
                                }),
                        ),
                )
                .child({
                    let pair_header = preview_pair.clone();
                    let pair_warn = preview_pair.clone();
                    let pair_warn_text = preview_pair.clone();
                    let pair_diff_old = preview_pair.clone();
                    let pair_diff_new = preview_pair;
                    view()
                        .mt_1()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(border_subtle())
                        .bg(bg_code())
                        .overflow_hidden()
                        .child(
                            view()
                                .flex_row()
                                .flex_wrap()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .px_3()
                                .py_2()
                                .border_b_1()
                                .border_b_color(border_subtle())
                                .child(
                                    mono(move || {
                                        let lang = ui_lang(langs);
                                        let (meta, _, _, report) = pair_header();
                                        let Some((id, method, url)) = meta else {
                                            return t(
                                                lang,
                                                "脱敏预览（当前列表暂无流量）",
                                                "Redaction preview (no flows in current list)",
                                            )
                                            .to_string();
                                        };
                                        match lang {
                                            Lang::Zh => format!(
                                                "预览 #{id} {method} {url} · 脱敏头 {} · 参数 {} · 正文字段 {}",
                                                report.redacted_headers,
                                                report.redacted_query_params,
                                                report.redacted_body_fields,
                                            ),
                                            Lang::En => format!(
                                                "Preview #{id} {method} {url} · headers {} · query {} · body fields {}",
                                                report.redacted_headers,
                                                report.redacted_query_params,
                                                report.redacted_body_fields,
                                            ),
                                        }
                                    })
                                    .tabular_nums()
                                    .flex_1()
                                    .min_w(px(160.0))
                                    .text_size(px(11.5))
                                    .text_color(fg_secondary()),
                                )
                                .child(when(
                                    move || {
                                        let (meta, _, _, report) = pair_warn();
                                        meta.is_some()
                                            && (report.has_unsanitizable_body()
                                                || report.has_strict_masked_body())
                                    },
                                    move || {
                                        let pair_warn_text = pair_warn_text.clone();
                                        view()
                                            .px_2()
                                            .py_0p5()
                                            .rounded(px(5.0))
                                            .bg(status_warn_bg())
                                            .border_1()
                                            .border_color(status_warn_fg())
                                            .flex_row()
                                            .items_center()
                                            .gap_1()
                                            .child(glyph_px(
                                                12.0,
                                                || Glyph::Alert,
                                                || color(status_warn_fg()),
                                            ))
                                            .child(
                                                text(move || -> SharedString {
                                                    let lang = ui_lang(langs);
                                                    let (_, _, _, report) = pair_warn_text();
                                                    if report.has_strict_masked_body() {
                                                        t(
                                                            lang,
                                                            "严格模式已遮罩非结构化/二进制正文",
                                                            "Unstructured/binary body masked by strict mode",
                                                        )
                                                        .into()
                                                    } else {
                                                        t(
                                                            lang,
                                                            "含非结构化/二进制正文（无法按字段脱敏）",
                                                            "Body format cannot be structurally sanitized",
                                                        )
                                                        .into()
                                                    }
                                                })
                                                .number_of_lines(1)
                                                .text_size(px(11.0))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(status_warn_fg()),
                                            )
                                    },
                                )),
                        )
                        .child(
                            view()
                                .h(move || px((window().height.0 - 300.0).clamp(180.0, 480.0)))
                                .overflow_hidden()
                                .child(
                                    diff_view()
                                        .old_value(move || {
                                            let (_, before, _, _) = pair_diff_old();
                                            SharedString::from(before)
                                        })
                                        .new_value(move || {
                                            let (_, _, after, _) = pair_diff_new();
                                            SharedString::from(after)
                                        })
                                        .layout(DiffLayout::Unified)
                                        .language("plaintext")
                                        .soft_wrap(true)
                                        .font_size(Points(12.0))
                                        .line_height(Points(18.0))
                                        .size_full(),
                                ),
                        )
                }),
        )
}

fn refresh_seq_diagram(controls: &Controls, seq_builder: SeqBuilder) -> String {
    let include_res = seq_builder.include_response.get();
    let include_query = seq_builder.include_query.get();
    let data = controls.hub.with_session(|session| {
        seq_builder.items.update(|items| {
            for item in items.iter_mut() {
                if let Some(f) = session.get(item.id) {
                    item.method = f.method.clone();
                    item.host = present::resolve_flow_host(f);
                    item.path = present::resolve_flow_target(f, true);
                    item.status = f.status;
                    item.failed = f.error.is_some() && f.status.is_none();
                    item.elapsed = present::format_elapsed(f.started, f.finished_at);
                    item.comment = f.comment.clone();
                }
            }
        });
        let mut checked_ids: Vec<u64> = seq_builder.items.with(|items| {
            items
                .iter()
                .filter(|item| item.checked)
                .map(|item| item.id)
                .collect()
        });
        checked_ids.sort_unstable();
        // The waterfall, sequence and dependency previews draw one view per
        // flow (or per arrow), and nsh keeps the grown node pools after the
        // panel closes. Preview only the newest checked flows; the copied
        // Mermaid still covers every checked flow.
        let preview_ids = &checked_ids[checked_ids.len().saturating_sub(SEQ_PREVIEW_CAP)..];
        let hydrated_flows: Vec<crate::session::Flow> = preview_ids
            .iter()
            .filter_map(|id| session.hydrated(*id))
            .collect();
        let flow_refs: Vec<&crate::session::Flow> = hydrated_flows.iter().collect();
        let mut data = present::build_sequence_diagram_data(&flow_refs, include_res, include_query);
        if checked_ids.len() > preview_ids.len() {
            let all: Vec<&crate::session::Flow> = checked_ids
                .iter()
                .filter_map(|id| session.get(*id))
                .collect();
            data.mermaid = present::build_sequence_mermaid(&all, include_res, include_query);
        }
        data
    });
    let mermaid = data.mermaid.clone();
    seq_builder.diagram.set(data);
    seq_builder.output.set(mermaid.clone());
    crate::session::trim_memory();
    mermaid
}

fn seq_candidate_target(item: &SeqCandidate, include_query: bool) -> String {
    let path = if include_query {
        item.path.as_str()
    } else {
        item.path.split(['?', '#']).next().unwrap_or(&item.path)
    };
    let path = if path.is_empty() { "/" } else { path };
    if item.host.is_empty() {
        path.to_string()
    } else if path.starts_with('/') || path.starts_with(':') {
        format!("{}{}", item.host, path)
    } else {
        format!("{}/{}", item.host, path)
    }
}

fn seq_action_button(
    label: impl IntoReactive<SharedString>,
    enabled: impl Fn() -> bool + 'static + Clone,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let enabled_border = enabled.clone();
    let enabled_hover = enabled.clone();
    let enabled_active = enabled.clone();
    let enabled_off = enabled.clone();
    let enabled_text = enabled;
    view()
        .h(px(26.0))
        .px_2p5()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || {
            if enabled_border() {
                border_control()
            } else {
                border_subtle()
            }
        })
        .bg(bg_control())
        .hover(|style| {
            style.bg(move || {
                if enabled_hover() {
                    bg_control_hover()
                } else {
                    bg_control()
                }
            })
        })
        .active(|style| style.opacity(move || if enabled_active() { 0.75_f32 } else { 1.0_f32 }))
        .cursor(Cursor::Pointer)
        .disabled(move || !enabled_off())
        .disabled_style(|style| style.cursor(Cursor::Default))
        .flex_row()
        .items_center()
        .on_press(on_press)
        .child(control_text(label).text_color(move || {
            if enabled_text() {
                fg_secondary()
            } else {
                fg_muted()
            }
        }))
}

fn seq_option_toggle(
    on: Signal<bool>,
    zh: &'static str,
    en: &'static str,
    on_toggle: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .h(px(26.0))
        .px_2p5()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || {
            if on.get() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if on.get() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if on.get() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
        .flex_row()
        .items_center()
        .gap_1p5()
        .on_press(move || {
            on.set(!on.get());
            on_toggle();
        })
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .bg(move || if on.get() { accent_blue() } else { fg_muted() }),
        )
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if on.get() {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

fn jump_to_seq_flow(
    flow_id: u64,
    selected: Signal<Option<u64>>,
    compose: Compose,
    seq_builder: SeqBuilder,
) {
    if flow_id == 0 {
        return;
    }
    reset_compose(compose);
    close_sequence_diagram(seq_builder);
    selected.set(Some(flow_id));
}

fn select_seq_mermaid_text<'a>(
    view_mode: u8,
    source_flowchart: bool,
    sequence_mermaid: &'a str,
    flowchart_mermaid: &'a str,
) -> &'a str {
    let want_flowchart = view_mode == 2 || (view_mode == 3 && source_flowchart);
    if want_flowchart && !flowchart_mermaid.is_empty() {
        flowchart_mermaid
    } else {
        sequence_mermaid
    }
}

fn active_seq_mermaid(seq_builder: SeqBuilder) -> String {
    let view_mode = seq_builder.view_mode.get();
    let source_flowchart = seq_builder.source_flowchart.get();
    let seq_text = seq_builder.output.get();
    seq_builder.diagram.with(|d| {
        select_seq_mermaid_text(view_mode, source_flowchart, &seq_text, &d.flowchart_mermaid)
            .to_string()
    })
}

fn seq_mode_tab(
    mode: Signal<u8>,
    source_flowchart: Signal<bool>,
    target: u8,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    let is_active = move || mode.get() == target;
    view()
        .h(px(22.0))
        .px_2p5()
        .rounded(px(5.0))
        .bg(move || if is_active() { bg_card() } else { bg_control() })
        .hover(|style| {
            style.bg(move || {
                if is_active() {
                    bg_card()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.85))
        .border_1()
        .border_color(move || {
            if is_active() {
                border_control()
            } else {
                bg_control()
            }
        })
        .cursor(Cursor::Pointer)
        .flex_row()
        .items_center()
        .on_press(move || {
            if target == 2 {
                source_flowchart.set(true);
            } else if target == 0 || target == 1 {
                source_flowchart.set(false);
            }
            mode.set(target);
        })
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || {
                    if is_active() {
                        fg_primary()
                    } else {
                        fg_muted()
                    }
                }),
        )
}

fn swimlane_pill_width(host: &str) -> f32 {
    ((host.chars().count() + 4) as f32 * 7.2 + 28.0).clamp(136.0, 280.0)
}

fn swimlane_layout(diagram: &present::SequenceDiagramData, viewport_w: f32) -> (Vec<f32>, f32) {
    let hosts_len = diagram.hosts.len();
    let x0 = 58.0_f32;
    if hosts_len == 0 {
        let w = viewport_w.max(520.0);
        return (vec![x0, (w - 80.0).max(280.0)], w);
    }
    let mut xs = Vec::with_capacity(hosts_len + 1);
    xs.push(x0);

    let mut prev_half = 46.0_f32;
    for (i, host) in diagram.hosts.iter().enumerate() {
        let pill_w = swimlane_pill_width(host);
        let half = pill_w * 0.5;
        let min_gap = (prev_half + half + 32.0).max(180.0);

        let max_step_span = diagram
            .steps
            .iter()
            .filter(|s| s.host_idx == i)
            .map(|s| {
                let label_w = s.label.chars().count() as f32 * 7.0;
                let note_w = if s.note.is_empty() {
                    0.0
                } else {
                    (s.note.chars().count().min(24) as f32 * 6.8 + 22.0).min(150.0)
                };
                (42.0 + label_w + note_w + 32.0).clamp(240.0, 760.0)
            })
            .fold(240.0_f32, f32::max);

        let prev_x = *xs.last().unwrap_or(&x0);
        let next_x = (prev_x + min_gap).max(x0 + max_step_span);
        xs.push(next_x);
        prev_half = half;
    }

    let right_pad = (prev_half + 28.0).max(96.0);
    let content_w = xs.last().copied().unwrap_or(x0) + right_pad;
    let total_w = content_w.max(viewport_w.max(520.0));

    if total_w > content_w && hosts_len > 0 {
        let extra = (total_w - content_w).min(240.0 * hosts_len as f32);
        let per_lane = extra / hosts_len as f32;
        for (idx, x) in xs.iter_mut().enumerate().skip(1) {
            *x += per_lane * idx as f32;
        }
    }

    (xs, total_w)
}

fn seq_candidate_max_rows(window_h: f32) -> usize {
    let win_h = if window_h > 0.0 { window_h } else { 760.0 };
    if win_h < 620.0 {
        3
    } else if win_h < 740.0 {
        5
    } else {
        7
    }
}

fn seq_preview_height(window_h: f32, candidate_count: usize) -> f32 {
    let win_h = if window_h > 0.0 { window_h } else { 760.0 };
    let max_rows = seq_candidate_max_rows(win_h);
    let candidate_h = candidate_count.clamp(1, max_rows) as f32 * 32.0;
    let chrome_h = 276.0 + candidate_h;
    (win_h - chrome_h).clamp(180.0, 960.0)
}

const SEQ_SWIMLANE_STEP_H: f32 = 42.0;

fn seq_swimlane_view(
    seq_builder: SeqBuilder,
    selected: Signal<Option<u64>>,
    compose: Compose,
) -> impl IntoElement {
    const HEADER_H: f32 = 42.0;
    let viewport_w = seq_builder.viewport_w;
    let layout = seq_builder.layout;

    view()
        .w_full()
        .h(move || {
            px(seq_preview_height(
                window().height.0,
                seq_builder.items.with(Vec::len),
            ))
        })
        .rounded(px(7.0))
        .border_1()
        .border_color(border_control())
        .bg(bg_code())
        .overflow_hidden()
        .on_layout(move |bounds: &Bounds<Points>| {
            let w = bounds.size.width.0;
            if (w - viewport_w.get_untracked()).abs() > 1.0 {
                viewport_w.set(w);
            }
        })
        .child(when(
            move || seq_builder.diagram.with(|d| d.steps.is_empty()),
            || {
                view().size_full().items_center().justify_center().child(
                    text(tr("请勾选至少一条流量", "Select at least one flow"))
                        .text_size(px(12.0))
                        .text_color(fg_muted()),
                )
            },
        ))
        .child(when(
            move || seq_builder.diagram.with(|d| !d.steps.is_empty()),
            move || {
                scroll().horizontal().size_full().child(
                    view()
                        .h_full()
                        .w(move || layout.with(|(_, total_w)| px(*total_w)))
                        .child(
                            view()
                                .w_full()
                                .h(px(HEADER_H))
                                .flex_shrink_0()
                                .relative()
                                .border_b_1()
                                .border_b_color(border_subtle())
                                .bg(bg_card())
                                .child(
                                    view()
                                        .absolute()
                                        .left(move || {
                                            let x0 = layout.with(|(xs, _)| {
                                                xs.first().copied().unwrap_or(58.0)
                                            });
                                            px((x0 - 38.0).max(8.0))
                                        })
                                        .top(px(8.0))
                                        .w(px(76.0))
                                        .h(px(26.0))
                                        .rounded(px(6.0))
                                        .border_1()
                                        .border_color(accent_blue_border())
                                        .bg(accent_blue_soft())
                                        .flex_row()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            mono("Client")
                                                .text_size(px(11.5))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(accent_blue_text()),
                                        ),
                                )
                                .child(each(
                                    move || {
                                        seq_builder.diagram.with(|d| {
                                            d.hosts
                                                .iter()
                                                .cloned()
                                                .enumerate()
                                                .collect::<Vec<(usize, String)>>()
                                        })
                                    },
                                    |(idx, _)| *idx,
                                    move |pair, _| {
                                        view()
                                            .absolute()
                                            .left(move || {
                                                let (idx, host) = pair.with(|p| p.clone());
                                                let x = layout.with(|(xs, _)| {
                                                    xs.get(idx + 1).copied().unwrap_or(300.0)
                                                });
                                                let pill_w = swimlane_pill_width(&host);
                                                px((x - pill_w * 0.5).max(8.0))
                                            })
                                            .top(px(8.0))
                                            .w(move || {
                                                pair.with(|(_, host)| px(swimlane_pill_width(host)))
                                            })
                                            .h(px(26.0))
                                            .px_2p5()
                                            .rounded(px(6.0))
                                            .border_1()
                                            .border_color(border_control())
                                            .bg(bg_control())
                                            .flex_row()
                                            .items_center()
                                            .justify_center()
                                            .gap_1()
                                            .overflow_hidden()
                                            .child(
                                                mono(move || {
                                                    format!("H{}", pair.with(|(i, _)| i + 1))
                                                })
                                                .tabular_nums()
                                                .flex_shrink_0()
                                                .text_size(px(11.0))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(accent_blue_text()),
                                            )
                                            .child(
                                                mono(move || pair.with(|(_, host)| host.clone()))
                                                    .min_w(px(0.0))
                                                    .number_of_lines(1)
                                                    .text_overflow(EllipsizeMode::Middle)
                                                    .text_size(px(11.0))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(fg_primary()),
                                            )
                                    },
                                )),
                        )
                        .child(
                            virtual_list(
                                move || seq_builder.diagram.with(|d| d.steps.len()),
                                move |at| {
                                    seq_swimlane_row(seq_builder, layout, selected, compose, at)
                                },
                            )
                            .keys(move || {
                                seq_builder.diagram.with(|d| {
                                    d.steps
                                        .iter()
                                        .map(|s| (s.step_no, s.flow_id, s.is_request))
                                        .collect::<Vec<_>>()
                                })
                            })
                            .estimated_item_size(px(SEQ_SWIMLANE_STEP_H))
                            .flex_1()
                            .w_full(),
                        ),
                )
            },
        ))
}

fn seq_swimlane_row(
    seq_builder: SeqBuilder,
    layout: Memo<(Vec<f32>, f32)>,
    selected: Signal<Option<u64>>,
    compose: Compose,
    at: Position,
) -> impl IntoElement {
    let step = move || {
        seq_builder
            .diagram
            .with(|d| d.steps.get(at.index()).cloned())
            .unwrap_or_default()
    };
    view()
        .w(move || layout.with(|(_, total_w)| px(*total_w)))
        .h(px(SEQ_SWIMLANE_STEP_H))
        .relative()
        .hover(|style| style.bg(bg_control_hover()))
        .cursor(Cursor::Pointer)
        .on_press(move || {
            jump_to_seq_flow(step().flow_id, selected, compose, seq_builder);
        })
        .child(
            canvas(move || {
                let s = step();
                layout.with(|(xs, _)| {
                    let mut p = Painter::new();
                    for (k, &x) in xs.iter().enumerate() {
                        p = p
                            .line_width(1.0)
                            .stroke_color(color(border_control()))
                            .stroke_svg(&format!("M{x:.1} 0 L{x:.1} {SEQ_SWIMLANE_STEP_H:.1}"));
                        let active = k == 0 || s.active_hosts.get(k - 1).copied().unwrap_or(false);
                        if active {
                            p = p
                                .line_width(3.6)
                                .stroke_color(color(accent_blue()))
                                .stroke_svg(&format!("M{x:.1} 0 L{x:.1} {SEQ_SWIMLANE_STEP_H:.1}"));
                        }
                    }
                    let x_client = xs.first().copied().unwrap_or(58.0);
                    let x_host = xs.get(s.host_idx + 1).copied().unwrap_or(x_client + 280.0);
                    let (x_from, x_to) = if s.is_request {
                        (x_client + 3.0, x_host - 3.0)
                    } else {
                        (x_host - 3.0, x_client + 3.0)
                    };
                    let y_arrow = 29.0_f32;
                    let stroke = color(if s.is_error {
                        status_err_fg()
                    } else if s.is_request {
                        accent_blue()
                    } else {
                        status_ok_fg()
                    });
                    if s.is_request {
                        p = p.line_width(1.6).stroke_color(stroke).stroke_svg(&format!(
                            "M{x_from:.1} {y_arrow:.1} L{x_to:.1} {y_arrow:.1}"
                        ));
                    } else {
                        let dir = if x_to >= x_from { 1.0_f32 } else { -1.0_f32 };
                        let span = (x_to - x_from).abs();
                        let mut cur = 0.0_f32;
                        let mut segs = String::new();
                        while cur < span {
                            let next = (cur + 6.0).min(span);
                            let a = x_from + dir * cur;
                            let b = x_from + dir * next;
                            use std::fmt::Write;
                            let _ = write!(segs, "M{a:.1} {y_arrow:.1} L{b:.1} {y_arrow:.1} ");
                            cur += 10.0;
                        }
                        if !segs.is_empty() {
                            p = p.line_width(1.5).stroke_color(stroke).stroke_svg(&segs);
                        }
                    }
                    let dir = if x_to >= x_from { 1.0_f32 } else { -1.0_f32 };
                    let ax = x_to - dir * 6.0;
                    let ay1 = y_arrow - 3.4;
                    let ay2 = y_arrow + 3.4;
                    p.line_width(1.6).stroke_color(stroke).stroke_svg(&format!(
                        "M{ax:.1} {ay1:.1} L{x_to:.1} {y_arrow:.1} L{ax:.1} {ay2:.1}"
                    ))
                })
            })
            .size_full(),
        )
        .child(
            view()
                .absolute()
                .left(move || {
                    let x0 = layout.with(|(xs, _)| xs.first().copied().unwrap_or(58.0));
                    px(x0 + 8.0)
                })
                .top(px(5.0))
                .max_w(move || {
                    let host_idx = step().host_idx;
                    layout.with(|(xs, _)| {
                        let x0 = xs.first().copied().unwrap_or(58.0);
                        let x_host = xs.get(host_idx + 1).copied().unwrap_or(x0 + 280.0);
                        px((x_host - x0 - 16.0).max(160.0))
                    })
                })
                .flex_row()
                .items_center()
                .gap_1()
                .overflow_hidden()
                .child(
                    view()
                        .px_1p5()
                        .py(px(0.5))
                        .rounded(px(3.5))
                        .flex_shrink_0()
                        .bg(bg_control())
                        .child(
                            mono(move || step().step_no.to_string())
                                .tabular_nums()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(fg_secondary()),
                        ),
                )
                .child(
                    view()
                        .px_1p5()
                        .py(px(0.5))
                        .rounded(px(3.5))
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .bg(bg_card())
                        .child(
                            mono(move || step().label)
                                .max_w(move || {
                                    let s = step();
                                    layout.with(|(xs, _)| {
                                        let x0 = xs.first().copied().unwrap_or(58.0);
                                        let x_host =
                                            xs.get(s.host_idx + 1).copied().unwrap_or(x0 + 280.0);
                                        let reserve = if !s.note.is_empty() { 180.0 } else { 56.0 };
                                        px((x_host - x0 - reserve).max(120.0))
                                    })
                                })
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Middle)
                                .text_size(px(11.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || {
                                    let s = step();
                                    if s.is_error {
                                        status_err_fg()
                                    } else if s.is_request {
                                        fg_primary()
                                    } else {
                                        status_ok_fg()
                                    }
                                }),
                        ),
                )
                .child(when(
                    move || !step().note.is_empty(),
                    move || {
                        view()
                            .px_1p5()
                            .py(px(0.5))
                            .rounded(px(3.5))
                            .max_w(px(140.0))
                            .flex_shrink_0()
                            .bg(accent_blue_soft())
                            .child(
                                text(move || format!("# {}", step().note))
                                    .number_of_lines(1)
                                    .text_overflow(EllipsizeMode::Tail)
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(accent_blue_text()),
                            )
                    },
                )),
        )
}

fn dep_kind_prefix(kind: present::DepKind, lang: Lang) -> &'static str {
    match (kind, lang) {
        (present::DepKind::Cookie, Lang::Zh) => "[Cookie] 依赖",
        (present::DepKind::Cookie, Lang::En) => "[Cookie]",
        (present::DepKind::Token, Lang::Zh) => "[Token] 依赖",
        (present::DepKind::Token, Lang::En) => "[Token]",
        (present::DepKind::Redirect, Lang::Zh) => "↪ 重定向依赖",
        (present::DepKind::Redirect, Lang::En) => "↪ Redirect",
        (present::DepKind::QueryParam, Lang::Zh) => "[Query] 参数依赖",
        (present::DepKind::QueryParam, Lang::En) => "[Query]",
        (present::DepKind::BodyOrPath, Lang::Zh) => "[Data] 字段依赖",
        (present::DepKind::BodyOrPath, Lang::En) => "[Data]",
        (present::DepKind::Sequential, Lang::Zh) => "[After] 时序后继",
        (present::DepKind::Sequential, Lang::En) => "[After]",
    }
}

#[derive(Clone, PartialEq, Default)]
struct DepFlatRow {
    stage_no: usize,
    stage_bar_count: usize,
    stage_offset_label: String,
    stage_span_label: String,
    stage_has_hard_dep: bool,
    is_first_in_stage: bool,
    is_last_in_stage: bool,
    bar: present::WaterfallBar,
}

fn build_dep_flat_rows(d: &present::SequenceDiagramData) -> Vec<DepFlatRow> {
    let mut out = Vec::with_capacity(d.bars.len());
    for stage in &d.stages {
        let count = stage.bar_indices.len();
        let has_hard = stage.bar_indices.iter().any(|&idx| {
            d.bars
                .get(idx)
                .is_some_and(|b| b.deps.iter().any(|dep| dep.kind.is_hard()))
        });
        for (pos, &bar_idx) in stage.bar_indices.iter().enumerate() {
            let Some(bar) = d.bars.get(bar_idx).cloned() else {
                continue;
            };
            out.push(DepFlatRow {
                stage_no: stage.stage_no,
                stage_bar_count: count,
                stage_offset_label: stage.offset_label.clone(),
                stage_span_label: stage.span_label.clone(),
                stage_has_hard_dep: has_hard,
                is_first_in_stage: pos == 0,
                is_last_in_stage: pos + 1 == count,
                bar,
            });
        }
    }
    out
}

fn seq_dependency_view(
    seq_builder: SeqBuilder,
    selected: Signal<Option<u64>>,
    compose: Compose,
) -> impl IntoElement {
    let langs = ui_langs();
    let flat_rows = seq_builder.flat_rows;
    view()
        .w_full()
        .h(move || {
            px(seq_preview_height(
                window().height.0,
                seq_builder.items.with(Vec::len),
            ))
        })
        .rounded(px(7.0))
        .border_1()
        .border_color(border_control())
        .bg(bg_code())
        .overflow_hidden()
        .child(when(
            move || seq_builder.diagram.with(|d| d.stages.is_empty()),
            || {
                view()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .child(
                        text(tr("请勾选至少一条流量", "Select at least one flow"))
                            .text_size(px(12.0))
                            .text_color(fg_muted()),
                    )
            },
        ))
        .child(when(
            move || seq_builder.diagram.with(|d| !d.stages.is_empty()),
            move || {
                view()
                    .size_full()
                    .child(
                        view()
                            .h(px(30.0))
                            .px_3()
                            .flex_shrink_0()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .border_b_1()
                            .border_b_color(border_subtle())
                            .bg(bg_card())
                            .child(
                                text(move || {
                                    let lang = ui_lang(langs);
                                    seq_builder.diagram.with(|d| {
                                        let stage_cnt = d.stages.len();
                                        let conc_cnt = d
                                            .stages
                                            .iter()
                                            .filter(|s| s.bar_indices.len() > 1)
                                            .count();
                                        let hard_cnt = d.hard_dep_count;
                                        match lang {
                                            Lang::Zh => format!(
                                                "共 {stage_cnt} 个阶段 · {conc_cnt} 组并发 · 识别出 {hard_cnt} 处参数/Cookie 依赖"
                                            ),
                                            Lang::En => format!(
                                                "{stage_cnt} stages · {conc_cnt} concurrent groups · {hard_cnt} data dependencies"
                                            ),
                                        }
                                    })
                                })
                                .number_of_lines(1)
                                .text_size(px(11.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg_secondary()),
                            )
                            .child(
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        view()
                                            .px_1p5()
                                            .py(px(1.0))
                                            .rounded(px(4.0))
                                            .bg(accent_blue_soft())
                                            .child(
                                                text(tr("↪ 参数/Cookie 传递", "↪ Data flow"))
                                                    .text_size(px(11.0))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(accent_blue_text()),
                                            ),
                                    )
                                    .child(
                                        view()
                                            .px_1p5()
                                            .py(px(1.0))
                                            .rounded(px(4.0))
                                            .bg(status_warn_bg())
                                            .child(
                                                text(tr("★ 关键路径", "★ Critical path"))
                                                    .text_size(px(11.0))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(status_warn_fg()),
                                            ),
                                    ),
                            ),
                    )
                    .child(
                        virtual_list(
                            move || flat_rows.with(Vec::len),
                            move |at| {
                                seq_dependency_row(
                                    seq_builder,
                                    flat_rows,
                                    selected,
                                    compose,
                                    langs,
                                    at,
                                )
                            },
                        )
                        .keys(move || {
                            flat_rows.with(|rows| {
                                rows.iter().map(|r| r.bar.flow_id).collect::<Vec<_>>()
                            })
                        })
                        .estimated_item_size(px(56.0))
                        .content_padding(px(12.0))
                        .flex_1()
                        .w_full(),
                    )
            },
        ))
}

fn seq_dep_badge(
    rows: Memo<Vec<DepFlatRow>>,
    langs: Signal<Option<Lang>>,
    at: Position,
    slot: usize,
) -> impl IntoElement {
    let dep = move || {
        rows.with(|r| {
            r.get(at.index())
                .and_then(|row| row.bar.deps.get(slot))
                .cloned()
        })
    };
    when(
        move || dep().is_some(),
        move || {
            view()
                .px_2()
                .py(px(1.5))
                .rounded(px(4.0))
                .border_1()
                .border_color(move || {
                    if dep().is_some_and(|d| d.kind.is_hard()) {
                        accent_blue_border()
                    } else {
                        border_subtle()
                    }
                })
                .bg(move || {
                    if dep().is_some_and(|d| d.kind.is_hard()) {
                        accent_blue_soft()
                    } else {
                        bg_control()
                    }
                })
                .child(
                    mono(move || {
                        let Some(d) = dep() else {
                            return String::new();
                        };
                        let prefix = dep_kind_prefix(d.kind, ui_lang(langs));
                        format!("{prefix} #{} · {}", d.from_flow_id, d.label)
                    })
                    .tabular_nums()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || {
                        if dep().is_some_and(|d| d.kind.is_hard()) {
                            accent_blue_text()
                        } else {
                            fg_secondary()
                        }
                    }),
                )
        },
    )
}

fn seq_dependency_row(
    seq_builder: SeqBuilder,
    rows: Memo<Vec<DepFlatRow>>,
    selected: Signal<Option<u64>>,
    compose: Compose,
    langs: Signal<Option<Lang>>,
    at: Position,
) -> impl IntoElement {
    let row = move || {
        rows.with(|r| r.get(at.index()).cloned())
            .unwrap_or_default()
    };
    view()
        .w_full()
        .child(when(
            move || {
                let r = row();
                r.is_first_in_stage && r.stage_no > 1
            },
            move || {
                view()
                    .w_full()
                    .h(px(28.0))
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(
                        mono("↓")
                            .text_size(px(13.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(accent_blue_text()),
                    )
                    .child(
                        view()
                            .px_2()
                            .py(px(1.5))
                            .rounded(px(4.0))
                            .border_1()
                            .border_color(border_subtle())
                            .bg(bg_card())
                            .child(
                                text(move || {
                                    let r = row();
                                    let st_no = r.stage_no;
                                    match (r.stage_has_hard_dep, ui_lang(langs)) {
                                        (true, Lang::Zh) => {
                                            format!("携带前序阶段返回参数进入阶段 {st_no}")
                                        }
                                        (true, Lang::En) => {
                                            format!("Passes response data into Stage {st_no}")
                                        }
                                        (false, Lang::Zh) => {
                                            format!(
                                                "等待阶段 {} 响应完成后触发",
                                                st_no.saturating_sub(1)
                                            )
                                        }
                                        (false, Lang::En) => {
                                            format!(
                                                "Triggered after Stage {} completed",
                                                st_no.saturating_sub(1)
                                            )
                                        }
                                    }
                                })
                                .text_size(px(11.0))
                                .text_color(fg_muted()),
                            ),
                    )
            },
        ))
        .child(when(
            move || row().is_first_in_stage,
            move || {
                view()
                    .w_full()
                    .h(px(26.0))
                    .px_3()
                    .rounded_t(px(7.0))
                    .border_t_1()
                    .border_l_1()
                    .border_r_1()
                    .border_b_1()
                    .border_color(border_control())
                    .border_b_color(border_subtle())
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .bg(bg_control())
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                text(move || {
                                    let no = row().stage_no;
                                    match ui_lang(langs) {
                                        Lang::Zh => format!("阶段 {no}"),
                                        Lang::En => format!("Stage {no}"),
                                    }
                                })
                                .tabular_nums()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(fg_primary()),
                            )
                            .child(
                                view()
                                    .px_1p5()
                                    .py(px(0.5))
                                    .rounded(px(4.0))
                                    .bg(move || {
                                        if row().stage_bar_count > 1 {
                                            accent_blue_soft()
                                        } else {
                                            bg_card()
                                        }
                                    })
                                    .child(
                                        text(move || {
                                            let count = row().stage_bar_count;
                                            match (count > 1, ui_lang(langs)) {
                                                (true, Lang::Zh) => format!("并发 ×{count}"),
                                                (true, Lang::En) => {
                                                    format!("Concurrent ×{count}")
                                                }
                                                (false, Lang::Zh) => "串行单步".to_string(),
                                                (false, Lang::En) => "Sequential".to_string(),
                                            }
                                        })
                                        .tabular_nums()
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(
                                            move || {
                                                if row().stage_bar_count > 1 {
                                                    accent_blue_text()
                                                } else {
                                                    fg_muted()
                                                }
                                            },
                                        ),
                                    ),
                            ),
                    )
                    .child(
                        mono(move || {
                            let r = row();
                            format!("[{}] · {}", r.stage_offset_label, r.stage_span_label)
                        })
                        .tabular_nums()
                        .text_size(px(11.0))
                        .text_color(fg_muted()),
                    )
            },
        ))
        .child(
            view()
                .w_full()
                .px_3()
                .py_2()
                .gap_1p5()
                .bg(bg_card())
                .border_l_1()
                .border_r_1()
                .border_color(border_control())
                .border_t(move || {
                    if row().is_first_in_stage {
                        px(0.0)
                    } else {
                        px(1.0)
                    }
                })
                .border_t_color(border_subtle())
                .border_b(move || {
                    if row().is_last_in_stage {
                        px(1.0)
                    } else {
                        px(0.0)
                    }
                })
                .border_b_color(border_control())
                .rounded_b(move || {
                    if row().is_last_in_stage {
                        px(7.0)
                    } else {
                        px(0.0)
                    }
                })
                .hover(|style| style.bg(bg_control_hover()))
                .cursor(Cursor::Pointer)
                .on_press(move || {
                    jump_to_seq_flow(row().bar.flow_id, selected, compose, seq_builder);
                })
                .child(
                    view()
                        .w_full()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            view()
                                .w(px(3.0))
                                .h(px(16.0))
                                .rounded_full()
                                .flex_shrink_0()
                                .bg(move || {
                                    let b = row().bar;
                                    if b.failed || b.status.is_some_and(|c| c >= 400) {
                                        status_err_fg()
                                    } else if b.status.is_some_and(|c| (300..400).contains(&c)) {
                                        status_warn_fg()
                                    } else if b.status.is_some() {
                                        status_ok_fg()
                                    } else {
                                        accent_blue()
                                    }
                                }),
                        )
                        .child(
                            mono(move || format!("#{}", row().bar.flow_id))
                                .tabular_nums()
                                .min_w(px(36.0))
                                .flex_shrink_0()
                                .text_size(px(11.5))
                                .text_color(fg_muted()),
                        )
                        .child(
                            view()
                                .min_w(px(44.0))
                                .px(px(5.0))
                                .py_0p5()
                                .rounded(px(4.0))
                                .flex_shrink_0()
                                .items_center()
                                .justify_center()
                                .bg(move || method_bg(&row().bar.method))
                                .child(
                                    mono(move || row().bar.method)
                                        .text_size(px(10.5))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(move || method_fg(&row().bar.method)),
                                ),
                        )
                        .child(
                            mono(move || {
                                let b = row().bar;
                                if b.host.is_empty() {
                                    b.target
                                } else if b.target.starts_with('/') {
                                    format!("{}{}", b.host, b.target)
                                } else {
                                    format!("{}/{}", b.host, b.target)
                                }
                            })
                            .flex_1()
                            .min_w(px(0.0))
                            .number_of_lines(1)
                            .text_overflow(EllipsizeMode::Middle)
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg_primary()),
                        )
                        .child(when(
                            move || row().bar.on_critical_path,
                            || {
                                view()
                                    .px_1p5()
                                    .py(px(0.5))
                                    .rounded(px(3.5))
                                    .flex_shrink_0()
                                    .bg(status_warn_bg())
                                    .child(
                                        text(tr("★ 关键路径", "★ Critical"))
                                            .text_size(px(11.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(status_warn_fg()),
                                    )
                            },
                        ))
                        .child(
                            mono(move || {
                                let b = row().bar;
                                list_status(b.status, b.failed)
                            })
                            .tabular_nums()
                            .min_w(px(32.0))
                            .text_right()
                            .flex_shrink_0()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || {
                                let b = row().bar;
                                http_status_fg(b.status, b.failed)
                            }),
                        )
                        .child(
                            mono(move || {
                                let b = row().bar;
                                if b.size_label.is_empty() {
                                    format!("[{}] · {}", b.offset_label, b.duration_label)
                                } else {
                                    format!(
                                        "[{}] · {} · {}",
                                        b.offset_label, b.duration_label, b.size_label
                                    )
                                }
                            })
                            .tabular_nums()
                            .flex_shrink_0()
                            .text_size(px(11.0))
                            .text_color(fg_muted()),
                        ),
                )
                .child(when(
                    move || {
                        let b = row().bar;
                        !b.deps.is_empty() || !b.comment.is_empty()
                    },
                    move || {
                        view()
                            .w_full()
                            .pl_3()
                            .flex_row()
                            .flex_wrap()
                            .items_center()
                            .gap_1p5()
                            .child(seq_dep_badge(rows, langs, at, 0))
                            .child(seq_dep_badge(rows, langs, at, 1))
                            .child(seq_dep_badge(rows, langs, at, 2))
                            .child(seq_dep_badge(rows, langs, at, 3))
                            .child(when(
                                move || !row().bar.comment.is_empty(),
                                move || {
                                    view()
                                        .px_2()
                                        .py(px(1.5))
                                        .rounded(px(4.0))
                                        .bg(accent_blue_soft())
                                        .child(
                                            text(move || format!("# {}", row().bar.comment))
                                                .text_size(px(11.0))
                                                .text_color(accent_blue_text()),
                                        )
                                },
                            ))
                    },
                )),
        )
}

const SEQ_WF_ROW_H: f32 = 30.0;

fn seq_wf_left_col_width(viewport_w: f32) -> f32 {
    if viewport_w > 0.0 {
        (viewport_w * 0.38).round().clamp(132.0, 260.0)
    } else {
        240.0
    }
}

fn seq_waterfall_view(
    seq_builder: SeqBuilder,
    selected: Signal<Option<u64>>,
    compose: Compose,
) -> impl IntoElement {
    const WF_HEADER_H: f32 = 28.0;
    const WF_RULER_H: f32 = 22.0;
    let track_w = seq_builder.track_w;
    let viewport_w = seq_builder.viewport_w;

    view()
        .w_full()
        .h(move || {
            px(seq_preview_height(
                window().height.0,
                seq_builder.items.with(Vec::len),
            ))
        })
        .rounded(px(7.0))
        .border_1()
        .border_color(border_control())
        .bg(bg_code())
        .overflow_hidden()
        .on_layout(move |bounds: &Bounds<Points>| {
            let w = bounds.size.width.0;
            if (w - viewport_w.get_untracked()).abs() > 1.0 {
                viewport_w.set(w);
            }
        })
        .child(when(
            move || seq_builder.diagram.with(|d| d.bars.is_empty()),
            || {
                view()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .child(
                        text(tr("请勾选至少一条流量", "Select at least one flow"))
                            .text_size(px(12.0))
                            .text_color(fg_muted()),
                    )
            },
        ))
        .child(when(
            move || seq_builder.diagram.with(|d| !d.bars.is_empty()),
            move || {
                view()
                    .size_full()
                    .child(
                        view()
                            .h(px(WF_HEADER_H))
                            .px_3()
                            .flex_shrink_0()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .border_b_1()
                            .border_b_color(border_subtle())
                            .bg(bg_card())
                            .child(
                                text(tr(
                                    "关键路径瀑布图（折线连接上下游依赖，垂直重叠代表同时并发）",
                                    "Critical-path waterfall (connectors show dependencies; vertical overlap shows concurrency)",
                                ))
                                .flex_1()
                                .min_w(px(0.0))
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Tail)
                                .text_size(px(11.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg_secondary()),
                            )
                            .child(
                                mono(move || {
                                    let hovered = seq_builder.hover_bar.get();
                                    let langs = ui_langs();
                                    seq_builder.diagram.with(|d| {
                                        if let Some(idx) = hovered
                                            && let Some(bar) = d.bars.get(idx)
                                        {
                                            let dep_note = bar
                                                .deps
                                                .first()
                                                .map(|dep| {
                                                    let prefix =
                                                        t(ui_lang(langs), " · 依赖 #", " · dep #");
                                                    format!(
                                                        "{prefix}{} ({})",
                                                        dep.from_flow_id, dep.label
                                                    )
                                                })
                                                .unwrap_or_default();
                                            return format!(
                                                "#{} {} {} · [+{:.0}ms .. +{:.0}ms] ({}){dep_note}",
                                                bar.flow_id,
                                                bar.method,
                                                bar.target,
                                                bar.start_ms,
                                                bar.end_ms,
                                                bar.duration_label
                                            );
                                        }
                                        let max_ms = d
                                            .bars
                                            .iter()
                                            .map(|b| b.end_ms)
                                            .fold(0.0_f64, f64::max);
                                        format!("0 ms .. {max_ms:.0} ms")
                                    })
                                })
                                .tabular_nums()
                                .flex_shrink_0()
                                .number_of_lines(1)
                                .text_size(px(11.0))
                                .text_color(accent_blue_text()),
                            ),
                    )
                    .child(
                        view()
                            .h(px(WF_RULER_H))
                            .w_full()
                            .flex_shrink_0()
                            .flex_row()
                            .items_center()
                            .border_b_1()
                            .border_b_color(border_subtle())
                            .bg(bg_control())
                            .child(
                                view()
                                    .w(move || px(seq_wf_left_col_width(viewport_w.get())))
                                    .h_full()
                                    .flex_shrink_0()
                                    .px_2p5()
                                    .flex_row()
                                    .items_center()
                                    .border_r_1()
                                    .border_r_color(border_subtle())
                                    .child(
                                        text(tr("阶段 / 接口", "Stage / Endpoint"))
                                            .number_of_lines(1)
                                            .text_overflow(EllipsizeMode::Tail)
                                            .text_size(px(11.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(fg_muted()),
                                    ),
                            )
                            .child(
                                view()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .h_full()
                                    .px_3()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .on_layout(move |bounds: &Bounds<Points>| {
                                        let w = bounds.size.width.0;
                                        if (w - track_w.get_untracked()).abs() > 1.0 {
                                            track_w.set(w);
                                        }
                                    })
                                    .children([0.0_f64, 0.25, 0.5, 0.75, 1.0].into_iter().map(
                                        move |ratio| {
                                            mono(move || {
                                                let tw = track_w.get();
                                                if tw > 0.0
                                                    && tw < 220.0
                                                    && (ratio == 0.25 || ratio == 0.75)
                                                {
                                                    return String::new();
                                                }
                                                let max_ms = seq_builder.diagram.with(|d| {
                                                    d.bars
                                                        .iter()
                                                        .map(|b| b.end_ms)
                                                        .fold(1.0_f64, f64::max)
                                                });
                                                let ms = max_ms * ratio;
                                                if ms >= 1000.0 {
                                                    format!("+{:.1}s", ms / 1000.0)
                                                } else {
                                                    format!("+{ms:.0}ms")
                                                }
                                            })
                                            .tabular_nums()
                                            .text_size(px(11.0))
                                            .text_color(fg_muted())
                                        },
                                    )),
                            ),
                    )
                    .child(
                        virtual_list(
                            move || seq_builder.diagram.with(|d| d.bars.len()),
                            move |at| seq_waterfall_row(seq_builder, track_w, selected, compose, at),
                        )
                        .keys(move || {
                            seq_builder
                                .diagram
                                .with(|d| d.bars.iter().map(|b| b.flow_id).collect::<Vec<_>>())
                        })
                        .estimated_item_size(px(SEQ_WF_ROW_H))
                        .flex_1()
                        .w_full(),
                    )
            },
        ))
}

fn seq_waterfall_row(
    seq_builder: SeqBuilder,
    track_w: Signal<f32>,
    selected: Signal<Option<u64>>,
    compose: Compose,
    at: Position,
) -> impl IntoElement {
    let viewport_w = seq_builder.viewport_w;
    let bar_item = move || {
        seq_builder
            .diagram
            .with(|d| d.bars.get(at.index()).cloned())
            .unwrap_or_default()
    };
    view()
        .w_full()
        .h(px(SEQ_WF_ROW_H))
        .flex_row()
        .items_center()
        .cursor(Cursor::Pointer)
        .on_press(move || {
            jump_to_seq_flow(bar_item().flow_id, selected, compose, seq_builder);
        })
        .bg(move || {
            if seq_builder.hover_bar.get() == Some(at.index()) {
                bg_control_hover()
            } else if at.index() % 2 == 1 {
                bg_code()
            } else {
                bg_card()
            }
        })
        .border_t(move || if at.index() > 0 { px(1.0) } else { px(0.0) })
        .border_t_color(border_subtle())
        .on_mouse_move(move |_: &MouseEvent| {
            seq_builder.hover_bar.set(Some(at.index()));
        })
        .on_mouse_leave(move |_: &MouseEvent| {
            if seq_builder.hover_bar.get() == Some(at.index()) {
                seq_builder.hover_bar.set(None);
            }
        })
        .child(
            view()
                .w(move || px(seq_wf_left_col_width(viewport_w.get())))
                .h_full()
                .px_2p5()
                .gap_1p5()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .border_r_1()
                .border_r_color(border_subtle())
                .child(
                    view()
                        .px_1()
                        .py(px(0.5))
                        .rounded(px(3.0))
                        .flex_shrink_0()
                        .bg(bg_control())
                        .child(
                            mono(move || format!("S{}", bar_item().stage))
                                .tabular_nums()
                                .text_size(px(10.5))
                                .font_weight(FontWeight::BOLD)
                                .text_color(accent_blue_text()),
                        ),
                )
                .child(
                    mono(move || format!("#{}", bar_item().flow_id))
                        .tabular_nums()
                        .flex_shrink_0()
                        .text_size(px(11.0))
                        .text_color(fg_muted()),
                )
                .child(
                    view()
                        .px(px(4.5))
                        .py(px(1.0))
                        .rounded(px(3.5))
                        .flex_shrink_0()
                        .bg(move || method_bg(&bar_item().method))
                        .child(
                            mono(move || bar_item().method)
                                .text_size(px(10.5))
                                .font_weight(FontWeight::BOLD)
                                .text_color(move || method_fg(&bar_item().method)),
                        ),
                )
                .child(
                    mono(move || bar_item().target)
                        .flex_1()
                        .min_w(px(0.0))
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Middle)
                        .text_size(px(11.0))
                        .text_color(fg_primary()),
                )
                .child(
                    mono(move || {
                        let b = bar_item();
                        list_status(b.status, b.failed)
                    })
                    .tabular_nums()
                    .flex_shrink_0()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || {
                        let b = bar_item();
                        http_status_fg(b.status, b.failed)
                    }),
                ),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .relative()
                .overflow_hidden()
                .child(
                    canvas(move || {
                        let row_i = at.index();
                        let w = (track_w.get() - 24.0).max(48.0);
                        let pad_x = 12.0_f32;
                        seq_builder.diagram.with(|d| {
                            let bars = &d.bars;
                            let max_ms = bars
                                .iter()
                                .map(|b| b.end_ms)
                                .fold(1.0_f64, f64::max)
                                as f32;
                            let y_mid = SEQ_WF_ROW_H * 0.5;
                            let mut p = Painter::new();

                            // 1. Vertical time grid lines for this row
                            for ratio in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
                                let gx = pad_x + ratio * w;
                                p = p
                                    .line_width(1.0)
                                    .stroke_color(color(border_subtle()))
                                    .stroke_svg(&format!("M{gx:.1} 0 L{gx:.1} {SEQ_WF_ROW_H:.1}"));
                            }

                            // 2. Cross-row dependency connector segments passing through `row_i`
                            for (target_idx, target_bar) in bars.iter().enumerate() {
                                for dep in &target_bar.deps {
                                    let from_idx = dep.from_index;
                                    if from_idx >= target_idx
                                        || row_i < from_idx
                                        || row_i > target_idx
                                    {
                                        continue;
                                    }
                                    let Some(parent) = bars.get(from_idx) else {
                                        continue;
                                    };
                                    let x_parent_end =
                                        pad_x + (parent.end_ms as f32 / max_ms) * w;
                                    let x_target_start =
                                        pad_x + (target_bar.start_ms as f32 / max_ms) * w;
                                    let elbow_x =
                                        x_parent_end.min(x_target_start - 2.0).max(pad_x);
                                    let conn_color = if dep.kind.is_hard() {
                                        color(accent_blue())
                                    } else {
                                        color(status_warn_fg())
                                    };
                                    let lw = if dep.kind.is_hard() { 1.5 } else { 1.1 };
                                    if row_i == from_idx {
                                        p = p.line_width(lw).stroke_color(conn_color).stroke_svg(
                                            &format!(
                                                "M{x_parent_end:.1} {y_mid:.1} L{elbow_x:.1} {y_mid:.1} L{elbow_x:.1} {SEQ_WF_ROW_H:.1}"
                                            ),
                                        );
                                    } else if row_i < target_idx {
                                        p = p.line_width(lw).stroke_color(conn_color).stroke_svg(
                                            &format!(
                                                "M{elbow_x:.1} 0 L{elbow_x:.1} {SEQ_WF_ROW_H:.1}"
                                            ),
                                        );
                                    } else {
                                        p = p.line_width(lw).stroke_color(conn_color).stroke_svg(
                                            &format!(
                                                "M{elbow_x:.1} 0 L{elbow_x:.1} {y_mid:.1} L{x_target_start:.1} {y_mid:.1}"
                                            ),
                                        );
                                        let ax = (x_target_start - 4.2).max(pad_x);
                                        let ay1 = y_mid - 2.6;
                                        let ay2 = y_mid + 2.6;
                                        p = p.line_width(1.4).stroke_color(conn_color).stroke_svg(
                                            &format!(
                                                "M{ax:.1} {ay1:.1} L{x_target_start:.1} {y_mid:.1} L{ax:.1} {ay2:.1}"
                                            ),
                                        );
                                    }
                                }
                            }

                            // 3. Horizontal Span Bar for `row_i`
                            if let Some(bar) = bars.get(row_i) {
                                let x1 = pad_x + (bar.start_ms as f32 / max_ms) * w;
                                let x2 = pad_x + (bar.end_ms as f32 / max_ms) * w;
                                let bw = (x2 - x1).max(5.0);
                                let y_top = 7.0_f32;
                                let bh = SEQ_WF_ROW_H - 14.0;
                                let fill = color(
                                    if bar.failed || bar.status.is_some_and(|c| c >= 400) {
                                        status_err_fg()
                                    } else if bar.status.is_some_and(|c| (300..400).contains(&c)) {
                                        status_warn_fg()
                                    } else if bar.status.is_some() {
                                        status_ok_fg()
                                    } else {
                                        accent_blue()
                                    },
                                );
                                let rect_svg = format!(
                                    "M{x1:.1} {y_top:.1} H{:.1} V{:.1} H{x1:.1} Z",
                                    x1 + bw,
                                    y_top + bh
                                );
                                p = p.fill_color(fill).fill_svg(&rect_svg);
                                if bar.on_critical_path {
                                    p = p
                                        .line_width(1.5)
                                        .stroke_color(color(status_warn_fg()))
                                        .stroke_svg(&rect_svg);
                                }
                            }
                            p
                        })
                    })
                    .size_full(),
                )
                .child(
                    view()
                        .absolute()
                        .left(move || {
                            let w = (track_w.get() - 24.0).max(48.0);
                            let max_ms = seq_builder.diagram.with(|d| {
                                d.bars
                                    .iter()
                                    .map(|b| b.end_ms)
                                    .fold(1.0_f64, f64::max)
                            }) as f32;
                            let b = bar_item();
                            let x1 = 12.0 + (b.start_ms as f32 / max_ms) * w;
                            let x2 = 12.0 + (b.end_ms as f32 / max_ms) * w;
                            let after = x1.max(x2) + 6.0;
                            if after + 104.0 < w + 12.0 {
                                px(after)
                            } else {
                                px((x1 + 4.0).min((w - 72.0).max(14.0)).max(14.0))
                            }
                        })
                        .top(px(5.0))
                        .px_1p5()
                        .py(px(0.5))
                        .rounded(px(3.0))
                        .bg(bg_card())
                        .child(
                            mono(move || {
                                let b = bar_item();
                                if let Some(dep) = b.deps.first() {
                                    if dep.kind.is_hard() {
                                        format!("{} · dep#{}", b.duration_label, dep.from_flow_id)
                                    } else {
                                        format!("{} · after#{}", b.duration_label, dep.from_flow_id)
                                    }
                                } else {
                                    b.duration_label
                                }
                            })
                            .tabular_nums()
                            .number_of_lines(1)
                            .text_size(px(11.0))
                            .text_color(fg_secondary()),
                        ),
                ),
        )
}

fn seq_source_kind_chip(
    source_flowchart: Signal<bool>,
    target_flowchart: bool,
    label: &'static str,
) -> impl IntoElement {
    let is_active = move || source_flowchart.get() == target_flowchart;
    view()
        .h(px(20.0))
        .px_2()
        .rounded(px(4.0))
        .bg(move || if is_active() { bg_card() } else { bg_control() })
        .hover(|style| {
            style.bg(move || {
                if is_active() {
                    bg_card()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.opacity(0.85))
        .border_1()
        .border_color(move || {
            if is_active() {
                border_control()
            } else {
                bg_control()
            }
        })
        .cursor(Cursor::Pointer)
        .flex_row()
        .items_center()
        .on_press(move || source_flowchart.set(target_flowchart))
        .child(
            mono(label)
                .number_of_lines(1)
                .text_size(px(10.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || {
                    if is_active() {
                        fg_primary()
                    } else {
                        fg_muted()
                    }
                }),
        )
}

fn seq_mermaid_source_view(seq_builder: SeqBuilder, controls: Arc<Controls>) -> impl IntoElement {
    view()
        .relative()
        .w_full()
        .h(move || {
            px(seq_preview_height(
                window().height.0,
                seq_builder.items.with(Vec::len),
            ))
        })
        .rounded(px(7.0))
        .border_1()
        .border_color(border_control())
        .bg(bg_code())
        .overflow_hidden()
        .child(
            editor()
                .value(move || SharedString::from(active_seq_mermaid(seq_builder)))
                .read_only(true)
                .language("plaintext")
                .line_numbers(true)
                .folding(false)
                .soft_wrap(true)
                .font_size(Points(12.0))
                .line_height(Points(18.0))
                .scroll_beyond_last_line(false)
                .size_full(),
        )
        .child(when(
            move || !active_seq_mermaid(seq_builder).is_empty(),
            move || {
                let controls = controls.clone();
                view()
                    .absolute()
                    .top(px(6.0))
                    .right(px(8.0))
                    .flex_row()
                    .items_center()
                    .gap_1p5()
                    .child(when(
                        move || {
                            seq_builder
                                .diagram
                                .with(|d| !d.flowchart_mermaid.is_empty())
                        },
                        move || {
                            view()
                                .h(px(24.0))
                                .px(px(2.0))
                                .rounded(px(5.0))
                                .border_1()
                                .border_color(border_control())
                                .bg(bg_control())
                                .flex_row()
                                .items_center()
                                .gap(px(2.0))
                                .child(seq_source_kind_chip(
                                    seq_builder.source_flowchart,
                                    false,
                                    "sequenceDiagram",
                                ))
                                .child(seq_source_kind_chip(
                                    seq_builder.source_flowchart,
                                    true,
                                    "flowchart TD",
                                ))
                        },
                    ))
                    .child(
                        view()
                            .h(px(24.0))
                            .px_2()
                            .rounded(px(5.0))
                            .border_1()
                            .border_color(border_control())
                            .bg(bg_card())
                            .hover(|style| style.bg(bg_control_hover()))
                            .active(|style| style.opacity(0.75))
                            .cursor(Cursor::Pointer)
                            .flex_row()
                            .items_center()
                            .on_press(move || {
                                set_clipboard(active_seq_mermaid(seq_builder));
                                set_status(&controls, StatusMsg::CopiedMermaid);
                            })
                            .child(
                                text(tr("复制", "Copy"))
                                    .number_of_lines(1)
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(fg_secondary()),
                            ),
                    )
            },
        ))
}

/// Checkbox rows for the sequence diagram. Virtualized: only the rows near the
/// viewport exist, so a long capture does not grow nsh's node pools.
fn seq_candidate_list(seq_builder: SeqBuilder, controls: Arc<Controls>) -> impl IntoElement {
    virtual_list(
        move || seq_builder.items.with(Vec::len),
        move |at| seq_candidate_row(seq_builder, controls.clone(), at),
    )
    .keys(move || {
        seq_builder
            .items
            .with(|items| items.iter().map(|item| item.id).collect::<Vec<_>>())
    })
    .estimated_item_size(px(32.0))
    .size_full()
}

fn seq_candidate_row(
    seq_builder: SeqBuilder,
    controls: Arc<Controls>,
    at: Position,
) -> impl IntoElement {
    let item = move || {
        seq_builder
            .items
            .with(|items| items.get(at.index()).cloned())
            .unwrap_or_default()
    };
    view()
        .w_full()
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .flex_row()
        .items_center()
        .bg(move || {
            if at.index() % 2 == 1 {
                bg_code()
            } else {
                bg_card()
            }
        })
        .border_t(move || if at.index() > 0 { px(1.0) } else { px(0.0) })
        .border_t_color(border_subtle())
        .hover(|style| style.bg(bg_control_hover()))
        .cursor(Cursor::Pointer)
        .on_press(move || {
            let index = at.index();
            seq_builder.items.update(|items| {
                if let Some(target) = items.get_mut(index) {
                    target.checked = !target.checked;
                }
            });
            refresh_seq_diagram(&controls, seq_builder);
        })
        .child(
            view()
                .w(px(14.0))
                .h(px(14.0))
                .flex_shrink_0()
                .rounded(px(4.0))
                .border_1()
                .border_color(move || {
                    if item().checked {
                        accent_blue()
                    } else {
                        border_control()
                    }
                })
                .bg(move || {
                    if item().checked {
                        accent_blue()
                    } else {
                        bg_card()
                    }
                })
                .items_center()
                .justify_center()
                .child(when(
                    move || item().checked,
                    || {
                        canvas(|| {
                            Painter::new()
                                .line_width(1.8)
                                .stroke_color(white())
                                .stroke_svg("M2.2 5.2 L4.4 7.4 L8.0 3.0")
                        })
                        .w(px(10.0))
                        .h(px(10.0))
                    },
                )),
        )
        .child(
            mono(move || format!("#{}", item().id))
                .tabular_nums()
                .min_w(px(36.0))
                .flex_shrink_0()
                .text_size(px(11.5))
                .text_color(fg_muted()),
        )
        .child(
            view()
                .min_w(px(44.0))
                .px(px(5.0))
                .py_0p5()
                .rounded(px(4.0))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .bg(move || method_bg(&item().method))
                .child(
                    mono(move || item().method)
                        .text_size(px(10.5))
                        .font_weight(FontWeight::BOLD)
                        .text_color(move || method_fg(&item().method)),
                ),
        )
        .child(
            mono(move || seq_candidate_target(&item(), seq_builder.include_query.get()))
                .flex_1()
                .min_w(px(0.0))
                .number_of_lines(1)
                .text_overflow(EllipsizeMode::Middle)
                .text_size(px(12.0))
                .text_color(fg_primary()),
        )
        .child(when(
            move || !item().comment.is_empty(),
            move || {
                view()
                    .px_1p5()
                    .py(px(1.0))
                    .rounded(px(4.0))
                    .max_w(px(140.0))
                    .flex_shrink_0()
                    .bg(accent_blue_soft())
                    .child(
                        text(move || format!("# {}", item().comment))
                            .number_of_lines(1)
                            .text_overflow(EllipsizeMode::Tail)
                            .text_size(px(11.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(accent_blue_text()),
                    )
            },
        ))
        .child(
            mono(move || {
                let i = item();
                list_status(i.status, i.failed)
            })
            .tabular_nums()
            .min_w(px(32.0))
            .text_right()
            .flex_shrink_0()
            .text_size(px(11.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || {
                let i = item();
                http_status_fg(i.status, i.failed)
            }),
        )
        .child(
            mono(move || item().elapsed)
                .tabular_nums()
                .min_w(px(52.0))
                .text_right()
                .flex_shrink_0()
                .text_size(px(11.5))
                .text_color(fg_muted()),
        )
}

fn sequence_diagram_panel(
    seq_builder: SeqBuilder,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    compose: Compose,
) -> impl IntoElement {
    let select_all_controls = controls.clone();
    let clear_controls = controls.clone();
    let invert_controls = controls.clone();
    let res_controls = controls.clone();
    let query_controls = controls.clone();
    let row_controls = controls.clone();
    let copy_mermaid_controls = controls.clone();
    let copy_md_controls = controls.clone();
    let source_controls = controls;
    let has_checked = move || {
        seq_builder
            .items
            .with(|items| items.iter().any(|item| item.checked))
    };
    view()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .overflow_hidden()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .px_3p5()
                .py_2()
                .border_b_1()
                .border_b_color(border_subtle())
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(section_title(tr("生成时序图", "Sequence diagram")))
                        .child(
                            view()
                                .px_2()
                                .py_0p5()
                                .rounded(px(5.0))
                                .bg(accent_blue_soft())
                                .child(
                                    mono({
                                        let langs = ui_langs();
                                        move || {
                                            let (checked, total) =
                                                seq_builder.items.with(|items| {
                                                    let c = items
                                                        .iter()
                                                        .filter(|item| item.checked)
                                                        .count();
                                                    (c, items.len())
                                                });
                                            match ui_lang(langs) {
                                                Lang::Zh => format!("已选 {checked} / {total}"),
                                                Lang::En => format!("{checked} / {total} selected"),
                                            }
                                        }
                                    })
                                    .tabular_nums()
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(accent_blue_text()),
                                ),
                        ),
                )
                .child(detail_button(tr("关闭", "Close"), move || {
                    close_sequence_diagram(seq_builder);
                })),
        )
        .child(
            view()
                .p_3p5()
                .gap_3()
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .child(detail_button(tr("全选", "Select all"), move || {
                                    seq_builder.items.update(|items| {
                                        for item in items {
                                            item.checked = true;
                                        }
                                    });
                                    refresh_seq_diagram(&select_all_controls, seq_builder);
                                }))
                                .child(detail_button(tr("全不选", "Clear"), move || {
                                    seq_builder.items.update(|items| {
                                        for item in items {
                                            item.checked = false;
                                        }
                                    });
                                    refresh_seq_diagram(&clear_controls, seq_builder);
                                }))
                                .child(detail_button(tr("反选", "Invert"), move || {
                                    seq_builder.items.update(|items| {
                                        for item in items {
                                            item.checked = !item.checked;
                                        }
                                    });
                                    refresh_seq_diagram(&invert_controls, seq_builder);
                                })),
                        )
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .child(seq_option_toggle(
                                    seq_builder.include_response,
                                    "含响应",
                                    "Responses",
                                    move || {
                                        refresh_seq_diagram(&res_controls, seq_builder);
                                    },
                                ))
                                .child(seq_option_toggle(
                                    seq_builder.include_query,
                                    "含查询参数",
                                    "Query string",
                                    move || {
                                        refresh_seq_diagram(&query_controls, seq_builder);
                                    },
                                )),
                        ),
                )
                .child(
                    view()
                        .w_full()
                        .h(move || {
                            let max_rows = seq_candidate_max_rows(window().height.0);
                            let rows = seq_builder.items.with(Vec::len).clamp(1, max_rows) as f32;
                            px(rows * 32.0)
                        })
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(border_subtle())
                        .bg(bg_code())
                        .overflow_hidden()
                        .child(seq_candidate_list(seq_builder, row_controls)),
                )
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .child(
                                    view()
                                        .h(px(26.0))
                                        .px_3()
                                        .rounded(px(6.0))
                                        .bg(move || {
                                            if has_checked() {
                                                accent_blue()
                                            } else {
                                                bg_control()
                                            }
                                        })
                                        .hover(|style| {
                                            style.bg(move || {
                                                if has_checked() {
                                                    accent_blue_hover()
                                                } else {
                                                    bg_control()
                                                }
                                            })
                                        })
                                        .active(|style| {
                                            style.opacity(move || {
                                                if has_checked() { 0.85_f32 } else { 1.0_f32 }
                                            })
                                        })
                                        .cursor(Cursor::Pointer)
                                        .disabled(move || !has_checked())
                                        .disabled_style(|style| style.cursor(Cursor::Default))
                                        .flex_row()
                                        .items_center()
                                        .on_press(move || {
                                            let count = seq_builder.items.with(|items| {
                                                items.iter().filter(|i| i.checked).count()
                                            });
                                            if count == 0 {
                                                return;
                                            }
                                            let _ = refresh_seq_diagram(
                                                &copy_mermaid_controls,
                                                seq_builder,
                                            );
                                            set_clipboard(active_seq_mermaid(seq_builder));
                                            set_status(
                                                &copy_mermaid_controls,
                                                StatusMsg::SequenceGenerated { count },
                                            );
                                        })
                                        .child(
                                            text(tr("复制 Mermaid", "Copy Mermaid"))
                                                .number_of_lines(1)
                                                .text_size(px(12.0))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(move || {
                                                    if has_checked() {
                                                        AdaptiveColor::rgb(0xffffff, 0xffffff)
                                                    } else {
                                                        fg_muted()
                                                    }
                                                }),
                                        ),
                                )
                                .child(seq_action_button(
                                    tr("复制 Markdown", "Copy Markdown"),
                                    has_checked,
                                    move || {
                                        let count = seq_builder.items.with(|items| {
                                            items.iter().filter(|i| i.checked).count()
                                        });
                                        if count == 0 {
                                            return;
                                        }
                                        let _ = refresh_seq_diagram(&copy_md_controls, seq_builder);
                                        let diagram = active_seq_mermaid(seq_builder);
                                        set_clipboard(present::mermaid_markdown_block(&diagram));
                                        set_status(
                                            &copy_md_controls,
                                            StatusMsg::CopiedMermaidMarkdown,
                                        );
                                    },
                                )),
                        )
                        .child(
                            view()
                                .h(px(26.0))
                                .px(px(2.0))
                                .rounded(px(6.5))
                                .border_1()
                                .border_color(border_subtle())
                                .bg(bg_control())
                                .flex_row()
                                .items_center()
                                .gap(px(2.0))
                                .child(seq_mode_tab(
                                    seq_builder.view_mode,
                                    seq_builder.source_flowchart,
                                    0,
                                    "瀑布图",
                                    "Waterfall",
                                ))
                                .child(seq_mode_tab(
                                    seq_builder.view_mode,
                                    seq_builder.source_flowchart,
                                    1,
                                    "时序图",
                                    "Sequence",
                                ))
                                .child(seq_mode_tab(
                                    seq_builder.view_mode,
                                    seq_builder.source_flowchart,
                                    2,
                                    "依赖图",
                                    "Dependency",
                                ))
                                .child(seq_mode_tab(
                                    seq_builder.view_mode,
                                    seq_builder.source_flowchart,
                                    3,
                                    "源码",
                                    "Mermaid",
                                )),
                        ),
                )
                .child(when(
                    move || {
                        seq_builder
                            .items
                            .with(|items| items.iter().filter(|i| i.checked).count())
                            > SEQ_PREVIEW_CAP
                    },
                    move || {
                        let langs = ui_langs();
                        view()
                            .w_full()
                            .px_3()
                            .py_1p5()
                            .rounded(px(6.0))
                            .bg(status_warn_bg())
                            .border_1()
                            .border_color(status_warn_fg())
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(glyph_px(13.0, || Glyph::Alert, || color(status_warn_fg())))
                            .child(
                                text(move || {
                                    let checked = seq_builder
                                        .items
                                        .with(|items| items.iter().filter(|i| i.checked).count());
                                    match ui_lang(langs) {
                                        Lang::Zh => format!(
                                            "仅预览最近 {SEQ_PREVIEW_CAP} 条；复制 Mermaid / Markdown 仍包含全部 {checked} 条"
                                        ),
                                        Lang::En => format!(
                                            "Previewing newest {SEQ_PREVIEW_CAP} flows; Copy Mermaid / Markdown still includes all {checked}"
                                        ),
                                    }
                                })
                                .text_size(px(12.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(status_warn_fg()),
                            )
                    },
                ))
                .child(when(
                    move || seq_builder.view_mode.get() == 0,
                    move || seq_waterfall_view(seq_builder, selected, compose),
                ))
                .child(when(
                    move || seq_builder.view_mode.get() == 1,
                    move || seq_swimlane_view(seq_builder, selected, compose),
                ))
                .child(when(
                    move || seq_builder.view_mode.get() == 2,
                    move || seq_dependency_view(seq_builder, selected, compose),
                ))
                .child(when(
                    move || seq_builder.view_mode.get() == 3,
                    move || seq_mermaid_source_view(seq_builder, source_controls.clone()),
                )),
        )
}

fn begin_compose(controls: &Controls, id: Option<u64>, compose: Compose) {
    if refuse_relay(controls, id) {
        return;
    }
    if id.is_none() {
        compose.response.set(false);
        compose.method.set("GET".to_string());
        compose.url.set(String::new());
        compose.headers.set(String::new());
        compose.body.set(String::new());
        compose.status.set(String::new());
        compose.composing.set(true);
        return;
    }
    let Some(flow) = selected_flow(controls, id) else {
        set_status(controls, StatusMsg::NeedSelection);
        return;
    };
    controls.hub.touch_break();
    let response = controls
        .hub
        .paused()
        .is_some_and(|(paused_id, side)| id == Some(paused_id) && side == BreakSide::Response);
    compose.response.set(response);
    compose.method.set(flow.method);
    compose.url.set(flow.url);
    if response {
        compose
            .status
            .set(flow.status.map(|code| code.to_string()).unwrap_or_default());
        compose.headers.set(header_text(&flow.response_headers));
        compose
            .body
            .set(String::from_utf8_lossy(&flow.response_body).into_owned());
    } else {
        compose.status.set(String::new());
        compose.headers.set(header_text(&flow.request_headers));
        compose
            .body
            .set(String::from_utf8_lossy(&flow.request_body).into_owned());
    }
    compose.composing.set(true);
}

fn header_text(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn reset_compose(compose: Compose) {
    compose.composing.set(false);
    compose.response.set(false);
    compose.method.set(String::new());
    compose.url.set(String::new());
    compose.headers.set(String::new());
    compose.body.set(String::new());
    compose.status.set(String::new());
}

fn clear_shown(shown: Shown) {
    shown.phase.set(Phase::Empty);
    shown.method.set(String::new());
    shown.url.set(String::new());
    shown.protocol.set(String::new());
    shown.rules.set(String::new());
    shown.status.set(None);
    shown.error.set(String::new());
    shown.request_headers.set(Vec::new());
    shown.response_headers.set(Vec::new());
    shown.request_body.set(String::new());
    shown.response_body.set(String::new());
    shown.request_hex.set(Vec::new());
    shown.response_hex.set(Vec::new());
    shown.request_language.set(String::new());
    shown.response_language.set(String::new());
    shown.request_doc.set(None);
    shown.response_doc.set(None);
    shown.req_doc_preview.set(false);
    shown.res_doc_preview.set(false);
    shown.request_form.set(Vec::new());
    shown.request_note.set(String::new());
    shown.response_note.set(String::new());
    reset_body_offsets(shown);
    shown.request_window.set(TextWindow::default());
    shown.response_window.set(TextWindow::default());
    shown.metrics.set(String::new());
    shown.timing_bar.set(present::TimingBreakdown::default());
    shown.query.set(Vec::new());
    shown.preview.set(String::new());
    shown.preview_w.set(0.0);
    shown.preview_h.set(0.0);
    shown.image_caption.set(String::new());
    shown.messages.set(Vec::new());
    shown.diff.set(String::new());
    shown.diff_old.set(String::new());
    shown.diff_new.set(String::new());
    shown.pinned.set(false);
    shown.relay.set(String::new());
    shown.comment.set(String::new());
    shown.editing_comment.set(false);
    shown.repeating.set(false);
}

/// Only hides the panels. The panel's views and memos are still mounted until
/// the next frame, so clearing their data here would make them read signals
/// that are being torn down. [`release_sequence_data`] frees the data later.
fn close_sequence_diagram(seq_builder: SeqBuilder) {
    seq_builder.open.set(false);
    seq_builder.redact_open.set(false);
}

/// Frees the sequence-diagram data once both panels are closed. Runs from the
/// periodic tick, after the closed panels have been unmounted.
fn release_sequence_data(seq_builder: SeqBuilder) {
    if seq_builder.open.get_untracked() || seq_builder.redact_open.get_untracked() {
        return;
    }
    let has_data = seq_builder.items.with_untracked(|items| !items.is_empty())
        || seq_builder
            .diagram
            .with_untracked(|d| !d.steps.is_empty() || !d.bars.is_empty());
    if !has_data {
        return;
    }
    seq_builder.hover_bar.set(None);
    seq_builder.source_flowchart.set(false);
    seq_builder.items.set(Vec::new());
    seq_builder.output.set(String::new());
    seq_builder
        .diagram
        .set(present::SequenceDiagramData::default());
    seq_builder.redact_flow_id.set(None);
    crate::session::trim_memory();
}

fn clear_captured(
    controls: &Controls,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    compose: Compose,
    seq_builder: SeqBuilder,
    shown: Shown,
) {
    controls.hub.clear_session();
    selected.set(None);
    baseline.set(None);
    reset_compose(compose);
    close_sequence_diagram(seq_builder);
    clear_shown(shown);
    crate::session::trim_memory();
}

fn clear_unpinned_captured(
    controls: &Controls,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    compose: Compose,
    seq_builder: SeqBuilder,
    shown: Shown,
) {
    controls.hub.clear_unpinned();
    if let Some(id) = selected.get()
        && controls
            .hub
            .with_session(|session| session.get(id).is_none())
    {
        selected.set(None);
        reset_compose(compose);
        clear_shown(shown);
    }
    if let Some(id) = baseline.get()
        && controls
            .hub
            .with_session(|session| session.get(id).is_none())
    {
        baseline.set(None);
    }
    if seq_builder.open.get() {
        let remaining_empty = controls.hub.with_session(|session| {
            seq_builder
                .items
                .update(|items| items.retain(|item| session.get(item.id).is_some()));
            seq_builder.items.with(Vec::is_empty)
        });
        if remaining_empty {
            close_sequence_diagram(seq_builder);
        } else {
            refresh_seq_diagram(controls, seq_builder);
        }
    }
    crate::session::trim_memory();
}

fn save_flow_comment(controls: &Controls, id: Option<u64>, shown: Shown) {
    let Some(id) = id else {
        shown.editing_comment.set(false);
        return;
    };
    let clean = shown.comment.get().trim().to_string();
    shown.comment.set(clean.clone());
    shown.editing_comment.set(false);
    let _ = controls.hub.set_comment(id, &clean);
}

/// Next id in the filtered list. `delta` is `-1` for up and `1` for down.
/// An empty list stays unselected. With nothing selected, down takes the
/// first row and up the last. Either end clamps. An id that is not in the
/// list is treated as no selection.
fn neighbor(ids: &[u64], selected: Option<u64>, delta: i32) -> Option<u64> {
    if ids.is_empty() {
        return None;
    }
    let Some(current) = selected else {
        return Some(end_id(ids, delta));
    };
    let Some(index) = ids.iter().position(|id| *id == current) else {
        return Some(end_id(ids, delta));
    };
    let last = ids.len() as i64 - 1;
    let next = (index as i64 + i64::from(delta)).clamp(0, last) as usize;
    Some(ids[next])
}

fn end_id(ids: &[u64], delta: i32) -> u64 {
    if delta < 0 {
        ids[ids.len() - 1]
    } else {
        ids[0]
    }
}

fn step_selected(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    compose: Compose,
    delta: i32,
) {
    let next = rows.with(|rows| {
        let ids: Vec<u64> = rows.iter().map(|row| row.id).collect();
        neighbor(&ids, selected.get(), delta)
    });
    if selected.get() != next {
        reset_compose(compose);
        selected.set(next);
    }
}

fn replay_flow(controls: &Arc<Controls>, id: Option<u64>, edit: ReplayOverride) {
    let hub = controls.hub.clone();
    let Some(id) = id else {
        if edit.url.as_deref().is_some_and(|u| !u.trim().is_empty()) {
            let controls = Arc::clone(controls);
            hub.send_composed_later(edit, move |result| match result {
                Ok(new_id) => set_status(&controls, StatusMsg::Replayed(new_id)),
                Err(err) => set_status(&controls, StatusMsg::ReplayFailed(format!("{err:#}"))),
            });
        } else {
            set_status(controls, StatusMsg::NeedSelection);
        }
        return;
    };
    if refuse_relay(controls, Some(id)) {
        return;
    }
    let controls = Arc::clone(controls);
    hub.replay_later(id, edit, move |result| match result {
        Ok(new_id) => set_status(&controls, StatusMsg::Replayed(new_id)),
        Err(err) => set_status(&controls, StatusMsg::ReplayFailed(format!("{err:#}"))),
    });
}

fn repeat_flow_batch(controls: &Arc<Controls>, id: Option<u64>, shown: Shown) {
    let Some(id) = id else {
        shown.repeating.set(false);
        set_status(controls, StatusMsg::NeedSelection);
        return;
    };
    let count = present::parse_repeat_count(&shown.repeat_count.get());
    let interval_ms = present::parse_repeat_interval_ms(&shown.repeat_interval.get());
    let concurrency = present::parse_repeat_concurrency(&shown.repeat_concurrency.get());
    shown.repeat_count.set(count.to_string());
    shown.repeat_interval.set(interval_ms.to_string());
    shown.repeat_concurrency.set(concurrency.to_string());
    shown.repeating.set(false);
    if refuse_relay(controls, Some(id)) {
        return;
    }
    let hub = controls.hub.clone();
    let controls = Arc::clone(controls);
    hub.replay_batch_later(
        id,
        count,
        concurrency,
        interval_ms,
        ReplayOverride::default(),
        move |result| match result {
            Ok(ids) => set_status(
                &controls,
                StatusMsg::Repeated {
                    id,
                    count: ids.len() as u32,
                },
            ),
            Err(err) => set_status(&controls, StatusMsg::ReplayFailed(format!("{err:#}"))),
        },
    );
}

fn parse_header_text(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn export_har_file(controls: Arc<Controls>, filter: &str) {
    let redact = redact_options(&controls);
    let value = controls
        .hub
        .export_har_matching_with_redact(filter, &redact);
    let bytes = match serde_json::to_vec_pretty(&value) {
        Ok(bytes) => bytes,
        Err(err) => {
            set_status(&controls, StatusMsg::ExportFailed(err.to_string()));
            return;
        }
    };
    let secs = clock::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let dialog = save_dialog_with_default_dir()
        .file_name(format!("zniff-{secs}.har"))
        .filter(nsh_core::file_dialog::FileFilter::new("HAR", ["har"]));
    spawn_detached(async move {
        match pick_save_path(dialog).await {
            Ok(Some(path)) => {
                if let Err(err) = write_export_file(&path, &bytes) {
                    set_status(&controls, StatusMsg::ExportFailed(err.to_string()));
                    return;
                }
                let text = path.display().to_string();
                set_clipboard(text.clone());
                set_status(&controls, StatusMsg::Exported(text));
            }
            Ok(None) => set_status(&controls, StatusMsg::ExportCancelled),
            Err(err) => set_status(&controls, StatusMsg::ExportFailed(err)),
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn export_response_web(controls: Arc<Controls>, id: Option<u64>) {
    let Some(id) = id else {
        set_status(&controls, StatusMsg::NeedSelection);
        return;
    };
    let url = format!("/api/flows/{id}/body");
    spawn_detached(async move {
        if open_url(url).await {
            set_status(&controls, StatusMsg::Saved(format!("flow-{id}")));
        } else {
            set_status(
                &controls,
                StatusMsg::ExportFailed("open_url blocked".into()),
            );
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn export_har_file(controls: Arc<Controls>, filter: &str) {
    let redact_on = lock_bool(&controls.redact_export);
    let redact_strict = lock_bool(&controls.redact_strict);
    let redact_fields = lock_string(&controls.redact_fields);
    let url = format!(
        "/api/export.har?filter={}&redact={}&redact_strict={}&redact_fields={}",
        url_encode_query_component(filter.trim()),
        if redact_on { "1" } else { "0" },
        if redact_strict { "1" } else { "0" },
        url_encode_query_component(&redact_fields),
    );
    spawn_detached(async move {
        if open_url(url).await {
            set_status(&controls, StatusMsg::Exported("zniff.har".into()));
        } else {
            set_status(
                &controls,
                StatusMsg::ExportFailed("open_url blocked".into()),
            );
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn export_html_file(controls: Arc<Controls>, filter: &str, zh: bool) {
    let (matched, _) = controls.hub.matching_ids(filter);
    let flows = controls.hub.with_session(|session| {
        matched
            .into_iter()
            .filter_map(|(id, _)| session.hydrated(id))
            .collect::<Vec<_>>()
    });
    let redact = redact_options(&controls);
    let json = share::document_with_redact(&flows, clock::now(), filter, &redact);
    let html = share::page(&json, "", zh);
    let secs = clock::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let dialog = save_dialog_with_default_dir()
        .file_name(format!("zniff-{secs}.html"))
        .filter(nsh_core::file_dialog::FileFilter::new(
            "HTML",
            ["html", "htm"],
        ));
    spawn_detached(async move {
        match pick_save_path(dialog).await {
            Ok(Some(path)) => {
                if let Err(err) = write_export_file(&path, html.as_bytes()) {
                    set_status(&controls, StatusMsg::ExportFailed(err.to_string()));
                    return;
                }
                let text = path.display().to_string();
                set_clipboard(text.clone());
                set_status(&controls, StatusMsg::Exported(text));
            }
            Ok(None) => set_status(&controls, StatusMsg::ExportCancelled),
            Err(err) => set_status(&controls, StatusMsg::ExportFailed(err)),
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn export_html_file(controls: Arc<Controls>, filter: &str, zh: bool) {
    let redact_on = lock_bool(&controls.redact_export);
    let redact_strict = lock_bool(&controls.redact_strict);
    let redact_fields = lock_string(&controls.redact_fields);
    let url = format!(
        "/api/export.html?filter={}&redact={}&redact_strict={}&redact_fields={}&lang={}",
        url_encode_query_component(filter.trim()),
        if redact_on { "1" } else { "0" },
        if redact_strict { "1" } else { "0" },
        url_encode_query_component(&redact_fields),
        if zh { "zh" } else { "en" },
    );
    spawn_detached(async move {
        if open_url(url).await {
            set_status(&controls, StatusMsg::Exported("zniff.html".into()));
        } else {
            set_status(
                &controls,
                StatusMsg::ExportFailed("open_url blocked".into()),
            );
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn begin_share(controls: &Arc<Controls>, minutes: u64, filter: &str, share_id: Signal<String>) {
    if !lock_bool(&controls.running) {
        set_status(controls, StatusMsg::ShareNeedsCapture);
        return;
    }
    let (local_base, local_only) = share_base(controls);
    let relay_url = lock_string(&controls.relay_url);
    let relay_token = lock_string(&controls.relay_token);
    let route = match relay::share_route(&relay_url, &relay_token, &local_base, local_only) {
        Ok(route) => route,
        Err(relay::ShareRouteError::BadUrl) => {
            set_status(controls, StatusMsg::RelayBadUrl);
            return;
        }
        Err(relay::ShareRouteError::EmptyToken) => {
            set_status(controls, StatusMsg::RelayNeedToken);
            return;
        }
    };
    stop_relay_push(controls);
    let redact = redact_options(controls);
    let (id, _expires) = controls.hub.begin_share_with_redact(
        Duration::from_secs(minutes.saturating_mul(60)),
        filter,
        redact,
    );
    let (url, via_relay, local_only) = match route {
        relay::ShareRoute::Local { base, local_only } => {
            (format!("{base}/s/{id}"), false, local_only)
        }
        relay::ShareRoute::Relay(base) => {
            let url = base.share_url(&id);
            let status_controls = Arc::clone(controls);
            let shown = url.clone();
            let handle = relay::spawn_push(
                Arc::clone(&controls.hub),
                id.clone(),
                base,
                relay_token.trim().to_string(),
                move |event| match event {
                    relay::PushEvent::Failed(err) => {
                        set_status(&status_controls, StatusMsg::RelayPushFailed(err));
                    }
                    relay::PushEvent::Connected => {
                        set_status(&status_controls, StatusMsg::RelayConnected(shown.clone()));
                    }
                },
            );
            relay::remember_push(handle.clone());
            *controls
                .relay_push
                .lock()
                .unwrap_or_else(|err| err.into_inner()) = Some(handle);
            (url, true, false)
        }
    };
    share_id.set(id);
    set_clipboard(url.clone());
    set_status(
        controls,
        StatusMsg::Shared {
            url,
            minutes,
            local_only,
            relay: via_relay,
        },
    );
}

#[cfg(not(target_arch = "wasm32"))]
fn stop_relay_push(controls: &Controls) {
    if let Some(handle) = controls
        .relay_push
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
    {
        handle.stop();
    }
}

#[cfg(target_arch = "wasm32")]
fn begin_share(controls: &Arc<Controls>, minutes: u64, filter: &str, share_id: Signal<String>) {
    if !lock_bool(&controls.running) {
        set_status(controls, StatusMsg::ShareNeedsCapture);
        return;
    }
    let redact_opts = share::RedactOptions::new(
        lock_bool(&controls.redact_export),
        &lock_string(&controls.redact_fields),
    )
    .with_strict(lock_bool(&controls.redact_strict));
    let body = serde_json::json!({
        "op": "begin_share",
        "minutes": minutes,
        "filter": filter,
        "redact": redact_opts.enabled,
        "redact_strict": redact_opts.strict,
        "redact_fields": redact_opts.to_wire_fields(),
        "relay_url": lock_string(&controls.relay_url),
        "relay_token": lock_string(&controls.relay_token),
    })
    .to_string()
    .into_bytes();
    let controls = Arc::clone(controls);
    spawn(async move {
        let request =
            HttpRequest::post("/api/hub", body).header("content-type", "application/json");
        let Ok(response) = fetch(request).await else {
            set_status(&controls, StatusMsg::RelayPushFailed("没有连上".into()));
            return;
        };
        let value: serde_json::Value =
            serde_json::from_str(&response.text()).unwrap_or(serde_json::json!({}));
        if let Some(err) = value.get("error").and_then(serde_json::Value::as_str) {
            let message = if err.contains("口令") {
                StatusMsg::RelayNeedToken
            } else if err.contains("地址") {
                StatusMsg::RelayBadUrl
            } else if err.contains("开始") {
                StatusMsg::ShareNeedsCapture
            } else {
                StatusMsg::RelayPushFailed(err.to_string())
            };
            set_status(&controls, message);
            return;
        }
        let Some(url) = value.get("url").and_then(serde_json::Value::as_str) else {
            set_status(&controls, StatusMsg::RelayPushFailed("没有分享地址".into()));
            return;
        };
        let id = value
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        share_id.set(id);
        set_clipboard(url.to_string());
        let relay = value
            .get("url")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|url| !url.contains("127.0.0.1"));
        set_status(
            &controls,
            StatusMsg::Shared {
                url: url.to_string(),
                minutes,
                local_only: url.contains("127.0.0.1"),
                relay,
            },
        );
    });
}

#[cfg(target_arch = "wasm32")]
fn end_share(controls: &Controls, share_id: Signal<String>) {
    controls.hub.end_share();
    share_id.set(String::new());
    set_status(controls, StatusMsg::ShareStopped);
}

#[cfg(not(target_arch = "wasm32"))]
fn end_share(controls: &Controls, share_id: Signal<String>) {
    stop_relay_push(controls);
    controls.hub.end_share();
    share_id.set(String::new());
    set_status(controls, StatusMsg::ShareStopped);
}

/// Address other people can open. `local_only` when the listener is not on every interface.
fn share_base(controls: &Controls) -> (String, bool) {
    let port = lock_port(&controls.listen_port);
    if lock_bool(&controls.listen_all)
        && let Some(ip) = choose_lan(&lan_addrs(), routed_v4(), "").map(|addr| addr.ip)
    {
        return (format!("http://{ip}:{port}"), false);
    }
    (format!("http://127.0.0.1:{port}"), true)
}

fn profiles_for_export(
    saved: &[config::SavedProfile],
    name: &str,
    filter: &str,
    bypass: &str,
    rules: &str,
    upstream: &str,
    rules_enabled: bool,
    breaks_enabled: bool,
    show_tunnel_failures: bool,
) -> Vec<config::SavedProfile> {
    let effective_name = if !name.trim().is_empty() {
        name.trim().to_string()
    } else {
        matched_saved_profile(saved, filter, bypass, rules, upstream).unwrap_or_default()
    };
    let current = push_saved_profile(
        &[],
        &effective_name,
        filter,
        bypass,
        rules,
        upstream,
        rules_enabled,
        breaks_enabled,
        show_tunnel_failures,
    );
    if !current.is_empty() {
        return current;
    }
    if !effective_name.is_empty()
        && let Some(matched) = saved
            .iter()
            .find(|preset| preset.name.eq_ignore_ascii_case(&effective_name))
    {
        return vec![matched.clone()];
    }
    saved.to_vec()
}

#[cfg(test)]
fn profile_export_filename(
    preferred_name: &str,
    profiles: &[config::SavedProfile],
    secs: u64,
) -> String {
    let raw = if !preferred_name.trim().is_empty() {
        preferred_name.trim()
    } else {
        profiles
            .first()
            .map(|preset| preset.name.trim())
            .unwrap_or_default()
    };
    let sanitized: String = raw
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            {
                '-'
            } else {
                ch
            }
        })
        .collect();
    let sanitized = sanitized.trim_matches(|ch: char| ch.is_whitespace() || ch == '.' || ch == '-');
    if sanitized.is_empty() {
        format!("zniff-profiles-{secs}.toml")
    } else {
        format!("zniff-{sanitized}.toml")
    }
}

fn export_profiles_clipboard(controls: &Controls, profiles: &[config::SavedProfile]) {
    match config::export_profiles_toml(profiles) {
        Ok(toml_text) => {
            let count = profiles.len();
            set_clipboard(toml_text);
            set_status(controls, StatusMsg::ProfilesExported { count });
        }
        Err(err) => {
            set_status(controls, StatusMsg::ExportFailed(err));
        }
    }
}

fn parse_profiles_from_text_or_path(
    input: &str,
) -> std::result::Result<Vec<config::SavedProfile>, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("empty TOML".to_string());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let candidate_str = dropped_toml_path(trimmed).unwrap_or_else(|| trimmed.to_string());
        let candidate = std::path::Path::new(&candidate_str);
        if candidate
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
            && candidate.is_file()
        {
            let content = std::fs::read_to_string(candidate).map_err(|err| err.to_string())?;
            return config::parse_profiles_toml(&content);
        }
    }
    config::parse_profiles_toml(trimmed)
}

fn import_profiles_text(controls: &Controls, raw_text: &str, profiles: ProfileSignals) -> bool {
    match parse_profiles_from_text_or_path(raw_text) {
        Ok(incoming) => {
            apply_imported_profiles(controls, incoming, profiles);
            true
        }
        Err(err) => {
            set_status(controls, StatusMsg::ImportFailed(err));
            false
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn import_profiles_file(controls: &Controls, path_text: &str, profiles: ProfileSignals) {
    let content = match std::fs::read_to_string(path_text.trim()) {
        Ok(content) => content,
        Err(err) => {
            set_status(controls, StatusMsg::ImportFailed(err.to_string()));
            return;
        }
    };
    let incoming = match config::parse_profiles_toml(&content) {
        Ok(incoming) => incoming,
        Err(err) => {
            set_status(controls, StatusMsg::ImportFailed(err));
            return;
        }
    };
    apply_imported_profiles(controls, incoming, profiles);
}

fn apply_imported_profiles(
    controls: &Controls,
    incoming: Vec<config::SavedProfile>,
    profiles: ProfileSignals,
) {
    let count = incoming.len();
    record_profile_history(
        &profiles.name.get(),
        &profiles.filter.get(),
        &profiles.bypass.get(),
        &profiles.rules.get(),
        &profiles.upstream.get(),
        profiles.rules_on.get(),
        profiles.breaks_on.get(),
        profiles.show_tunnels.get(),
        profiles.history,
    );
    for item in incoming.iter().rev() {
        record_profile_history(
            &item.name,
            &item.filter,
            &item.bypass_hosts,
            &item.rules,
            &item.upstream_proxy,
            item.rules_enabled,
            item.breaks_enabled,
            item.show_tunnel_failures,
            profiles.history,
        );
    }
    let merged = config::merge_imported_profiles(&profiles.saved.get(), &incoming, SAVED_RULE_CAP);
    profiles.saved.set(merged.clone());
    let _ = save_config(|config| config.saved_profiles = merged);
    if let Some(first) = incoming.first() {
        profiles.name.set(first.name.clone());
        apply_saved_profile(
            first,
            profiles.filter,
            profiles.bypass,
            profiles.rules,
            profiles.upstream,
            profiles.rules_on,
            profiles.breaks_on,
            profiles.show_tunnels,
            controls,
        );
    }
    set_status(controls, StatusMsg::ProfilesImported { count });
}

#[cfg(not(target_arch = "wasm32"))]
fn import_dropped(
    controls: &Controls,
    event: &FileDropEvent,
    lang: Lang,
    profiles: ProfileSignals,
) {
    let mut found = false;
    for uri in &event.uris {
        if let Some(path) = dropped_har_path(uri) {
            found = true;
            import_har_file(controls, &path);
        } else if let Some(path) = dropped_toml_path(uri) {
            found = true;
            import_profiles_file(controls, &path, profiles);
        }
    }
    if !found {
        set_status(
            controls,
            StatusMsg::ImportFailed(
                t(
                    lang,
                    "放下的文件不是 HAR 或 TOML 配置",
                    "Dropped file is not a HAR or TOML setup",
                )
                .to_string(),
            ),
        );
    }
}

#[cfg(target_arch = "wasm32")]
fn import_dropped(
    controls: &Controls,
    _event: &FileDropEvent,
    lang: Lang,
    _profiles: ProfileSignals,
) {
    set_status(
        controls,
        StatusMsg::ImportFailed(
            t(
                lang,
                "Web 模式请打开「过滤 → 导入」粘贴 TOML 配置",
                "In Web mode, open Filter → Import to paste TOML setup",
            )
            .to_string(),
        ),
    );
}

/// `file:` URL of a `.har`, or nothing. Encoded names are decoded. A Windows
/// `file:///C:/...` drops the extra slash. Other schemes are ignored.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn dropped_har_path(uri: &str) -> Option<String> {
    dropped_ext_path(uri, "har")
}

/// `file:` URL of a `.toml` profile file, or nothing.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn dropped_toml_path(uri: &str) -> Option<String> {
    dropped_ext_path(uri, "toml")
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn dropped_ext_path(uri: &str, ext_name: &str) -> Option<String> {
    let rest = uri.strip_prefix("file:")?;
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let rest = rest
        .strip_prefix("localhost")
        .filter(|rest| rest.starts_with('/'))
        .unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let path = file_url_path(rest);
    let matches = std::path::Path::new(&path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(ext_name));
    matches.then_some(path)
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn file_url_path(path: &str) -> String {
    let decoded = percent_decode_path(path);
    let bytes = decoded.as_bytes();
    if bytes.len() >= 4
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
        && (bytes[3] == b'/' || bytes[3] == b'\\')
    {
        decoded[1..].to_string()
    } else {
        decoded
    }
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn percent_decode_path(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            out.push((hi << 4) | lo);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn import_har_file(controls: &Controls, path_text: &str) {
    let path = if path_text.trim().is_empty() {
        ca::data_dir().join("import.har")
    } else {
        std::path::PathBuf::from(path_text.trim())
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) => {
            set_status(controls, StatusMsg::ImportFailed(err.to_string()));
            return;
        }
    };
    let value = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(value) => value,
        Err(err) => {
            set_status(controls, StatusMsg::ImportFailed(err.to_string()));
            return;
        }
    };
    let count = controls.hub.import_har(&value);
    set_status(
        controls,
        StatusMsg::Imported {
            count,
            path: path.display().to_string(),
        },
    );
}

fn apply_saved_prefs(hub: &Hub) {
    let saved = saved_config();
    hub.set_bypass(&saved.bypass_hosts);
    hub.set_rules(&saved.rules);
    hub.set_upstream(&saved.upstream_proxy);
    hub.set_proxy_auth(&config::runtime_proxy_auth(&saved.proxy_auth));
    hub.set_rules_enabled(saved.rules_enabled);
    hub.set_breaks_enabled(saved.breaks_enabled);
    hub.set_show_tunnel_failures(saved.show_tunnel_failures);
    hub.set_relay_ingress(saved.relay_accept, &saved.relay_token);
}

fn refuse_relay(controls: &Controls, id: Option<u64>) -> bool {
    let Some(id) = id else {
        return false;
    };
    let locked = controls.hub.with_session(|session| {
        session
            .get(id)
            .is_some_and(|flow| flow.relay_source.is_some())
    });
    if locked {
        set_status(controls, StatusMsg::RelayViewOnly);
    }
    locked
}

fn load_listen_all() -> bool {
    saved_config().listen_all
}

fn store_listen_all(on: bool) -> std::io::Result<()> {
    save_config(|config| config.listen_all = on)
}

fn load_lang_pref() -> Option<Lang> {
    saved_config().language.as_deref().and_then(parse_pref)
}

fn store_lang_pref(lang: Option<Lang>) -> std::io::Result<()> {
    save_config(|config| config.language = config::language_name(lang))
}

fn publish_lan(
    listen: &str,
    previous: &str,
    addrs_out: Signal<Vec<LanAddr>>,
    choice: Signal<String>,
    url_out: Signal<String>,
    qr_out: Signal<String>,
) {
    let host_ok = listen
        .rsplit_once(':')
        .is_some_and(|(host, _)| host == "0.0.0.0");
    if !host_ok {
        addrs_out.set(Vec::new());
        url_out.set(String::new());
        qr_out.set(String::new());
        return;
    }
    let addrs = lan_addrs();
    let chosen = choose_lan(&addrs, routed_v4(), previous);
    let ip = chosen.as_ref().map(|addr| addr.ip);
    addrs_out.set(addrs);
    choice.set(ip.map(|ip| ip.to_string()).unwrap_or_default());
    let url = cert_url(listen, ip);
    url_out.set(url.clone());
    qr_out.set(match ip {
        Some(ip) if !url.is_empty() => write_ca_qr(&url, ip).unwrap_or_default(),
        _ => String::new(),
    });
}

fn cert_url(listen: &str, ip: Option<Ipv4Addr>) -> String {
    let Some((host, port)) = listen.rsplit_once(':') else {
        return String::new();
    };
    if host != "0.0.0.0" {
        return String::new();
    }
    let Some(ip) = ip else {
        return String::new();
    };
    let Ok(port) = port.parse::<u16>() else {
        return String::new();
    };
    if port == 0 {
        return String::new();
    }
    format!("http://{ip}:{port}/zniff")
}

/// Keeps `previous` when that address is still up. Otherwise the public
/// route when it is a normal LAN address, then a physical interface ahead
/// of a tunnel. `198.18.0.0/15` is the usual TUN fake-ip range.
fn choose_lan(addrs: &[LanAddr], routed: Option<Ipv4Addr>, previous: &str) -> Option<LanAddr> {
    if let Some(found) = addrs.iter().find(|addr| addr.ip.to_string() == previous) {
        return Some(found.clone());
    }
    if let Some(ip) = routed
        && let Some(found) = addrs
            .iter()
            .find(|addr| addr.ip == ip && lan_rank(addr) < 9)
    {
        return Some(found.clone());
    }
    if let Some(found) = addrs
        .iter()
        .filter(|addr| lan_rank(addr) < 9)
        .min_by(|left, right| {
            lan_rank(left)
                .cmp(&lan_rank(right))
                .then(left.name.cmp(&right.name))
                .then(left.ip.octets().cmp(&right.ip.octets()))
        })
    {
        return Some(found.clone());
    }
    addrs.first().cloned().or_else(|| {
        routed
            .filter(|ip| usable_v4(*ip) && !fake_ip(*ip))
            .map(|ip| LanAddr {
                name: String::new(),
                ip,
            })
    })
}

fn lan_rank(addr: &LanAddr) -> u8 {
    if tunnel_name(&addr.name) || fake_ip(addr.ip) {
        return 9;
    }
    let name = addr.name.to_ascii_lowercase();
    if name.starts_with("en")
        || name.starts_with("eth")
        || name.starts_with("wlan")
        || name.starts_with("wl")
    {
        return 0;
    }
    if name.starts_with("bridge") {
        return 2;
    }
    5
}

fn tunnel_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("utun")
        || name.starts_with("tun")
        || name.starts_with("tap")
        || name.starts_with("tailscale")
        || name.starts_with("wg")
}

fn fake_ip(ip: Ipv4Addr) -> bool {
    let [first, second, _, _] = ip.octets();
    first == 198 && (second == 18 || second == 19)
}

fn usable_v4(ip: Ipv4Addr) -> bool {
    !ip.is_unspecified()
        && !ip.is_loopback()
        && !ip.is_link_local()
        && !ip.is_broadcast()
        && !ip.is_multicast()
}

/// IPv4 of the interface the OS would use to reach the public internet.
fn routed_v4() -> Option<Ipv4Addr> {
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
        socket.connect("1.1.1.1:80").ok()?;
        match socket.local_addr().ok()?.ip() {
            std::net::IpAddr::V4(ip) if usable_v4(ip) => Some(ip),
            _ => None,
        }
    }
}

fn lan_addrs() -> Vec<LanAddr> {
    let mut addrs = list_v4_addrs();
    addrs.sort_by(|left, right| {
        lan_rank(left)
            .cmp(&lan_rank(right))
            .then(left.name.cmp(&right.name))
            .then(left.ip.octets().cmp(&right.ip.octets()))
    });
    addrs
}

#[cfg(unix)]
fn list_v4_addrs() -> Vec<LanAddr> {
    let mut out = Vec::new();
    unsafe {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut head) != 0 {
            return out;
        }
        let mut cursor = head;
        while !cursor.is_null() {
            let ifa = &*cursor;
            cursor = ifa.ifa_next;
            if ifa.ifa_addr.is_null() {
                continue;
            }
            let flags = ifa.ifa_flags as u32;
            let up = libc::IFF_UP as u32;
            let loopback = libc::IFF_LOOPBACK as u32;
            if flags & up == 0 || flags & loopback != 0 {
                continue;
            }
            if i32::from((*ifa.ifa_addr).sa_family) != libc::AF_INET {
                continue;
            }
            let sa = &*(ifa.ifa_addr as *const libc::sockaddr_in);
            let ip = Ipv4Addr::from(u32::from_be(sa.sin_addr.s_addr));
            if !usable_v4(ip) || out.iter().any(|addr| addr.ip == ip) {
                continue;
            }
            let name = if ifa.ifa_name.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(ifa.ifa_name)
                    .to_string_lossy()
                    .into_owned()
            };
            out.push(LanAddr { name, ip });
        }
        libc::freeifaddrs(head);
    }
    out
}

#[cfg(windows)]
fn list_v4_addrs() -> Vec<LanAddr> {
    use windows_sys::Win32::Foundation::ERROR_BUFFER_OVERFLOW;
    use windows_sys::Win32::NetworkManagement::IpHelper::{GetAdaptersInfo, IP_ADAPTER_INFO};

    let mut out = Vec::new();
    unsafe {
        let mut size = 16 * 1024u32;
        let mut buf;
        loop {
            buf = vec![0u8; size as usize];
            let result = GetAdaptersInfo(buf.as_mut_ptr() as *mut IP_ADAPTER_INFO, &mut size);
            if result == 0 {
                break;
            }
            if result != ERROR_BUFFER_OVERFLOW || size > 1024 * 1024 {
                return out;
            }
        }
        let mut cursor = buf.as_ptr() as *const IP_ADAPTER_INFO;
        while !cursor.is_null() {
            let adapter = &*cursor;
            let name = c_array_to_string(&adapter.Description);
            let mut address = &adapter.IpAddressList
                as *const windows_sys::Win32::NetworkManagement::IpHelper::IP_ADDR_STRING;
            while !address.is_null() {
                let item = &*address;
                let text = c_array_to_string(&item.IpAddress.String);
                if let Ok(ip) = text.parse::<Ipv4Addr>()
                    && usable_v4(ip)
                    && !out.iter().any(|addr| addr.ip == ip)
                {
                    out.push(LanAddr {
                        name: name.clone(),
                        ip,
                    });
                }
                address = item.Next;
            }
            cursor = adapter.Next;
        }
    }
    out
}

#[cfg(windows)]
fn c_array_to_string(bytes: &[i8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let raw = bytes[..end]
        .iter()
        .map(|byte| *byte as u8)
        .collect::<Vec<_>>();
    String::from_utf8_lossy(&raw).into_owned()
}

#[cfg(not(any(unix, windows)))]
fn list_v4_addrs() -> Vec<LanAddr> {
    Vec::new()
}

fn write_ca_qr(text: &str, ip: Ipv4Addr) -> Option<String> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let modules = code.width();
    let scale = 4u32;
    let quiet = 2u32;
    let side = (modules as u32 + quiet * 2) * scale;
    let mut raw = Vec::with_capacity(((side + 1) * side) as usize);
    for y in 0..side {
        raw.push(0);
        for x in 0..side {
            let mx = x / scale;
            let my = y / scale;
            let inside = mx >= quiet
                && my >= quiet
                && (mx - quiet) < modules as u32
                && (my - quiet) < modules as u32;
            let dark = inside
                && code[((mx - quiet) as usize, (my - quiet) as usize)] == qrcode::Color::Dark;
            raw.push(if dark { 0 } else { 255 });
        }
    }
    let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut zlib, &raw).ok()?;
    let compressed = zlib.finish().ok()?;
    let png = png_gray(&compressed, side, side);
    #[cfg(target_arch = "wasm32")]
    {
        let _ = ip;
        Some(format!(
            "data:image/png;base64,{}",
            session::base64_encode(&png)
        ))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = ca::data_dir().join("preview");
        std::fs::create_dir_all(&dir).ok()?;
        let port = text.rsplit_once(':')?.1.split('/').next().unwrap_or("0");
        let path = dir.join(format!(
            "ca-qr-{}-{port}.png",
            ip.to_string().replace('.', "-")
        ));
        std::fs::write(&path, png).ok()?;
        Some(path.to_string_lossy().into_owned())
    }
}

fn png_gray(zlib_data: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut ihdr = Vec::new();
    ihdr.extend(width.to_be_bytes());
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 0, 0, 0, 0]);
    let mut out = vec![137, 80, 78, 71, 13, 10, 26, 10];
    out.extend(present::png_chunk(b"IHDR", &ihdr));
    out.extend(present::png_chunk(b"IDAT", zlib_data));
    out.extend(present::png_chunk(b"IEND", &[]));
    out
}

fn format_bytes(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        let mb = bytes as f64 / (1024.0 * 1024.0);
        if mb >= 99.95 {
            format!("{mb:.0} MB")
        } else {
            format!("{mb:.1} MB")
        }
    } else if bytes >= 1024 {
        let kb = bytes as f64 / 1024.0;
        if kb >= 99.95 {
            format!("{kb:.0} KB")
        } else {
            format!("{kb:.1} KB")
        }
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {

    use std::net::Ipv4Addr;

    use super::{LanAddr, cert_url, choose_lan, usable_v4};
    use crate::i18n::Lang;
    use crate::session::Protocol;

    #[test]
    fn protocol_tunnel_label_is_localized() {
        assert_eq!(Protocol::Tunnel.label_in(Lang::Zh), "隧道");
        assert_eq!(Protocol::Tunnel.label_in(Lang::En), "Tunnel");
        assert_eq!(Protocol::WebSocket.label_in(Lang::En), "WebSocket");
        assert_eq!(Protocol::Http2.label_in(Lang::En), "HTTP/2");
    }

    #[test]
    fn cert_url_uses_only_the_chosen_lan_address() {
        let ip = Ipv4Addr::new(192, 168, 1, 8);
        assert_eq!(cert_url("127.0.0.1:8877", Some(ip)), "");
        assert_eq!(cert_url("0.0.0.0:8877", None), "");
        assert_eq!(cert_url("0.0.0.0:0", Some(ip)), "");
        assert_eq!(
            cert_url("0.0.0.0:8877", Some(ip)),
            "http://192.168.1.8:8877/zniff"
        );
    }

    #[test]
    fn choose_lan_keeps_a_present_choice_then_the_routed_address() {
        let wifi = LanAddr {
            name: "en0".into(),
            ip: Ipv4Addr::new(192, 168, 1, 8),
        };
        let vpn = LanAddr {
            name: "utun3".into(),
            ip: Ipv4Addr::new(10, 8, 0, 2),
        };
        let bridge = LanAddr {
            name: "bridge100".into(),
            ip: Ipv4Addr::new(192, 168, 64, 1),
        };
        let addrs = [vpn.clone(), bridge, wifi.clone()];
        assert_eq!(
            choose_lan(&addrs, Some(vpn.ip), "192.168.1.8").map(|addr| addr.ip),
            Some(wifi.ip)
        );
        assert_eq!(
            choose_lan(&addrs, Some(vpn.ip), "10.8.0.2").map(|addr| addr.ip),
            Some(vpn.ip)
        );
        assert_eq!(
            choose_lan(&addrs, Some(vpn.ip), "").map(|addr| addr.ip),
            Some(wifi.ip)
        );
        assert_eq!(
            choose_lan(&addrs, Some(vpn.ip), "10.1.1.1").map(|addr| addr.ip),
            Some(wifi.ip)
        );
        assert_eq!(
            choose_lan(&[], Some(wifi.ip), "").map(|addr| addr.ip),
            Some(wifi.ip)
        );
        assert!(choose_lan(&[], Some(Ipv4Addr::LOCALHOST), "").is_none());
        assert!(choose_lan(&[], Some(Ipv4Addr::new(198, 18, 0, 1)), "").is_none());
    }

    #[test]
    fn usable_v4_skips_addresses_a_phone_cannot_open() {
        assert!(usable_v4(Ipv4Addr::new(192, 168, 1, 8)));
        assert!(usable_v4(Ipv4Addr::new(10, 8, 0, 2)));
        assert!(!usable_v4(Ipv4Addr::UNSPECIFIED));
        assert!(!usable_v4(Ipv4Addr::LOCALHOST));
        assert!(!usable_v4(Ipv4Addr::new(169, 254, 1, 1)));
        assert!(!usable_v4(Ipv4Addr::BROADCAST));
    }

    #[test]
    fn filter_chips_toggle_a_whole_token() {
        assert_eq!(
            super::toggle_filter_chip("", "type:json", super::JSON_ALIASES),
            "type:json"
        );
        assert_eq!(
            super::toggle_filter_chip("api.test type:json", "type:json", super::JSON_ALIASES),
            "api.test"
        );
        assert_eq!(
            super::toggle_filter_chip("CT:JSON pin:", "type:json", super::JSON_ALIASES),
            "pin:"
        );
        assert_eq!(
            super::toggle_filter_chip("fail example", "失败", super::FAIL_ALIASES),
            "example"
        );
        assert_eq!(
            super::toggle_filter_chip("example", "失败", super::FAIL_ALIASES),
            "example 失败"
        );
        assert_eq!(
            super::toggle_filter_chip("pin: slow:", "slow:", super::SLOW_ALIASES),
            "pin:"
        );
        assert!(!super::filter_has_token("type:jsonl", super::JSON_ALIASES));
        assert!(super::filter_has_token("pin:", super::PIN_ALIASES));
    }

    #[test]
    fn saved_filters_keep_the_newest_sixteen_without_duplicates() {
        let saved: Vec<String> = (0..16).map(|n| format!("f{n}")).collect();
        let next = super::push_saved_filter(&saved, "fresh");
        assert_eq!(next.len(), 16);
        assert_eq!(next[0], "fresh");
        assert!(!next.iter().any(|item| item == "f15"));
        assert_eq!(
            super::push_saved_filter(&["a".into()], "  "),
            vec!["a".to_string()]
        );
        assert_eq!(
            super::push_saved_filter(&["a".into(), "b".into()], "b"),
            vec!["b".to_string(), "a".to_string()]
        );
    }

    #[test]
    fn neighbor_steps_the_filtered_ids_and_clamps() {
        assert_eq!(super::neighbor(&[], None, 1), None);
        assert_eq!(super::neighbor(&[], Some(1), -1), None);
        assert_eq!(super::neighbor(&[2, 4, 9], None, 1), Some(2));
        assert_eq!(super::neighbor(&[2, 4, 9], None, -1), Some(9));
        assert_eq!(super::neighbor(&[2, 4, 9], Some(4), 1), Some(9));
        assert_eq!(super::neighbor(&[2, 4, 9], Some(4), -1), Some(2));
        assert_eq!(super::neighbor(&[2, 4, 9], Some(2), -1), Some(2));
        assert_eq!(super::neighbor(&[2, 4, 9], Some(9), 1), Some(9));
        assert_eq!(super::neighbor(&[2, 4, 9], Some(7), 1), Some(2));
        assert_eq!(super::neighbor(&[2, 4, 9], Some(7), -1), Some(9));
    }

    #[test]
    fn listed_lan_addresses_are_usable_ipv4() {
        for addr in super::lan_addrs() {
            assert!(usable_v4(addr.ip), "{} {}", addr.name, addr.ip);
        }
    }

    #[test]
    fn xdg_download_dir_reads_the_quoted_home_path() {
        let text = "\
# comment
XDG_DESKTOP_DIR=\"$HOME/Desktop\"
XDG_DOWNLOAD_DIR=\"$HOME/Downloads\"
XDG_DOCUMENTS_DIR=\"$HOME/Documents\"
";
        let path = super::xdg_download_dir("/home/ada", text).unwrap();
        assert_eq!(path, std::path::PathBuf::from("/home/ada/Downloads"));
        let absolute = super::xdg_download_dir("/home/ada", "XDG_DOWNLOAD_DIR=/srv/downloads\n");
        assert_eq!(absolute, Some(std::path::PathBuf::from("/srv/downloads")));
        assert!(super::xdg_download_dir("/home/ada", "XDG_DOWNLOAD_DIR=\"\"\n").is_none());
        assert!(
            super::xdg_download_dir("/home/ada", "XDG_DESKTOP_DIR=\"$HOME/Desktop\"\n").is_none()
        );
    }

    #[test]
    fn hex_display_numbers_each_row_and_copy_does_not() {
        let body: Vec<u8> = (0..20).collect();
        let shown = super::hex_display_rows(&body, 16);
        let mut lines = shown.lines();
        let first = lines.next().unwrap();
        let second = lines.next().unwrap();
        assert!(first.starts_with("0001  "), "{first}");
        assert!(second.starts_with("0002  "), "{second}");
        assert!(first.split_whitespace().skip(1).count() == 16);
        let wide = super::hex_display_rows(&body, 32);
        assert_eq!(wide.lines().count(), 1, "{wide}");
        assert!(wide.split_whitespace().skip(1).count() == 20);
        assert!(wide.contains("0f  10 "), "{wide}");
        assert_eq!(super::hex_row_bytes(400.0), 16);
        assert_eq!(super::hex_row_bytes(super::hex_line_width(32)), 32);
        assert_eq!(super::hex_row_bytes(super::hex_line_width(32) - 1.0), 16);
        assert_eq!(super::hex_cols_for_pane(0.0), 16);
        assert_eq!(super::hex_cols_for_pane(400.0), 16);
        let wide_pane = super::hex_line_width(32) + super::HEX_TEXT_PAD + super::HEX_AREA_CHROME;
        assert_eq!(super::hex_cols_for_pane(wide_pane), 32);
        assert_eq!(super::hex_cols_for_pane(wide_pane - 1.0), 16);
        let copied = super::hex_dump(&body);
        assert!(!copied.contains("0001"), "{copied}");
        assert!(copied.starts_with("00 01 02 "));
        assert!(
            super::response_hex_for_view(true, "/tmp/preview.png", body.clone()).is_empty(),
            "pretty image view hides hex bytes"
        );
        assert_eq!(
            super::response_hex_for_view(false, "/tmp/preview.png", body.clone()),
            body,
            "raw image view keeps hex bytes for 16/32-byte responsive reflow"
        );
    }

    #[test]
    fn body_box_follows_the_text_and_stops_at_ten_lines() {
        assert_eq!(super::body_box_height(1), 34.0);
        assert_eq!(super::body_box_height(super::body_line_count("a\nb")), 52.0);
        assert_eq!(super::body_box_height(10), 196.0);
        assert_eq!(super::body_box_height(40), 196.0);
        assert_eq!(super::editor_box_height("only"), 34.0);
        let long = "x\n".repeat(30);
        assert_eq!(super::editor_box_height(&long), 196.0);
    }

    #[test]
    fn body_text_cuts_on_a_char_boundary() {
        let mut raw = vec![b'a'; 256 * 1024 - 2];
        raw.extend_from_slice("不".as_bytes());
        raw.extend(std::iter::repeat_n(b'b', 40));
        let shown = super::body_text(&raw, false, None, None, super::Lang::Zh, 0);
        assert_eq!(shown.text.len(), 256 * 1024 - 2);
        assert!(shown.text.is_char_boundary(shown.text.len()));
        assert!(shown.text.chars().all(|ch| ch == 'a'));
        let rest = raw.len() - shown.text.len();
        assert!(shown.note.contains(&rest.to_string()), "{}", shown.note);
        assert!(shown.window.has_more());
        assert!(!shown.window.has_prev());

        let ascii = vec![b'x'; 256 * 1024 + 10];
        let ascii_shown = super::body_text(&ascii, false, None, None, super::Lang::En, 0);
        assert_eq!(ascii_shown.text.len(), 256 * 1024);
        assert!(
            ascii_shown.note.contains("10 more bytes"),
            "{}",
            ascii_shown.note
        );

        let next = super::body_text(
            &ascii,
            false,
            None,
            None,
            super::Lang::En,
            ascii_shown.window.end,
        );
        assert_eq!(next.text, "x".repeat(10));
        assert!(next.window.has_prev());
        assert!(!next.window.has_more());
        assert!(
            next.note.contains("Showing bytes 262145-262154 of 262154"),
            "{}",
            next.note
        );

        let continued =
            super::body_text(&raw, false, None, None, super::Lang::Zh, shown.window.end);
        assert!(continued.text.starts_with('不'), "{}", continued.text);
        assert!(continued.text.ends_with('b'));
        assert!(continued.window.has_prev());
        let inside = super::body_text(&raw, false, None, None, super::Lang::Zh, 256 * 1024);
        assert_eq!(inside.text, continued.text);

        let mut binary = vec![0xff; 5000];
        binary[0] = 0xfe;
        let binary_shown = super::body_text(&binary, false, None, None, super::Lang::Zh, 4096);
        assert_eq!(binary_shown.hex.len(), 4096);
        assert!(!binary_shown.window.has_more());
        assert!(!binary_shown.window.has_prev());
        assert!(binary_shown.note.contains("4096"), "{}", binary_shown.note);

        let payload = format!("{{\"k\":\"{}\"}}", "a".repeat(super::BODY_DISPLAY_CAP));
        let mut pretty_first =
            super::body_text(payload.as_bytes(), false, None, None, super::Lang::En, 0);
        super::apply_pretty(
            &mut pretty_first,
            payload.as_bytes(),
            Some("application/json"),
            super::Lang::En,
            0,
        );
        assert!(pretty_first.window.has_more(), "{}", pretty_first.note);
        assert!(pretty_first.text.len() <= super::BODY_DISPLAY_CAP);
        assert!(
            pretty_first.note.contains("more bytes"),
            "{}",
            pretty_first.note
        );
        let mut pretty_next = super::body_text(
            payload.as_bytes(),
            false,
            None,
            None,
            super::Lang::En,
            pretty_first.window.end,
        );
        super::apply_pretty(
            &mut pretty_next,
            payload.as_bytes(),
            Some("application/json"),
            super::Lang::En,
            pretty_first.window.end,
        );
        assert!(pretty_next.window.has_prev());
        assert_ne!(pretty_next.text, pretty_first.text);
        assert!(
            pretty_next.note.contains("Showing bytes"),
            "{}",
            pretty_next.note
        );

        let expired_shown = super::body_text(&[], false, Some(65536), None, super::Lang::Zh, 0);
        assert!(expired_shown.text.is_empty());
        assert!(
            expired_shown.note.contains("64.0 KB"),
            "{}",
            expired_shown.note
        );
    }

    #[test]
    fn flow_count_shows_total_or_filtered_over_total() {
        assert_eq!(super::flow_count_text(0, 0, false), "0");
        assert_eq!(super::flow_count_text(4, 4, false), "4");
        assert_eq!(super::flow_count_text(2, 9, true), "2 / 9");
    }

    #[test]
    fn list_columns_round_trip_and_the_pane_clamps() {
        let mut config = crate::config::Config::default();
        assert!(
            config.show_id
                && config.show_method
                && config.show_status
                && config.show_path
                && config.show_gap
                && config.show_elapsed
                && config.show_size
        );
        assert!(!config.show_protocol && !config.show_host && !config.show_client_ip);
        let columns = super::ListColumns::from_config(&config)
            .toggle(super::ListColumn::Protocol)
            .toggle(super::ListColumn::ClientIp)
            .toggle(super::ListColumn::Size)
            .with_full_url(true)
            .with_ellipsize(crate::config::ListEllipsize::Tail);
        columns.write(&mut config);
        assert!(config.show_protocol);
        assert!(config.show_client_ip);
        assert!(!config.show_size);
        assert!(config.show_path);
        assert!(config.show_full_url);
        assert_eq!(config.ellipsize, crate::config::ListEllipsize::Tail);
        assert_eq!(super::ListColumns::from_config(&config), columns);
        assert_eq!(columns.ellipsize_mode(), super::EllipsizeMode::Tail);
        let row = super::RowLine {
            id: 1,
            method: "GET".to_string(),
            status: Some(200),
            time: "0 ms".to_string(),
            gap_nanos: 0,
            clock: "12:00:00.000".to_string(),
            elapsed: "10 ms".to_string(),
            elapsed_nanos: Some(10_000_000),
            elapsed_slow: false,
            host: "api.example.com".to_string(),
            client_ip: "127.0.0.1".to_string(),
            path: "/v1/users".to_string(),
            url: "https://api.example.com/v1/users".to_string(),
            comment: String::new(),
            protocol: "HTTP/2".to_string(),
            size: "128 B".to_string(),
            size_bytes: 128,
            failed: false,
            pinned: false,
            relay_short: String::new(),
        };
        assert_eq!(super::row_target_text(&row, false), "/v1/users");
        assert_eq!(
            super::row_target_text(&row, true),
            "https://api.example.com/v1/users"
        );
        let empty_url_row = super::RowLine {
            url: String::new(),
            ..row
        };
        assert_eq!(super::row_target_text(&empty_url_row, true), "/v1/users");
        assert_eq!(
            super::clamp_list_px(10.0),
            crate::config::LIST_WIDTH_MIN as f32
        );
        assert_eq!(
            super::clamp_list_px(5_000.0),
            crate::config::LIST_WIDTH_MAX as f32
        );
        assert_eq!(super::clamp_list_px(640.6), 641.0);
        assert_eq!(
            super::clamp_list_px(-20.0),
            crate::config::LIST_WIDTH_MIN as f32
        );
        assert_eq!(super::clamp_list_px_for_window(900.0, 1000.0), 660.0);
        assert_eq!(super::clamp_list_px_for_window(900.0, 0.0), 900.0);
        assert_eq!(super::clamp_list_px_for_window(900.0, 500.0), 280.0);
        assert_eq!(super::adaptive_filter_panel_width(0.0, 560.0), 760.0);
        assert_eq!(super::adaptive_filter_panel_width(600.0, 560.0), 580.0);
        assert_eq!(super::adaptive_filter_panel_width(900.0, 560.0), 680.0);
        assert_eq!(super::adaptive_filter_panel_width(1200.0, 560.0), 792.0);
        assert_eq!(super::adaptive_filter_panel_width(1448.0, 560.0), 1040.0);
        assert_eq!(super::adaptive_filter_panel_width(1448.0, 644.0), 981.0);
        assert_eq!(super::adaptive_rules_editor_height(0.0), 105.0);
        assert_eq!(super::adaptive_rules_editor_height(520.0), 84.0);
        assert_eq!(super::adaptive_rules_editor_height(760.0), 84.0);
        assert_eq!(super::adaptive_rules_editor_height(1018.0), 264.0);
        assert_eq!(super::adaptive_rules_editor_height_for(760.0, true), 56.0);
        assert_eq!(super::adaptive_rules_editor_height_for(1018.0, true), 82.0);
        for window_h in [520.0, 640.0, 760.0, 900.0, 1018.0, 1440.0] {
            for import_open in [false, true] {
                let panel_max = super::adaptive_filter_panel_max_height(window_h);
                let body_max = super::adaptive_filter_body_max_height(window_h, import_open);
                let import_h = if import_open {
                    super::FILTER_PANEL_IMPORT_EXTRA_HEIGHT
                } else {
                    0.0
                };
                // Top toolbar (42) + popover offset (6) + panel_max + bottom status bar (26) <= window_h
                assert!(
                    48.0 + panel_max + 26.0 <= window_h,
                    "filter panel overflows window_h={window_h}: panel_max={panel_max}"
                );
                assert!(
                    body_max + super::FILTER_PANEL_DOCK_HEIGHT + import_h <= panel_max,
                    "filter body+dock exceeds panel_max at window_h={window_h}, import_open={import_open}"
                );
            }
        }
        assert_eq!(
            super::StatusMsg::ProfilesExported { count: 2 }.toast_kind(),
            Some(false)
        );
        assert_eq!(
            super::StatusMsg::ExportFailed("err".into()).toast_kind(),
            Some(true)
        );
        assert_eq!(
            super::StatusMsg::Listening {
                port: 8877,
                via_system_proxy: false,
            }
            .toast_kind(),
            None
        );
    }

    #[test]
    fn flow_columns_pin_method_and_path_and_keep_a_dragged_width() {
        let shown = super::ListColumns {
            id: false,
            method: true,
            path: true,
            status: true,
            gap: true,
            elapsed: true,
            size: false,
            protocol: false,
            host: false,
            client_ip: false,
            full_url: false,
            ellipsize: crate::config::ListEllipsize::Middle,
            order: super::ListColumn::ORDERED,
            order_rev: 0,
        };
        let mut widths = std::collections::HashMap::new();
        widths.insert("path".to_string(), 400);
        let cols = super::flow_table_columns(shown, &widths);
        let keys: Vec<&str> = cols.iter().map(|column| column.key.as_ref()).collect();
        assert_eq!(keys, ["method", "path", "status", "gap", "elapsed"]);
        assert_eq!(cols[0].fixed, super::ColumnFixed::Left);
        assert_eq!(cols[1].fixed, super::ColumnFixed::Left);
        assert!(
            cols[2..]
                .iter()
                .all(|column| column.fixed == super::ColumnFixed::None)
        );
        assert_eq!(cols[0].width.unwrap().0, 76.0);
        assert_eq!(cols[1].width.unwrap().0, 400.0);
        assert_eq!(cols[1].min_width.unwrap().0, 80.0);
        assert!(
            cols.iter()
                .all(|column| { column.movable == Some(true) && column.resizable == Some(true) })
        );
        let hidden_path = super::ListColumns {
            path: false,
            ..shown
        };
        let cols = super::flow_table_columns(hidden_path, &widths);
        assert_eq!(cols[0].key.as_ref(), "method");
        assert!(cols.iter().all(|column| column.key.as_ref() != "path"));
        assert_eq!(
            cols.iter()
                .filter(|column| column.fixed == super::ColumnFixed::Left)
                .count(),
            1
        );

        let default_shown = super::ListColumns::from_config(&crate::config::Config::default());
        let default_cols =
            super::flow_table_columns(default_shown, &std::collections::HashMap::new());
        let default_keys: Vec<&str> = default_cols.iter().map(|c| c.key.as_ref()).collect();
        assert_eq!(
            default_keys,
            ["id", "method", "path", "status", "gap", "elapsed", "size"]
        );
        assert!(
            default_cols[..3]
                .iter()
                .all(|column| column.fixed == super::ColumnFixed::Left)
        );
        let default_sum: f32 = default_cols.iter().map(|c| c.width.unwrap().0).sum();
        assert_eq!(default_sum, 568.0);

        let all_shown = super::ListColumns {
            id: true,
            method: true,
            path: true,
            status: true,
            gap: true,
            elapsed: true,
            size: true,
            protocol: true,
            host: true,
            client_ip: true,
            full_url: false,
            ellipsize: crate::config::ListEllipsize::Middle,
            order: super::ListColumn::ORDERED,
            order_rev: 0,
        };
        let all_cols = super::flow_table_columns(all_shown, &widths);
        let toggle_keys: Vec<&str> = super::ListColumn::ORDERED.iter().map(|c| c.key()).collect();
        let table_keys: Vec<&str> = all_cols.iter().map(|column| column.key.as_ref()).collect();
        assert_eq!(
            toggle_keys,
            [
                "id",
                "method",
                "path",
                "status",
                "gap",
                "elapsed",
                "size",
                "protocol",
                "host",
                "client_ip"
            ]
        );
        assert_eq!(toggle_keys, table_keys);

        // Reordering via up/down buttons respects fixed (0..3) and scrolling (3..10) zones.
        assert!(!default_shown.has_custom_order());
        assert!(!default_shown.can_move(super::ListColumn::Id, -1));
        assert!(default_shown.can_move(super::ListColumn::Id, 1));
        assert!(!default_shown.can_move(super::ListColumn::Path, 1));
        assert!(!default_shown.can_move(super::ListColumn::Status, -1));
        assert!(!default_shown.can_move(super::ListColumn::ClientIp, 1));

        let moved = default_shown
            .move_by(super::ListColumn::Method, -1)
            .move_by(super::ListColumn::Elapsed, -1);
        assert!(moved.has_custom_order());
        assert_eq!(moved.first_visible(), Some(super::ListColumn::Method));
        let moved_cols = super::flow_table_columns(moved, &std::collections::HashMap::new());
        let moved_keys: Vec<&str> = moved_cols.iter().map(|c| c.key.as_ref()).collect();
        assert_eq!(
            moved_keys,
            ["method", "id", "path", "status", "elapsed", "gap", "size"]
        );

        // Dragging visible headers updates visible slots while preserving hidden slots.
        let dragged = moved.apply_visible_order(&[
            "path".into(),
            "method".into(),
            "id".into(),
            "size".into(),
            "status".into(),
            "elapsed".into(),
            "gap".into(),
        ]);
        assert_eq!(dragged.first_visible(), Some(super::ListColumn::Path));
        assert_eq!(dragged.last_visible(), Some(super::ListColumn::Gap));
        assert_eq!(
            super::column_pad_right(Some(super::ListColumn::Size), true, default_shown).0,
            14.0
        );
        assert_eq!(
            super::column_pad_right(Some(super::ListColumn::Size), true, dragged).0,
            6.0
        );
        assert_eq!(
            super::column_pad_right(Some(super::ListColumn::Gap), true, dragged).0,
            14.0
        );
        assert_eq!(
            super::column_pad_right(Some(super::ListColumn::ClientIp), false, all_shown).0,
            12.0
        );
        assert_eq!(
            super::column_pad_right(Some(super::ListColumn::Status), false, all_shown).0,
            4.0
        );
        let dragged_cols = super::flow_table_columns(dragged, &std::collections::HashMap::new());
        let dragged_keys: Vec<&str> = dragged_cols.iter().map(|c| c.key.as_ref()).collect();
        assert_eq!(
            dragged_keys,
            ["path", "method", "id", "size", "status", "elapsed", "gap"]
        );
        let mut cfg = crate::config::Config::default();
        dragged.write(&mut cfg);
        assert_eq!(super::ListColumns::from_config(&cfg), dragged);
        assert_eq!(dragged.reset_order(), default_shown);
    }

    #[test]
    fn failed_rows_show_err_and_the_short_reason() {
        assert_eq!(super::list_status(Some(304), false), "304");
        assert_eq!(super::list_status(None, false), "—");
        assert_eq!(super::list_status(None, true), "ERR");
        assert_eq!(super::list_status(Some(502), true), "502");
        assert_eq!(
            super::list_path("/index", None, false, super::Lang::Zh),
            "/index"
        );
        assert_eq!(
            super::list_path("/get?env=staging&ts=1", None, false, super::Lang::Zh),
            "/get"
        );
        assert_eq!(
            super::list_url(
                "https://httpbin.org/get?env=staging&ts=1",
                "/get?env=staging&ts=1",
                None,
                false,
                super::Lang::Zh,
            ),
            "https://httpbin.org/get?env=staging&ts=1"
        );
        assert_eq!(
            super::list_url("", "/get?env=staging&ts=1", None, false, super::Lang::Zh),
            "/get?env=staging&ts=1"
        );
        assert_eq!(
            super::list_path("/", Some("证书被拒绝"), true, super::Lang::Zh),
            "证书被拒绝"
        );
        assert_eq!(
            super::list_path(":443", Some("证书被拒绝"), true, super::Lang::En),
            "Certificate rejected"
        );
        assert_eq!(super::list_path("/", None, true, super::Lang::En), "");
    }

    #[test]
    fn dropped_files_keep_only_har_urls() {
        assert_eq!(
            super::dropped_har_path("file:///Users/tree/a.har").as_deref(),
            Some("/Users/tree/a.har")
        );
        assert_eq!(
            super::dropped_har_path("file://localhost/tmp/my%20capture.har").as_deref(),
            Some("/tmp/my capture.har")
        );
        assert_eq!(
            super::dropped_har_path("file:///C:/captures/one.HAR").as_deref(),
            Some("C:/captures/one.HAR")
        );
        assert_eq!(
            super::dropped_har_path("file:///tmp/%E4%B8%AD.har").as_deref(),
            Some("/tmp/中.har")
        );
        assert!(super::dropped_har_path("file:///tmp/a.txt").is_none());
        assert!(super::dropped_har_path("/tmp/a.har").is_none());
        assert!(super::dropped_har_path("file://server/tmp/a.har").is_none());
        assert!(super::dropped_har_path("file:///tmp/notes.har.bak").is_none());
        assert_eq!(
            super::dropped_toml_path("file:///Users/tree/profiles.toml").as_deref(),
            Some("/Users/tree/profiles.toml")
        );
        assert_eq!(
            super::dropped_toml_path("file:///C:/Users/tree/My%20Setups/dev.TOML").as_deref(),
            Some("C:/Users/tree/My Setups/dev.TOML")
        );
        assert!(super::dropped_toml_path("file:///Users/tree/a.har").is_none());
    }

    #[test]
    fn profiles_export_and_parse_from_text_or_path() {
        let exported = super::profiles_for_export(
            &[],
            "Dev",
            "host:api.dev",
            "",
            "delay api.dev/v1 100ms",
            "",
            true,
            true,
            false,
        );
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0].name, "Dev");
        assert_eq!(
            super::profile_export_filename("Dev", &exported, 123),
            "zniff-Dev.toml"
        );
        assert_eq!(
            super::profile_export_filename("prod/api:v1", &exported, 123),
            "zniff-prod-api-v1.toml"
        );
        assert_eq!(
            super::profile_export_filename("", &[], 123),
            "zniff-profiles-123.toml"
        );
        let toml_text = crate::config::export_profiles_toml(&exported).unwrap();
        let parsed = super::parse_profiles_from_text_or_path(&toml_text).unwrap();
        assert_eq!(parsed, exported);
        assert!(super::parse_profiles_from_text_or_path("   ").is_err());

        // When `saved` already has a profile and the unnamed working setup matches it,
        // `profiles_for_export` updates that profile without creating a duplicate.
        let same_unnamed = super::profiles_for_export(
            &exported,
            "",
            "host:api.dev",
            "",
            "delay api.dev/v1 100ms",
            "",
            false,
            true,
            false,
        );
        assert_eq!(same_unnamed.len(), 1);
        assert_eq!(same_unnamed[0].name, "Dev");
        assert!(!same_unnamed[0].rules_enabled);

        // When `saved` has a profile and the user is editing a new or selected setup,
        // `profiles_for_export` exports only the currently selected/editing setup.
        let with_new_unnamed = super::profiles_for_export(
            &exported,
            "",
            "host:staging.dev",
            "",
            "",
            "",
            true,
            true,
            false,
        );
        assert_eq!(with_new_unnamed.len(), 1);
        assert_eq!(with_new_unnamed[0].name, "host:staging.dev");

        // When the working fields are empty, `profiles_for_export` falls back to `saved`.
        let fallback_all =
            super::profiles_for_export(&exported, "", "", "", "", "", true, true, false);
        assert_eq!(fallback_all.len(), 1);
        assert_eq!(fallback_all[0].name, "Dev");
    }

    #[test]
    fn structured_filter_fields_sync_host_path_and_exclude_or() {
        let f = super::set_host_or("GET", "api.example.com | *.cdn.com, sub.test");
        assert_eq!(f, "host:api.example.com|*.cdn.com|sub.test GET");
        assert_eq!(
            super::extract_host_or(&f),
            "api.example.com | *.cdn.com | sub.test"
        );
        let f = super::set_path_or(&f, "/v1 | /auth");
        assert_eq!(
            f,
            "host:api.example.com|*.cdn.com|sub.test GET path:/v1|/auth"
        );
        assert_eq!(super::extract_path_or(&f), "/v1 | /auth");
        let f = super::set_exclude_or(&f, "ads.test | /health");
        assert_eq!(super::extract_exclude_or(&f), "ads.test | /health");
    }

    #[test]
    fn named_filters_and_rules_store_and_deduplicate() {
        let saved = super::push_saved_filter_named(&[], "调试接口", "host:example.com type:json");
        assert_eq!(
            saved,
            vec!["调试接口\thost:example.com type:json".to_string()]
        );
        assert_eq!(
            super::split_saved_filter(&saved[0]),
            ("调试接口", "host:example.com type:json")
        );
        // Updating the same expression with a new name replaces the old entry.
        let saved =
            super::push_saved_filter_named(&saved, "预发接口", "host:example.com type:json");
        assert_eq!(
            saved,
            vec!["预发接口\thost:example.com type:json".to_string()]
        );

        let rules =
            super::push_saved_rule(&[], "改文案", "replace example.com/api req hello -> hi");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "改文案");
        // Unnamed rule auto-generates name from the first rule line.
        let rules = super::push_saved_rule(&rules, "", "break api.test/v1");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].name, "break api.test/v1");
    }

    #[test]
    fn save_stays_off_until_the_open_profile_changes() {
        let saved = [crate::config::SavedProfile {
            name: "test".into(),
            filter: "host:a.test".into(),
            bypass_hosts: String::new(),
            rules: "break a.test/api\n".into(),
            upstream_proxy: String::new(),
            rules_enabled: true,
            breaks_enabled: true,
            show_tunnel_failures: false,
        }];
        let ready = |name, filter, rules, rules_on, tunnels, host, from| {
            super::profile_save_ready(
                &saved, name, filter, "", rules, "", rules_on, true, tunnels, host, "", "", from,
                "", false, 0,
            )
        };
        assert!(!ready(
            "test",
            "host:a.test",
            "break a.test/api\n",
            true,
            false,
            "",
            ""
        ));
        assert!(!ready(
            "test",
            " host:a.test ",
            "break a.test/api",
            true,
            false,
            "",
            ""
        ));
        assert!(ready(
            "test",
            "host:b.test",
            "break a.test/api",
            true,
            false,
            "",
            ""
        ));
        assert!(ready(
            "test",
            "host:a.test",
            "break a.test/api",
            false,
            false,
            "",
            ""
        ));
        assert!(ready(
            "test",
            "host:a.test",
            "break a.test/api",
            true,
            true,
            "",
            ""
        ));
        assert!(ready(
            "Test",
            "host:a.test",
            "break a.test/api",
            true,
            false,
            "",
            ""
        ));
        assert!(ready(
            "other",
            "host:a.test",
            "break a.test/api",
            true,
            false,
            "",
            ""
        ));
        assert!(!ready("test", "", "", true, false, "", ""));
        assert!(ready("", "host:b.test", "", true, false, "", ""));
        assert!(ready(
            "test",
            "host:a.test",
            "break a.test/api",
            true,
            false,
            "a.test",
            "keep"
        ));
        assert_eq!(
            super::draft_rules("break a.test/api", "a.test", "", "keep", "", false, 0),
            "break a.test/api\nreplace a.test req keep -> \n"
        );
        assert_eq!(
            super::draft_rules("break a.test/api", "a.test", "", "with space", "", false, 0),
            "break a.test/api\nreplace a.test req \"with space\" -> \n"
        );
        assert!(!ready(
            "test",
            "host:a.test",
            "break a.test/api",
            true,
            false,
            "a.test",
            ""
        ));
    }

    #[test]
    fn parse_status_code_accepts_three_digit_codes_and_reason_phrases() {
        assert_eq!(super::parse_status_code("201"), Some(201));
        assert_eq!(super::parse_status_code(" 201 Created "), Some(201));
        assert_eq!(super::parse_status_code("99"), None);
        assert_eq!(super::parse_status_code("600"), None);
        assert_eq!(super::parse_status_code(""), None);
    }

    #[test]
    fn detached_owner_survives_child_owner_disposal() {
        use nsh_core::element::{Runtime, create_root};
        use nsh_core::prelude::{Owner, cleanup};
        use std::cell::Cell;
        use std::rc::Rc;

        let runtime = Runtime::default();
        let _entered = runtime.enter();
        let scoped_cancelled = Rc::new(Cell::new(false));
        let detached_cancelled = Rc::new(Cell::new(false));
        let scoped_flag = Rc::clone(&scoped_cancelled);
        let detached_flag = Rc::clone(&detached_cancelled);
        let (menu_owner, ()) = create_root(move || {
            cleanup(move || scoped_flag.set(true));
            Owner::detached().run(|| {
                cleanup(move || detached_flag.set(true));
            });
        });
        menu_owner.dispose();
        assert!(scoped_cancelled.get());
        assert!(!detached_cancelled.get());
    }

    #[test]
    fn list_sort_cycles_asc_desc_none_and_orders_rows() {
        let s0 = None;
        let s1 = super::ListSort::toggle(s0, super::ListColumn::Method);
        assert_eq!(
            s1,
            Some(super::ListSort {
                column: super::ListColumn::Method,
                desc: false
            })
        );
        let s2 = super::ListSort::toggle(s1, super::ListColumn::Method);
        assert_eq!(
            s2,
            Some(super::ListSort {
                column: super::ListColumn::Method,
                desc: true
            })
        );
        let s3 = super::ListSort::toggle(s2, super::ListColumn::Method);
        assert_eq!(s3, None);
        let s_other = super::ListSort::toggle(s2, super::ListColumn::Status);
        assert_eq!(
            s_other,
            Some(super::ListSort {
                column: super::ListColumn::Status,
                desc: false
            })
        );

        let make_row = |id: u64,
                        method: &str,
                        status: Option<u16>,
                        failed: bool,
                        size_bytes: u64,
                        elapsed_nanos: Option<u128>,
                        host: &str,
                        path: &str| super::RowLine {
            id,
            method: method.to_string(),
            status,
            time: "0 ms".to_string(),
            gap_nanos: id as u128 * 10,
            clock: "12:00:00.000".to_string(),
            elapsed: "—".to_string(),
            elapsed_nanos,
            elapsed_slow: false,
            host: host.to_string(),
            client_ip: "127.0.0.1".to_string(),
            path: path.to_string(),
            url: format!("https://{host}{path}"),
            comment: String::new(),
            protocol: "HTTP/1.1".to_string(),
            size: "0 B".to_string(),
            size_bytes,
            failed,
            pinned: false,
            relay_short: String::new(),
        };

        let base = vec![
            make_row(
                1,
                "POST",
                Some(500),
                false,
                120,
                Some(300),
                "b.test",
                "/users",
            ),
            make_row(
                2,
                "GET",
                Some(200),
                false,
                4096,
                Some(50),
                "a.test",
                "/alpha",
            ),
            make_row(3, "DELETE", None, true, 0, None, "c.test", "/err"),
            make_row(
                4,
                "GET",
                Some(200),
                false,
                512,
                Some(100),
                "a.test",
                "/zeta",
            ),
        ];

        let mut by_method_asc = base.clone();
        super::sort_row_lines(
            &mut by_method_asc,
            super::ListSort {
                column: super::ListColumn::Method,
                desc: false,
            },
            false,
        );
        let ids: Vec<u64> = by_method_asc.iter().map(|r| r.id).collect();
        // DELETE (3), GET (2, then 4 by id tie-breaker), POST (1)
        assert_eq!(ids, vec![3, 2, 4, 1]);

        let mut by_size_desc = base.clone();
        super::sort_row_lines(
            &mut by_size_desc,
            super::ListSort {
                column: super::ListColumn::Size,
                desc: true,
            },
            false,
        );
        let ids: Vec<u64> = by_size_desc.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![2, 4, 1, 3]);

        let mut by_elapsed_desc = base.clone();
        super::sort_row_lines(
            &mut by_elapsed_desc,
            super::ListSort {
                column: super::ListColumn::Elapsed,
                desc: true,
            },
            false,
        );
        let ids: Vec<u64> = by_elapsed_desc.iter().map(|r| r.id).collect();
        // Unfinished (`None`, id 3) stays at the bottom even in descending sort.
        assert_eq!(ids, vec![1, 4, 2, 3]);

        let candidate = super::SeqCandidate {
            id: 9,
            method: "GET".into(),
            host: "api.example.com".into(),
            path: "/v1/orders?limit=10".into(),
            status: Some(200),
            failed: false,
            elapsed: "12 ms".into(),
            comment: "fetch orders".into(),
            checked: true,
        };
        assert_eq!(
            super::seq_candidate_target(&candidate, false),
            "api.example.com/v1/orders"
        );
        assert_eq!(
            super::seq_candidate_target(&candidate, true),
            "api.example.com/v1/orders?limit=10"
        );
    }

    #[test]
    fn default_save_dir_prefers_xdg_downloads_then_home_and_cwd() {
        let base = std::env::temp_dir().join(format!("zniff-save-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let home = base.join("home");
        let custom_dl = home.join("MyDownloads");
        let std_dl = home.join("Downloads");
        let cwd = base.join("cwd");
        std::fs::create_dir_all(&custom_dl).unwrap();
        std::fs::create_dir_all(&std_dl).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();

        let home_str = home.to_string_lossy();
        let user_dirs = "XDG_DOWNLOAD_DIR=\"$HOME/MyDownloads\"\n";
        assert_eq!(
            super::default_save_dir_from(Some(&home_str), Some(user_dirs), Some(&cwd)),
            Some(custom_dl)
        );
        assert_eq!(
            super::default_save_dir_from(Some(&home_str), None, Some(&cwd)),
            Some(std_dl.clone())
        );
        std::fs::remove_dir_all(&std_dl).unwrap();
        assert_eq!(
            super::default_save_dir_from(Some(&home_str), None, Some(&cwd)),
            Some(home.clone())
        );
        assert_eq!(
            super::default_save_dir_from(None, None, Some(&cwd)),
            Some(cwd.clone())
        );

        let exported = base.join("nested").join("export.har");
        super::write_export_file(&exported, b"{\"log\":{}}").unwrap();
        assert_eq!(std::fs::read(&exported).unwrap(), b"{\"log\":{}}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn facet_toggles_host_app_and_mute_tokens_cleanly() {
        let f0 = "type:json";
        let f1 = super::toggle_host_facet(f0, "api.example.com");
        assert_eq!(f1, "host:api.example.com type:json");
        assert!(super::host_facet_active(&f1, "api.example.com"));
        let f2 = super::toggle_host_facet(&f1, "cdn.example.com");
        assert_eq!(f2, "host:api.example.com|cdn.example.com type:json");
        let f3 = super::toggle_host_facet(&f2, "api.example.com");
        assert_eq!(f3, "host:cdn.example.com type:json");
        let f4 = super::toggle_host_facet(&f3, "cdn.example.com");
        assert_eq!(f4, "type:json");

        let a1 = super::toggle_app_facet("type:json", "Google_Chrome");
        assert_eq!(a1, "type:json app:Google_Chrome");
        assert!(super::app_facet_active(&a1, "google_chrome"));
        let a2 = super::toggle_app_facet(&a1, "curl");
        assert_eq!(a2, "type:json app:Google_Chrome|curl");
        let a3 = super::toggle_app_facet(&a2, "Google_Chrome");
        assert_eq!(a3, "type:json app:curl");

        let m1 = super::toggle_mute_facet("type:json", "ads.example.com");
        assert_eq!(m1, "type:json mute:ads.example.com");
        assert!(super::mute_facet_active(&m1, "ads.example.com"));
        let m2 = super::toggle_mute_facet(&m1, "ads.example.com");
        assert_eq!(m2, "type:json");
    }

    #[test]
    fn seq_preview_height_fills_remaining_space_and_clamps_bounds() {
        assert_eq!(super::seq_preview_height(760.0, 7), 260.0);
        assert_eq!(super::seq_preview_height(900.0, 7), 400.0);
        assert_eq!(super::seq_preview_height(900.0, 20), 400.0);
        assert_eq!(super::seq_preview_height(900.0, 2), 560.0);
        assert_eq!(super::seq_preview_height(500.0, 7), 180.0);
        assert_eq!(super::seq_preview_height(0.0, 7), 260.0);
    }

    #[test]
    fn push_profile_history_keeps_last_ten_distinct_versions() {
        let mut history = Vec::new();
        for idx in 1..=12 {
            history = super::push_profile_history(
                &history,
                "test",
                &format!("host:v{idx}.example.com"),
                "",
                "no-cache *",
                "",
                true,
                true,
                false,
            );
        }
        assert_eq!(history.len(), crate::config::PROFILE_HISTORY_CAP);
        assert_eq!(history[0].filter, "host:v12.example.com");
        assert_eq!(history[9].filter, "host:v3.example.com");

        // Re-recording an existing content snapshot moves it to the front without duplicating.
        history = super::push_profile_history(
            &history,
            "test-renamed",
            "host:v5.example.com",
            "",
            "no-cache *",
            "",
            true,
            true,
            false,
        );
        assert_eq!(history.len(), 10);
        assert_eq!(history[0].filter, "host:v5.example.com");
        assert_eq!(history[0].name, "test-renamed");
        assert_eq!(
            history
                .iter()
                .filter(|item| item.filter == "host:v5.example.com")
                .count(),
            1
        );
    }

    #[test]
    fn toolbar_summary_shows_profile_name_and_primary_filter_conditions() {
        let saved = vec![crate::config::SavedProfile {
            name: "test".into(),
            filter: "host:test.gf.com.cn|t1.gf.com.cn path:/finance-api/online-hall".into(),
            bypass_hosts: String::new(),
            rules: "no-cache *".into(),
            upstream_proxy: String::new(),
            rules_enabled: true,
            breaks_enabled: true,
            show_tunnel_failures: false,
        }];
        assert_eq!(
            super::toolbar_active_profile_name(
                &saved,
                "",
                "host:test.gf.com.cn|t1.gf.com.cn path:/finance-api/online-hall",
                "",
                "no-cache *",
                "",
            ),
            Some("test".to_string())
        );
        assert_eq!(
            super::toolbar_active_profile_name(&saved, "custom", "host:a.com", "", "", ""),
            Some("custom".to_string())
        );
        assert_eq!(
            super::toolbar_filter_conditions(
                "host:test.gf.com.cn|t1.gf.com.cn path:/finance-api/online-hall",
                "",
                "no-cache *",
                "",
                true,
                crate::i18n::Lang::Zh,
            ),
            (
                "host:test.gf.com.cn|t1.gf.com.cn path:/finance-api/online-hall".to_string(),
                true,
            )
        );
        assert_eq!(
            super::toolbar_filter_conditions("", "", "", "", true, crate::i18n::Lang::Zh),
            ("全部流量 · 未设置过滤条件".to_string(), false)
        );
        assert_eq!(
            super::toolbar_filter_conditions(
                "",
                "",
                "",
                "http://upuser:s3cr3t@127.0.0.1:7890",
                true,
                crate::i18n::Lang::Zh,
            ),
            ("upstream:upuser@127.0.0.1:7890".to_string(), true)
        );
    }

    #[test]
    fn format_bytes_avoids_clipping_for_large_values() {
        assert_eq!(super::format_bytes(0), "0 B");
        assert_eq!(super::format_bytes(512), "512 B");
        assert_eq!(super::format_bytes(1024), "1.0 KB");
        assert_eq!(super::format_bytes(2048), "2.0 KB");
        assert_eq!(super::format_bytes(99 * 1024), "99.0 KB");
        // Values >= 100 KB omit decimal .0 to prevent cell clipping
        assert_eq!(super::format_bytes(100 * 1024), "100 KB");
        assert_eq!(super::format_bytes(256 * 1024), "256 KB");
        assert_eq!(super::format_bytes(1023 * 1024), "1023 KB");
        // MB formatting
        assert_eq!(super::format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(super::format_bytes(12 * 1024 * 1024), "12.0 MB");
        assert_eq!(super::format_bytes(99 * 1024 * 1024), "99.0 MB");
        assert_eq!(super::format_bytes(100 * 1024 * 1024), "100 MB");
        assert_eq!(super::format_bytes(256 * 1024 * 1024), "256 MB");
    }

    #[test]
    fn draft_rules_ignores_hidden_leftover_inputs_across_rule_kinds() {
        // Leftover `from` / `to` from Replace must not emit a global `cors *` when host & path are empty.
        assert_eq!(
            super::draft_rules("no-cache *", "", "", "old_from", "old_to", true, 8),
            "no-cache *"
        );
        assert!(!super::profile_save_ready(
            &[crate::config::SavedProfile {
                name: "dev".into(),
                filter: String::new(),
                bypass_hosts: String::new(),
                rules: "no-cache *".into(),
                upstream_proxy: String::new(),
                rules_enabled: true,
                breaks_enabled: true,
                show_tunnel_failures: false,
            }],
            "dev",
            "",
            "",
            "no-cache *",
            "",
            true,
            true,
            false,
            "",
            "",
            "LeftoverOp",
            "old_from",
            "old_to",
            true,
            8,
        ));
        // When `op` is filled on an `op=`-capable rule (`break`, kind 4), `draft_rules_with_op` commits it.
        assert_eq!(
            super::draft_rules_with_op("", "api.test", "/graphql", "GetUser", "", "", false, 4),
            "break api.test/graphql op=GetUser\n"
        );
        // When `op` is left over on `cors` (`kind == 8`), it is ignored.
        assert_eq!(
            super::draft_rules_with_op("", "api.test", "", "GetUser", "", "", true, 8),
            "cors api.test\n"
        );
        // When `host` IS filled on `cors`, leftover `from`/`to` are ignored and `cors api.test` is appended.
        assert_eq!(
            super::draft_rules("", "api.test", "", "old_from", "old_to", true, 8),
            "cors api.test\n"
        );
        // On `block` (`kind == 13`) in `Drop` mode (`response == true`), hidden `to` is ignored.
        assert_eq!(
            super::draft_rules("", "", "", "", "leftover with spaces", true, 13),
            ""
        );
        assert_eq!(
            super::draft_rules("", "api.test", "", "", "leftover with spaces", true, 13),
            "block api.test drop\n"
        );
        // On `map-remote` (`kind == 1`), hidden `to` is ignored so spaces in `to` do not reject a valid target.
        assert_eq!(
            super::draft_rules(
                "",
                "api.test",
                "/v1",
                "https://staging.test/v1",
                "leftover with spaces",
                true,
                1,
            ),
            "map-remote api.test/v1 https://staging.test/v1\n"
        );
    }

    #[test]
    fn url_encode_query_component_encodes_filter_operators_and_utf8() {
        assert_eq!(
            super::url_encode_query_component("host:a.test|b.test 失败"),
            "host%3Aa.test%7Cb.test%20%E5%A4%B1%E8%B4%A5"
        );
    }

    #[test]
    fn cli_info_output_handles_version_and_help() {
        assert_eq!(super::cli_info_output(&[]), None);
        assert_eq!(super::cli_info_output(&["--web".into()]), None);
        let expected_ver = format!("zniff {}", env!("CARGO_PKG_VERSION"));
        assert_eq!(
            super::cli_info_output(&["--version".into()]).as_deref(),
            Some(expected_ver.as_str())
        );
        assert_eq!(
            super::cli_info_output(&["-V".into()]).as_deref(),
            Some(expected_ver.as_str())
        );
        let help = super::cli_info_output(&["--help".into()]).expect("help text");
        assert!(help.contains(&expected_ver));
        assert!(help.contains("--web"));
        assert!(help.contains("--relay"));
        assert!(help.contains("--mcp"));
        assert_eq!(
            super::cli_info_output(&["-h".into()]).as_deref(),
            Some(help.as_str())
        );
    }

    #[test]
    fn merge_secondary_filter_deduplicates_tokens_and_normalizes_pipes() {
        assert_eq!(
            super::merge_secondary_filter("host:a.test type:json", "type:JSON path:/v1 | /v2"),
            "host:a.test type:json path:/v1|/v2"
        );
        assert_eq!(
            super::merge_secondary_filter("", "  slow:   pin: "),
            "slow: pin:"
        );
        assert_eq!(super::merge_secondary_filter("GET", "   "), "GET");
    }

    #[test]
    fn edit_action_label_and_seq_mermaid_mode_follow_context() {
        use crate::i18n::Lang;
        use crate::proxy::BreakSide;

        assert_eq!(
            super::edit_action_label(Lang::Zh, Some(5), None),
            "编辑重发"
        );
        assert_eq!(
            super::edit_action_label(Lang::En, Some(5), None),
            "Edit & resend"
        );
        assert_eq!(
            super::edit_action_label(Lang::En, Some(5), Some((5, BreakSide::Request))),
            "Edit request"
        );
        assert_eq!(
            super::edit_action_label(Lang::Zh, Some(5), Some((5, BreakSide::Response))),
            "编辑响应"
        );
        assert_eq!(
            super::edit_action_label(Lang::En, Some(6), Some((5, BreakSide::Response))),
            "Edit & resend"
        );

        assert_eq!(
            super::select_seq_mermaid_text(
                0,
                true,
                "sequenceDiagram\nA->>B: 1",
                "flowchart TD\nA-->B"
            ),
            "sequenceDiagram\nA->>B: 1"
        );
        assert_eq!(
            super::select_seq_mermaid_text(
                2,
                false,
                "sequenceDiagram\nA->>B: 1",
                "flowchart TD\nA-->B"
            ),
            "flowchart TD\nA-->B"
        );
        assert_eq!(
            super::select_seq_mermaid_text(
                3,
                true,
                "sequenceDiagram\nA->>B: 1",
                "flowchart TD\nA-->B"
            ),
            "flowchart TD\nA-->B"
        );
        assert_eq!(
            super::select_seq_mermaid_text(
                3,
                false,
                "sequenceDiagram\nA->>B: 1",
                "flowchart TD\nA-->B"
            ),
            "sequenceDiagram\nA->>B: 1"
        );
    }

    #[test]
    fn auto_height_popover_scroll_containers_never_use_flex_1() {
        for window_h in [
            0.0, 320.0, 348.0, 420.0, 520.0, 760.0, 917.0, 1018.0, 1440.0,
        ] {
            for import_open in [false, true] {
                let panel_max = super::adaptive_filter_panel_max_height(window_h);
                let body_max = super::adaptive_filter_body_max_height(window_h, import_open);
                let import_h = if import_open {
                    super::FILTER_PANEL_IMPORT_EXTRA_HEIGHT
                } else {
                    0.0
                };
                assert!(
                    body_max > 0.0,
                    "filter body_max must stay positive at window_h={window_h}, import_open={import_open}"
                );
                assert!(
                    body_max + super::FILTER_PANEL_DOCK_HEIGHT + import_h <= panel_max,
                    "filter body+dock ({body_max} + {} + {import_h}) exceeds panel_max ({panel_max}) at window_h={window_h}",
                    super::FILTER_PANEL_DOCK_HEIGHT
                );
            }
        }

        for raw_src in [
            std::borrow::Cow::Borrowed(include_str!("lib.rs")),
            std::borrow::Cow::Owned(
                include_str!("lib.rs")
                    .replace("\r\n", "\n")
                    .replace('\n', "\r\n"),
            ),
        ] {
            let normalized = raw_src.replace("\r\n", "\n");
            let prod_src = normalized
                .split("\n#[cfg(test)]\nmod tests")
                .next()
                .expect("lib.rs production source before mod tests");
            assert!(
                !prod_src.contains("fn auto_height_popover_scroll_containers_never_use_flex_1"),
                "prod_src must exclude #[cfg(test)] mod tests on both LF and CRLF checkouts"
            );

            // In Taffy Flexbox, `.flex_1()` sets `flex-basis: 0px`. Putting `.flex_1()` on a
            // vertical `scroll().w_full()` inside an auto-height `.max_h(...)` popover/menu/drawer
            // collapses the scroll body to 0px height!
            for (idx, _) in prod_src.match_indices("scroll()") {
                let tail = &prod_src[idx..];
                let head = tail.split(".child(").next().unwrap_or(tail);
                if head.contains(".w_full()") || head.contains(".max_h(") {
                    assert!(
                        !head.contains(".flex_1()"),
                        "scroll() with .w_full()/.max_h() must never use .flex_1() (collapses to 0px in auto-height parent): {head}"
                    );
                }
            }

            let filter_panel_src = prod_src
                .split("fn saved_filter_panel(")
                .nth(1)
                .and_then(|s| s.split("\nfn ").next())
                .expect("saved_filter_panel source");
            let filter_outer_and_scroll = filter_panel_src
                .split("view()\n                        .w_full()\n                        .p_3()")
                .next()
                .unwrap_or(filter_panel_src);
            assert!(
                filter_outer_and_scroll.contains(".max_h(")
                    && !filter_outer_and_scroll.contains(".flex_1()"),
                "saved_filter_panel outer auto-height view + scroll() must not use .flex_1()"
            );
        }
    }

    #[test]
    fn deselecting_active_profile_clears_config_and_keeps_saved_presets() {
        let test_preset = crate::config::SavedProfile {
            name: "test".into(),
            filter: "host:test.gf.com.cn|t1.gf.com.cn path:/finance-api/online-hall|/super".into(),
            bypass_hosts: "internal.example.com".into(),
            rules: "no-cache *".into(),
            upstream_proxy: "127.0.0.1:9000".into(),
            rules_enabled: false,
            breaks_enabled: false,
            show_tunnel_failures: true,
        };
        let mut cfg = crate::config::Config {
            active_profile: "test".into(),
            filter: test_preset.filter.clone(),
            bypass_hosts: test_preset.bypass_hosts.clone(),
            rules: test_preset.rules.clone(),
            upstream_proxy: test_preset.upstream_proxy.clone(),
            rules_enabled: false,
            breaks_enabled: false,
            show_tunnel_failures: true,
            saved_profiles: vec![test_preset.clone()],
            ..crate::config::Config::default()
        };

        let history = super::push_profile_history(
            &cfg.profile_history,
            &cfg.active_profile,
            &cfg.filter,
            &cfg.bypass_hosts,
            &cfg.rules,
            &cfg.upstream_proxy,
            cfg.rules_enabled,
            cfg.breaks_enabled,
            cfg.show_tunnel_failures,
        );
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].name, "test");

        super::clear_working_setup_config(&mut cfg);
        assert_eq!(cfg.active_profile, "");
        assert_eq!(cfg.filter, "");
        assert_eq!(cfg.bypass_hosts, "");
        assert_eq!(cfg.rules, "");
        assert_eq!(cfg.upstream_proxy, "");
        assert!(cfg.rules_enabled);
        assert!(cfg.breaks_enabled);
        assert!(!cfg.show_tunnel_failures);
        assert_eq!(cfg.saved_profiles, vec![test_preset]);

        let mut saved_with_blank = cfg.saved_profiles.clone();
        saved_with_blank.push(crate::config::SavedProfile {
            name: "empty".into(),
            filter: String::new(),
            bypass_hosts: String::new(),
            rules: String::new(),
            upstream_proxy: String::new(),
            rules_enabled: true,
            breaks_enabled: true,
            show_tunnel_failures: false,
        });
        assert_eq!(
            super::toolbar_active_profile_name(
                &saved_with_blank,
                &cfg.active_profile,
                &cfg.filter,
                &cfg.bypass_hosts,
                &cfg.rules,
                &cfg.upstream_proxy,
            ),
            None
        );
        assert_eq!(
            super::toolbar_filter_conditions(
                &cfg.filter,
                &cfg.bypass_hosts,
                &cfg.rules,
                &cfg.upstream_proxy,
                cfg.rules_enabled,
                crate::i18n::Lang::Zh,
            ),
            ("全部流量 · 未设置过滤条件".to_string(), false)
        );
    }
}
