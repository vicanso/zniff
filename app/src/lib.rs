//! zniff: a desktop HTTP(S) capture app. The window is an nsh Rust view.
//! The proxy runs on its own tokio runtime in this process.

#![allow(clippy::too_many_arguments)]

mod bypass;
mod ca;
mod client_hello;
mod config;
mod decompress;
mod har;
mod i18n;
mod icon;
mod present;
mod proxy;
mod rules;
mod session;
mod share;
mod sysproxy;
mod ws;

use std::cell::Cell;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use nsh_base::{
    DataTable, DataTableApi, DataTableCell, DataTableClasses, DataTableColumn,
    DataTableContextMenu, DataTableEvent, DataTableHeaderCell, DataTablePress, DataTableRange,
    DataTableRows, DataTableSelection, DiffLayout, data_table, diff_view, editor, input, menu,
    menu_item, popover, theme, tooltip,
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

pub use sysproxy::restore_if_dirty;

pub fn initial_window_size() -> (i32, i32) {
    let cfg = saved_config();
    (cfg.window_width as i32, cfg.window_height as i32)
}

struct Controls {
    hub: Arc<Hub>,
    capture: Mutex<Option<Capture>>,
    status: Mutex<StatusMsg>,
    running: Mutex<bool>,
    listen: Mutex<String>,
    listen_port: Mutex<u16>,
    ca_installed: Mutex<bool>,
    /// User preference. Off until they turn on 「系统代理」.
    use_system_proxy: Mutex<bool>,
    /// True only after this session's `sysproxy::enable` returned `Applied`.
    proxy_applied: Mutex<bool>,
    /// Start, stop, and the proxy switch share this so they cannot interleave.
    ops: Mutex<()>,
    /// Next 「开始」binds `0.0.0.0` when this is on.
    listen_all: Mutex<bool>,
}

#[derive(Clone, Copy)]
struct ProfileSignals {
    saved: Signal<Vec<config::SavedProfile>>,
    name: Signal<String>,
    filter: Signal<String>,
    bypass: Signal<String>,
    rules: Signal<String>,
    upstream: Signal<String>,
    rules_on: Signal<bool>,
    breaks_on: Signal<bool>,
    show_tunnels: Signal<bool>,
    list_width: Signal<f32>,
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

/// The app's UI set (`nsh.json` `"rust"`).
#[native_module("zniff/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

/// Header and rows keep a 2pt selection bar plus `px_3` (12pt). The filter uses the same inset.
const LIST_INSET: f32 = 14.0;
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
    AdaptiveColor::rgb(0x64748b, 0x7e8ba3)
}

fn accent_blue() -> AdaptiveColor {
    AdaptiveColor::rgb(0x2563eb, 0x3b82f6)
}

fn accent_blue_hover() -> AdaptiveColor {
    AdaptiveColor::rgb(0x1d4ed8, 0x2563eb)
}

fn accent_blue_soft() -> AdaptiveColor {
    AdaptiveColor::rgb(0xdbeafe, 0x172554)
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
    AdaptiveColor::rgb(0x059669, 0x34d399)
}

fn status_ok_bg() -> AdaptiveColor {
    AdaptiveColor::rgb(0xd1fae5, 0x0a2e26)
}

fn status_warn_fg() -> AdaptiveColor {
    AdaptiveColor::rgb(0xd97706, 0xfbbf24)
}

fn status_warn_bg() -> AdaptiveColor {
    AdaptiveColor::rgb(0xfef3c7, 0x33240b)
}

fn status_err_fg() -> AdaptiveColor {
    AdaptiveColor::rgb(0xe11d48, 0xfb7185)
}

fn status_err_bg() -> AdaptiveColor {
    AdaptiveColor::rgb(0xffe4e6, 0x361320)
}

fn danger_button_bg() -> AdaptiveColor {
    AdaptiveColor::rgb(0xdc2626, 0xe11d48)
}

fn danger_button_hover() -> AdaptiveColor {
    AdaptiveColor::rgb(0xb91c1c, 0xbe123c)
}

fn method_fg(method: &str) -> AdaptiveColor {
    match method {
        "GET" => AdaptiveColor::rgb(0x0284c7, 0x38bdf8),
        "POST" => AdaptiveColor::rgb(0x059669, 0x34d399),
        "PUT" | "PATCH" => AdaptiveColor::rgb(0xd97706, 0xfbbf24),
        "DELETE" => AdaptiveColor::rgb(0xe11d48, 0xfb7185),
        "CONNECT" => AdaptiveColor::rgb(0x7c3aed, 0xa78bfa),
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
    /// Local `HH:MM:SS.mmm`, shown when the gap cell is hovered.
    clock: String,
    /// Request start until the full response. `—` while it is still open.
    elapsed: String,
    elapsed_slow: bool,
    host: String,
    client_ip: String,
    path: String,
    url: String,
    protocol: String,
    size: String,
    failed: bool,
    pinned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

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

    fn visible_count(self) -> usize {
        ListColumn::ORDERED
            .into_iter()
            .filter(|&col| self.on(col))
            .count()
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
    }
}

/// Columns the table shows, in order. Id, Method, and Path (when shown) stay in
/// the fixed left group. The rest scroll under them.
/// Widths are the last saved drag, not a live value: the table keeps a drag
/// until this list changes, and a changed width here would fight the drag.
fn flow_table_columns(shown: ListColumns, widths: &HashMap<String, u32>) -> Vec<DataTableColumn> {
    ListColumn::ORDERED
        .into_iter()
        .filter(|&col| shown.on(col))
        .map(|col| {
            let key = col.key();
            let fixed = matches!(col, ListColumn::Id | ListColumn::Method | ListColumn::Path);
            let limit = config::column_width_limit(key).expect("known column");
            let width = widths.get(key).copied().unwrap_or(limit.default);
            let mut column = DataTableColumn::default();
            column.key = key.into();
            column.width = Some(px(width as f32));
            column.min_width = Some(px(limit.min as f32));
            column.max_width = Some(px(limit.max as f32));
            column.fixed = fixed;
            column.resizable = Some(true);
            column.movable = Some(false);
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
    classes.header_cell = Some("bg-[#f1f5f9] dark:bg-[#151b26]".into());
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
    doc_preview: Signal<bool>,
    request_form: Signal<Vec<HeaderLine>>,
    request_note: Signal<String>,
    response_note: Signal<String>,
    metrics: Signal<String>,
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
    let langs = signal(load_lang_pref());
    provide_context(langs);
    let status = signal(StatusMsg::Idle);
    let running = signal(false);
    let ca_installed = signal(false);
    let use_system_proxy = signal(false);
    let memory = signal(String::from("0 B"));
    let dropped = signal(0_u64);
    let saved_filter = saved_config();
    let filter = signal(saved_filter.filter);
    let saved_profiles = signal(saved_filter.saved_profiles);
    let selected = signal(None::<u64>);
    let rows = signal(Vec::<RowLine>::new());
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
        doc_preview: signal(false),
        request_form: signal(Vec::new()),
        request_note: signal(String::new()),
        response_note: signal(String::new()),
        metrics: signal(String::new()),
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
    };
    let port_text = signal(String::new());
    let listen_all = signal(false);
    let paused = signal(None::<(u64, BreakSide)>);
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
    let list_width = signal(saved.list_width as f32);
    let columns = signal(ListColumns::from_config(&saved));
    let bypass_text = signal(saved.bypass_hosts);
    let rules_text = signal(saved.rules);
    let upstream_text = signal(saved.upstream_proxy);
    let profile_signals = ProfileSignals {
        saved: saved_profiles,
        name: signal(String::new()),
        filter,
        bypass: bypass_text,
        rules: rules_text,
        upstream: upstream_text,
        rules_on: signal(true),
        breaks_on: signal(true),
        show_tunnels: signal(saved.show_tunnel_failures),
        list_width,
    };
    let lan_url = signal(String::new());
    let lan_choice = signal(String::new());
    let lan_addrs = signal(Vec::<LanAddr>::new());
    let qr_path = signal(String::new());
    let controls = open_controls();
    status.set(lock_status(&controls));
    ca_installed.set(lock_bool(&controls.ca_installed));
    use_system_proxy.set(lock_bool(&controls.use_system_proxy));
    port_text.set(lock_port(&controls.listen_port).to_string());
    listen_all.set(lock_bool(&controls.listen_all));
    if saved_config().capturing {
        let controls_boot = controls.clone();
        std::thread::spawn(move || start_capture(&controls_boot));
    }

    let controls_tick = controls.clone();
    let filter_tick = filter;
    let filter_pending = stored(filter.get());
    let filter_since = stored(None::<SystemTime>);
    let win_pending = stored((saved.window_width, saved.window_height));
    let win_since = stored(None::<SystemTime>);
    let selected_tick = selected;
    let rows_tick = rows;
    let total_tick = flow_total;
    let shown_tick = shown;
    let seen_gen = stored(0_u64);
    let seen_filter = stored(String::new());
    let seen_selected = stored(None::<u64>);
    let seen_listen = stored(String::new());
    let seen_lang = stored(ui_lang(langs));
    let baseline_tick = baseline;
    let seen_baseline = stored(None::<u64>);
    let seen_pretty = stored(shown_tick.pretty.get());
    let share_id = signal(String::new());
    let share_id_tick = share_id;
    let seen_share = stored(String::new());
    let spill_gc_at = stored(SystemTime::now());
    // The hex text's own box is only as wide as the current line, so measuring
    // it always says 16 bytes fit. The pane viewport is the width the card
    // actually has. Publishing it from this tick rebuilds the text; a signal
    // write inside `on_layout` can miss that text.
    let pane_px = Rc::new(Cell::new(0.0_f32));
    let pane_tick = pane_px.clone();
    set_interval(Duration::from_millis(16), move || {
        let width = pane_tick.get();
        let published = shown_tick.pane_width.get_untracked();
        if hex_cols_for_pane(width) != hex_cols_for_pane(published) {
            shown_tick.pane_width.set(width);
        }
        let share_now = controls_tick.hub.share_id().unwrap_or_default();
        if share_now != seen_share.get() {
            seen_share.replace(share_now.clone());
            share_id_tick.set(share_now);
        }
        status.set(lock_status(&controls_tick));
        running.set(
            *controls_tick
                .running
                .lock()
                .unwrap_or_else(|err| err.into_inner()),
        );
        let listen_now = lock_string(&controls_tick.listen);
        listen.set(listen_now.clone());
        ca_installed.set(lock_bool(&controls_tick.ca_installed));
        use_system_proxy.set(lock_bool(&controls_tick.use_system_proxy));
        paused.set(controls_tick.hub.paused());
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
        if filter_now != filter_pending.get() {
            filter_pending.replace(filter_now.clone());
            filter_since.replace(Some(SystemTime::now()));
        } else if filter_since
            .get()
            .is_some_and(|at| at.elapsed().unwrap_or_default() >= Duration::from_millis(500))
        {
            remember_filter(&filter_now);
            filter_since.replace(None);
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
                win_since.replace(Some(SystemTime::now()));
            } else if win_since
                .get()
                .is_some_and(|at| at.elapsed().unwrap_or_default() >= Duration::from_millis(500))
            {
                remember_window_size(win_now.0, win_now.1);
                win_since.replace(None);
            }
        }
        let spill_expired =
            if spill_gc_at.get().elapsed().unwrap_or_default() >= Duration::from_secs(15) {
                let now = SystemTime::now();
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
        if generation != seen_gen.get()
            || filter_now != seen_filter.get()
            || selected_now != seen_selected.get()
            || lang_now != seen_lang.get()
            || baseline_now != seen_baseline.get()
            || pretty_now != seen_pretty.get()
            || spill_expired
        {
            seen_gen.replace(generation);
            seen_filter.replace(filter_now.clone());
            seen_selected.replace(selected_now);
            seen_lang.replace(lang_now);
            seen_baseline.replace(baseline_now);
            seen_pretty.replace(pretty_now);
            let lines = row_lines(&controls_tick.hub, &filter_now, lang_now);
            total_tick.set(controls_tick.hub.with_session(|session| session.len()));
            rows_tick.set(lines);
            memory.set(format_bytes(
                controls_tick
                    .hub
                    .with_session(|session| session.memory_bytes()),
            ));
            dropped.set(controls_tick.hub.with_session(|session| session.dropped()));
            publish_detail(
                &shown_tick,
                &controls_tick.hub,
                selected_now,
                lang_now,
                baseline_now,
            );
        }
    });

    let controls_start = controls.clone();
    let controls_ca = controls.clone();
    let controls_remove = controls.clone();
    let controls_proxy = controls.clone();
    let controls_clear = controls.clone();
    let controls_clear_key = controls_clear.clone();
    let controls_replay_key = controls.clone();
    let controls_export = controls.clone();
    let controls_env = controls.clone();
    let selected_clear = selected;
    let baseline_clear = baseline;
    let compose_clear = compose;
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
    on_key("mod+k", move |_| {
        clear_captured(
            &controls_clear_key,
            selected_clear_key,
            baseline_clear_key,
            compose_clear_key,
        );
    });
    on_key("mod+f", move |_| filter_focus.focus());
    let selected_replay = selected;
    on_key("mod+r", move |_| {
        replay_flow(
            &controls_replay_key,
            selected_replay.get(),
            ReplayOverride::default(),
        );
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
            ca_installed,
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
                );
            },
            move || export_har_file(controls_export.clone(), &filter.get()),
            move || copy_proxy_env(&controls_env),
            move || toggle_capture(controls_start.clone()),
            controls_import,
            rows,
            flow_total,
            filter,
            shown.pretty,
            share_id,
            profile_signals,
        ))
        .child(when(
            move || paused.get().is_some(),
            move || break_bar(paused, compose, selected, controls_break.clone()),
        ))
        .child(
            view()
                .flex_1()
                .flex_row()
                .child(flow_list(
                    rows,
                    selected,
                    compose,
                    filter,
                    rules_text,
                    list_width,
                    columns,
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
                    compose,
                    filter,
                    baseline,
                    folded,
                    pane_px,
                )),
        )
        .child(footer(
            listen, running, memory, dropped, port_text, lan_url, lan_choice, lan_addrs, qr_path,
            listen_all, controls,
        ))
}

fn toolbar(
    status: Signal<StatusMsg>,
    running: Signal<bool>,
    ca_installed: Signal<bool>,
    use_system_proxy: Signal<bool>,
    install: impl Fn() + 'static,
    remove: impl Fn() + 'static,
    toggle_proxy: impl Fn() + 'static,
    clear: impl Fn() + 'static,
    export_har: impl Fn() + 'static,
    copy_env: impl Fn() + 'static,
    toggle: impl Fn() + 'static,
    import_controls: Arc<Controls>,
    rows: Signal<Vec<RowLine>>,
    flow_total: Signal<usize>,
    filter: Signal<String>,
    pretty: Signal<bool>,
    share_id: Signal<String>,
    profiles: ProfileSignals,
) -> impl IntoElement {
    let install = Rc::new(install);
    let clear = Rc::new(clear);
    let remove = Rc::new(remove);
    let filter_controls = import_controls.clone();
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .h(px(50.0))
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
                        .w(px(22.0))
                        .h(px(22.0))
                        .rounded(px(6.0))
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
                        .text_size(px(15.5))
                        .font_weight(FontWeight::BOLD)
                        .text_color(fg_primary()),
                ),
        )
        .child(view().w(px(1.0)).h(px(16.0)).bg(border_strong()))
        .child(
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
                        .rounded(px(99.0))
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
                    text({
                        let langs = ui_langs();
                        move || status.get().text(ui_lang(langs))
                    })
                    .flex_1()
                    .number_of_lines(1)
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || {
                        if running.get() {
                            fg_secondary()
                        } else {
                            fg_muted()
                        }
                    }),
                ),
        )
        .child(flow_count_badge(rows, flow_total, filter))
        .child(toolbar_text_button(tr("清空", "Clear"), {
            let clear = Rc::clone(&clear);
            move || clear()
        }))
        .child(saved_filter_menu(profiles, filter_controls))
        .child(toolbar_menu(
            ca_installed,
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
            import_controls,
            rows,
            flow_total,
            filter,
            pretty,
            share_id,
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
    popover()
        .placement(Placement::Bottom)
        .align(Align::Start)
        .offset(px(6.0))
        .margin(px(10.0))
        .trigger(
            view()
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
                    } else if !filter.get().trim().is_empty()
                        || !saved.with(Vec::is_empty)
                        || config_set(bypass_text, rules_text, upstream_text)
                    {
                        accent_blue_border()
                    } else {
                        border_control()
                    }
                })
                .bg(move || {
                    if !filter.get().trim().is_empty() {
                        accent_blue_soft()
                    } else {
                        bg_control()
                    }
                })
                .hover(|style| {
                    style.bg(move || {
                        if !filter.get().trim().is_empty() {
                            accent_blue_soft()
                        } else {
                            bg_control_hover()
                        }
                    })
                })
                .cursor(Cursor::Pointer)
                .tooltip(label_tip("过滤与规则配置", "Filters & Rules Config"))
                .child(when(
                    move || {
                        !filter.get().trim().is_empty()
                            || config_set(bypass_text, rules_text, upstream_text)
                            || capture_muted(rules_on, breaks_on)
                    },
                    move || {
                        view().w(px(6.0)).h(px(6.0)).rounded(px(99.0)).bg(move || {
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
                        } else if !filter.get().trim().is_empty()
                            || config_set(bypass_text, rules_text, upstream_text)
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
                    } else if !filter.get().trim().is_empty() {
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
    let saved = profiles.saved;
    let bypass_text = profiles.bypass;
    let rules_text = profiles.rules;
    let upstream_text = profiles.upstream;
    let rules_on = profiles.rules_on;
    let breaks_on = profiles.breaks_on;
    let show_tunnels = profiles.show_tunnels;
    let rule_name = profiles.name;
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
    let tunnel_controls = controls;
    let draft_host = signal(String::new());
    let draft_path = signal(String::new());
    let draft_from = signal(String::new());
    let draft_to = signal(String::new());
    let draft_response = signal(true);
    let draft_kind = signal(0_u8);

    view()
        .w(move || {
            px(adaptive_filter_panel_width(
                window().width.0,
                list_width.get(),
            ))
        })
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
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
                                                            .h(px(22.0))
                                                            .px_2()
                                                            .flex_shrink_0()
                                                            .rounded(px(6.0))
                                                            .border_1()
                                                            .border_color(border_control())
                                                            .bg(bg_control())
                                                            .hover(|style| {
                                                                style.bg(bg_control_hover())
                                                            })
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
                                .child(rules_editor(rules_text, rules_controls.clone())),
                        )
                        .child(replace_row(
                            rules_text,
                            rules_controls,
                            draft_host,
                            draft_path,
                            draft_from,
                            draft_to,
                            draft_response,
                            draft_kind,
                        ))
                        .child(config_field(
                            "上游代理",
                            "Upstream Proxy",
                            side_input(
                                upstream_text,
                                tr("host:port，空着直连", "host:port, empty means direct"),
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
                        )),
                ),
        )
        .child(profile_dock(
            rule_name,
            field,
            bypass_text,
            rules_text,
            upstream_text,
            rules_on,
            breaks_on,
            show_tunnels,
            saved,
            draft_host,
            draft_path,
            draft_from,
            draft_to,
            draft_response,
            draft_kind,
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

fn profile_dock(
    rule_name: Signal<String>,
    filter: Signal<String>,
    bypass_text: Signal<String>,
    rules_text: Signal<String>,
    upstream_text: Signal<String>,
    rules_on: Signal<bool>,
    breaks_on: Signal<bool>,
    show_tunnels: Signal<bool>,
    saved: Signal<Vec<config::SavedProfile>>,
    draft_host: Signal<String>,
    draft_path: Signal<String>,
    draft_from: Signal<String>,
    draft_to: Signal<String>,
    draft_response: Signal<bool>,
    draft_kind: Signal<u8>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let preset_controls = controls.clone();
    let blank_controls = controls.clone();
    let export_controls = controls.clone();
    let import_controls = controls.clone();
    let save_controls = controls;
    let dropdown_open = signal(false);
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

    view()
        .w_full()
        .px_3()
        .py_2p5()
        .border_t_1()
        .border_t_color(border_subtle())
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
                                .w(px(176.0))
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
                                    if shown_name().is_some() {
                                        accent_blue_soft()
                                    } else {
                                        bg_control()
                                    }
                                })
                                .hover(|style| {
                                    style.bg(move || {
                                        if shown_name().is_some() {
                                            accent_blue_soft()
                                        } else {
                                            bg_control_hover()
                                        }
                                    })
                                })
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .cursor(Cursor::Pointer)
                                .child(
                                    view()
                                        .w(px(6.0))
                                        .h(px(6.0))
                                        .rounded(px(99.0))
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
                                    .flex_1()
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
                                .max_h(px(220.0))
                                .p_1()
                                .rounded(px(10.0))
                                .border_1()
                                .border_color(border_strong())
                                .bg(bg_card())
                                .shadow_md()
                                .child(scroll().w_full().max_h(px(210.0)).child(
                                    view().w_full().gap_0p5().child(each(
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
                                                        preset.name.eq_ignore_ascii_case(typed)
                                                    });
                                                }
                                                mark.with(|preset| {
                                                    preset.filter.trim() == filter.get().trim()
                                                        && preset.bypass_hosts.trim()
                                                            == bypass_text.get().trim()
                                                        && preset.rules.trim()
                                                            == rules_text.get().trim()
                                                        && preset.upstream_proxy.trim()
                                                            == upstream_text.get().trim()
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
                                                        .hover(|style| style.bg(bg_control_hover()))
                                                        .cursor(Cursor::Pointer)
                                                        .on_select(move || {
                                                            let preset =
                                                                apply.with(|preset| preset.clone());
                                                            rule_name.set(preset.name.clone());
                                                            clear_rule_draft(
                                                                draft_host,
                                                                draft_path,
                                                                draft_from,
                                                                draft_to,
                                                                draft_response,
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
                                                        })
                                                        .child(
                                                            view()
                                                                .w(px(6.0))
                                                                .h(px(6.0))
                                                                .rounded(px(99.0))
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
                                                                .max_w(px(150.0))
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
                                                                    .max_w(px(138.0))
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
                                                        ),
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
                                                        .hover(|style| style.bg(bg_control_hover()))
                                                        .tooltip(label_tip(
                                                            "删除配置",
                                                            "Delete setup",
                                                        ))
                                                        .on_press(move || {
                                                            let target = remove
                                                                .with(|preset| preset.clone());
                                                            drop_saved_profile(&target, saved);
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
                                )),
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
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .cursor(Cursor::Pointer)
                        .tooltip(label_tip(
                            "导出配置为 TOML 文件并复制到剪贴板",
                            "Export setups to a TOML file and copy",
                        ))
                        .on_press(move || {
                            dropdown_open.set(false);
                            let rules_now = draft_rules(
                                &rules_text.get(),
                                &draft_host.get(),
                                &draft_path.get(),
                                &draft_from.get(),
                                &draft_to.get(),
                                draft_response.get(),
                                draft_kind.get(),
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
                            let preferred_name = shown_name().unwrap_or_default();
                            export_profiles_file(export_controls.clone(), preferred_name, profiles);
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
                        .border_color(border_control())
                        .bg(bg_control())
                        .hover(|style| style.bg(bg_control_hover()))
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .cursor(Cursor::Pointer)
                        .tooltip(label_tip(
                            "选择 TOML 配置文件导入（或拖拽 .toml 到窗口）",
                            "Pick a TOML setup file to import (or drop .toml)",
                        ))
                        .on_press(move || {
                            dropdown_open.set(false);
                            let import_controls = import_controls.clone();
                            spawn(async move {
                                let dialog = nsh_core::file_dialog::OpenDialog::files().filter(
                                    nsh_core::file_dialog::FileFilter::new("TOML", ["toml"]),
                                );
                                match pick_paths(dialog).await {
                                    Ok(Some(paths)) => {
                                        if let Some(path) = paths.first() {
                                            clear_rule_draft(
                                                draft_host,
                                                draft_path,
                                                draft_from,
                                                draft_to,
                                                draft_response,
                                                draft_kind,
                                            );
                                            import_profiles_file(
                                                &import_controls,
                                                &path.to_string_lossy(),
                                                ProfileSignals {
                                                    saved,
                                                    name: rule_name,
                                                    filter,
                                                    bypass: bypass_text,
                                                    rules: rules_text,
                                                    upstream: upstream_text,
                                                    rules_on,
                                                    breaks_on,
                                                    show_tunnels,
                                                    list_width: signal(
                                                        config::LIST_WIDTH_DEFAULT as f32,
                                                    ),
                                                },
                                            );
                                        }
                                    }
                                    Ok(None) => {}
                                    Err(err) => {
                                        set_status(&import_controls, StatusMsg::ImportFailed(err));
                                    }
                                }
                            });
                        })
                        .child(
                            text(tr("导入", "Import"))
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg_secondary()),
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
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .cursor(Cursor::Pointer)
                        .on_press(move || {
                            dropdown_open.set(false);
                            adopted.set(true);
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
                                draft_from,
                                draft_to,
                                draft_response,
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
                            &draft_from.get(),
                            &draft_to.get(),
                            draft_response.get(),
                            draft_kind.get(),
                        )
                    });
                    let ready_bg = save_ready.clone();
                    let ready_border = save_ready.clone();
                    let ready_hover = save_ready.clone();
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
                            let from = draft_from.get();
                            let to = draft_to.get();
                            let current = rules_text.get();
                            let rules_now = draft_rules(
                                &current,
                                &host,
                                &path,
                                &from,
                                &to,
                                draft_response.get(),
                                draft_kind.get(),
                            );
                            if rules_now != current {
                                rules_text.set(rules_now.clone());
                                save_controls.hub.set_rules(&rules_now);
                                let persisted = rules_now.clone();
                                let _ = save_config(|config| config.rules = persisted);
                                draft_host.set(String::new());
                                draft_path.set(String::new());
                                draft_from.set(String::new());
                                draft_to.set(String::new());
                            }
                            store_saved_profile(
                                &rule_name.get(),
                                &filter.get(),
                                &bypass_text.get(),
                                &rules_now,
                                &upstream_text.get(),
                                rules_on.get(),
                                breaks_on.get(),
                                show_tunnels.get(),
                                saved,
                            );
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

fn clear_rule_draft(
    host: Signal<String>,
    path: Signal<String>,
    from: Signal<String>,
    to: Signal<String>,
    response: Signal<bool>,
    kind: Signal<u8>,
) {
    host.set(String::new());
    path.set(String::new());
    from.set(String::new());
    to.set(String::new());
    response.set(true);
    kind.set(0);
}

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
    draft_from: Signal<String>,
    draft_to: Signal<String>,
    draft_response: Signal<bool>,
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
        draft_from,
        draft_to,
        draft_response,
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
    let _ = save_config(|config| {
        config.filter.clear();
        config.bypass_hosts.clear();
        config.rules.clear();
        config.upstream_proxy.clear();
        config.show_tunnel_failures = false;
    });
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
) {
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
    if next == saved.get() {
        return;
    }
    saved.set(next.clone());
    let _ = save_config(|config| config.saved_profiles = next);
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
        config.filter = profile.filter.clone();
        config.bypass_hosts = profile.bypass_hosts.clone();
        config.rules = profile.rules.clone();
        config.upstream_proxy = profile.upstream_proxy.clone();
        config.show_tunnel_failures = profile.show_tunnel_failures;
    });
}

fn drop_saved_profile(target: &config::SavedProfile, saved: Signal<Vec<config::SavedProfile>>) {
    let next: Vec<config::SavedProfile> = saved
        .get()
        .into_iter()
        .filter(|item| item != target)
        .collect();
    saved.set(next.clone());
    let _ = save_config(|config| config.saved_profiles = next);
}

/// Rules after 保存 commits a filled add-rule row. An empty row stays out,
/// so it cannot turn into a global `*`.
fn draft_rules(
    rules: &str,
    host: &str,
    path: &str,
    from: &str,
    to: &str,
    response: bool,
    kind: u8,
) -> String {
    let touched = !host.trim().is_empty()
        || !path.trim().is_empty()
        || !from.trim().is_empty()
        || !to.trim().is_empty();
    if !touched {
        return rules.to_string();
    }
    let combined = crate::rules::combine_rule_pattern(host, path);
    crate::rules::append_quick_rule(rules, quick_kind(kind), &combined, response, from, to)
        .unwrap_or_else(|| rules.to_string())
}

fn resolve_profile_name(name: &str, filter: &str, rules: &str) -> String {
    let name = name.trim();
    if !name.is_empty() {
        return name.to_string();
    }
    if !filter.is_empty() {
        return filter.chars().take(28).collect();
    }
    if !rules.is_empty() {
        return rules
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .unwrap_or("配置")
            .chars()
            .take(28)
            .collect();
    }
    "配置".to_string()
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
    draft_from: &str,
    draft_to: &str,
    draft_response: bool,
    draft_kind: u8,
) -> bool {
    let rules = draft_rules(
        rules,
        draft_host,
        draft_path,
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
        let max_w = (window_w - 20.0).max(680.0);
        (window_w - list_px * 0.7 - 16.0)
            .round()
            .clamp(680.0, 1360.0)
            .min(max_w)
    } else {
        760.0
    }
}

fn adaptive_rules_editor_height(window_h: f32) -> f32 {
    if window_h.is_finite() && window_h > 0.0 {
        (window_h - 460.0).round().clamp(105.0, 560.0)
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
            store_list_width(
                clamp_list_px_for_window(width_release.get(), window().width.0),
                &controls_release,
            );
        })
        .on_responder_terminate(move |_event: &ResponderEvent| {
            store_list_width(width_cancel.get(), &controls_cancel);
        })
        .child(
            view()
                .w(move || if hot.get() { px(3.0) } else { px(1.0) })
                .h_full()
                .bg(move || {
                    if hot.get() {
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

fn flow_list(
    rows: Signal<Vec<RowLine>>,
    selected: Signal<Option<u64>>,
    compose: Compose,
    filter: Signal<String>,
    rules_text: Signal<String>,
    width: Signal<f32>,
    columns: Signal<ListColumns>,
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
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(view().flex_1().min_w(px(0.0)).child(focused_input(
                            filter,
                            tr(
                                "主机、路径，或点击顶栏「过滤」配置规则",
                                "Host, path, or click Filters in toolbar",
                            ),
                            30.0,
                            false,
                            move |text| filter.set(text.to_string()),
                            Some(filter_focus),
                        )))
                        .child(when(
                            move || !filter.get().is_empty(),
                            move || {
                                view()
                                    .w(px(30.0))
                                    .h(px(30.0))
                                    .flex_shrink_0()
                                    .flex_row()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(CONTROL_RADIUS))
                                    .border_1()
                                    .border_color(border_control())
                                    .bg(bg_control())
                                    .hover(|style| style.bg(bg_control_hover()))
                                    .cursor(Cursor::Pointer)
                                    .tooltip(label_tip("清空过滤", "Clear filter"))
                                    .on_press(move || filter.set(String::new()))
                                    .child(
                                        control_text("×")
                                            .text_size(px(14.0))
                                            .text_color(fg_secondary()),
                                    )
                            },
                        )),
                )
                .child(column_menu(columns, column_controls)),
        )
        .child(view().flex_1().min_h(px(0.0)).child(list_menu(
            rows,
            selected,
            compose,
            columns,
            row_controls,
            filter,
            shown,
            baseline,
            rules_text,
            picked,
            menu_open,
            menu_at,
        )))
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
                    accent_blue_soft()
                } else {
                    bg_control_hover()
                }
            })
        })
        .cursor(Cursor::Pointer)
        .on_press(move || filter.set(toggle_filter_chip(&filter.get(), token, aliases)))
        .child(view().w(px(5.0)).h(px(5.0)).rounded(px(99.0)).bg(move || {
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
                    accent_blue_soft()
                } else {
                    bg_control_hover()
                }
            })
        })
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
                .rounded(px(99.0))
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

fn rules_editor(value: Signal<String>, controls: Arc<Controls>) -> impl IntoElement {
    view()
        .w_full()
        .h(move || px(adaptive_rules_editor_height(window().height.0)))
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
                    accent_blue_soft()
                } else {
                    bg_control_hover()
                }
            })
        })
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
    from: Signal<String>,
    to: Signal<String>,
    response: Signal<bool>,
    kind: Signal<u8>,
) -> impl IntoElement {
    let kinds = [0_u8, 1, 2, 3, 4, 5, 6, 7];
    let ready_to_add = move || {
        let combined = crate::rules::combine_rule_pattern(&host_pattern.get(), &path_pattern.get());
        crate::rules::append_quick_rule(
            "",
            quick_kind(kind.get()),
            &combined,
            response.get(),
            &from.get(),
            &to.get(),
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
                .child(view().w_full().flex_row().items_center().gap_1().children(
                    kinds.into_iter().map(move |idx| {
                        let (zh, en) = quick_kind(idx).label();
                        view()
                            .h(px(26.0))
                            .px_1()
                            .flex_1()
                            .min_w(px(0.0))
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
                                        accent_blue_soft()
                                    } else {
                                        bg_control_hover()
                                    }
                                })
                            })
                            .cursor(Cursor::Pointer)
                            .on_press(move || kind.set(idx))
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
                    }),
                ))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(view().flex_1().min_w(px(0.0)).child(side_input(
                            host_pattern,
                            tr("域名，空着匹配全部", "Domain, empty matches all"),
                            28.0,
                            false,
                            move |text| host_pattern.set(text.to_string()),
                        )))
                        .child(view().flex_1().min_w(px(0.0)).child(side_input(
                            path_pattern,
                            tr("Path 前缀，空着匹配全部", "Path prefix, empty matches all"),
                            28.0,
                            false,
                            move |text| path_pattern.set(text.to_string()),
                        )))
                        .child(when(
                            move || kind.get() == 0 || kind.get() == 6,
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
                                    .child(quick_side_chip(response, false, "请求", "Req"))
                                    .child(quick_side_chip(response, true, "响应", "Res"))
                            },
                        )),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(when(
                            move || kind.get() < 4 || kind.get() == 6,
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
                                            "本地文件路径: ./mock.json",
                                            "Local file path: ./mock.json",
                                        )
                                        .to_string(),
                                        3 => t(
                                            lang,
                                            "延迟时长: 2000ms 或 2s",
                                            "Delay duration: 2000ms or 2s",
                                        )
                                        .to_string(),
                                        6 => {
                                            t(lang, "Header 名称: X-Debug", "Header name: X-Debug")
                                                .to_string()
                                        }
                                        _ => t(lang, "查找原文 (from)", "Find text (from)")
                                            .to_string(),
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
                        ))
                        .child(when(
                            move || kind.get() == 2,
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
                                        spawn(async move {
                                            if let Ok(Some(paths)) = pick_paths(
                                                nsh_core::file_dialog::OpenDialog::files(),
                                            )
                                            .await
                                                && let Some(path) = paths.first()
                                            {
                                                from.set(path.display().to_string());
                                            }
                                        });
                                    })
                                    .child(
                                        text(tr("选择文件", "Browse"))
                                            .text_size(px(11.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(fg_secondary()),
                                    )
                            },
                        ))
                        .child(when(
                            move || kind.get() == 0 || kind.get() == 6,
                            move || {
                                let placeholder = move || {
                                    let lang = ui_lang(ui_langs());
                                    if kind.get() == 6 {
                                        t(lang, "Header 值 (留空删除)", "Value (empty to delete)")
                                            .to_string()
                                    } else {
                                        t(lang, "替换成 (to)", "Replace with (to)").to_string()
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
                        ))
                        .child(
                            view()
                                .h(px(28.0))
                                .px_3()
                                .flex_grow(move || {
                                    if matches!(kind.get(), 4 | 5 | 7) {
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
                                            accent_blue_soft()
                                        } else {
                                            bg_control_hover()
                                        }
                                    })
                                })
                                .cursor(Cursor::Pointer)
                                .on_press(move || {
                                    let combined_pattern = crate::rules::combine_rule_pattern(
                                        &host_pattern.get(),
                                        &path_pattern.get(),
                                    );
                                    let Some(next) = crate::rules::append_quick_rule(
                                        &rules_text.get(),
                                        quick_kind(kind.get()),
                                        &combined_pattern,
                                        response.get(),
                                        &from.get(),
                                        &to.get(),
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
                        ),
                ),
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
            .text_size(px(12.0))
            .text_color(fg_muted()),
        )
}

fn list_menu(
    rows: Signal<Vec<RowLine>>,
    selected: Signal<Option<u64>>,
    compose: Compose,
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
    filter: Signal<String>,
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
            controls.clone(),
            filter,
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
                        controls.clone(),
                        filter,
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
    rows: Signal<Vec<RowLine>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    compose: Compose,
    columns: Signal<ListColumns>,
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
    view()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .child(dynamic(move || {
            let _ = columns.with(|c| {
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
                )
            });
            let blur_header = Rc::clone(&blur_header);
            let blur_press = Rc::clone(&blur_press);
            let blur_menu = Rc::clone(&blur_menu);
            let blur_select = Rc::clone(&blur_select);
            let controls = controls.clone();
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
                        let mut built = flow_table_columns(shown, &current_widths);
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
                    .movable_columns(false)
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
                        flow_header(cell, columns).on_press(move || blur_header())
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
    menu()
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(4.0))
        .margin(px(8.0))
        .trigger(column_picker_trigger())
        .child(column_picker_panel(columns, controls))
}

fn column_picker_trigger() -> impl IntoElement {
    view()
        .w(px(30.0))
        .h(px(30.0))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(border_control())
        .bg(bg_control())
        .hover(|style| style.bg(bg_control_hover()))
        .cursor(Cursor::Pointer)
        .tooltip(label_tip("列", "Columns"))
        .child(glyph(|| Glyph::Columns, || color(fg_secondary())))
}

fn column_picker_panel(columns: Signal<ListColumns>, controls: Arc<Controls>) -> impl IntoElement {
    let toggle_controls = controls.clone();
    view()
        .w(px(176.0))
        .p_1()
        .gap_0p5()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
        .children(ListColumn::ORDERED.into_iter().map(move |column| {
            let (zh, en) = column.labels();
            column_toggle(columns, toggle_controls.clone(), column, zh, en)
        }))
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
        ))
}

fn column_full_url_choice(
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
    full_url: bool,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    let marked = columns;
    marked_action(
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
    marked_action(
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

fn column_toggle(
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
    column: ListColumn,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    let marked = columns;
    marked_action(
        move || marked.get().on(column),
        tr(zh, en),
        move || {
            let next = columns.get().toggle(column);
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

fn row_context_menu(
    picked: Signal<Option<u64>>,
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
    let replay_controls = controls;
    view()
        .w(px(196.0))
        .p_1()
        .gap_0p5()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
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
        .child(menu_rule())
        .child(action_item(
            tr("重放", "Replay"),
            closing(menu_open, move || {
                replay_flow(&replay_controls, picked.get(), ReplayOverride::default());
            }),
        ))
        .child(action_item(
            tr("填入断点", "Add break"),
            closing(menu_open, move || {
                fill_break_rule(&break_controls, picked.get(), rules_text);
            }),
        ))
}

fn closing(open: Signal<bool>, run: impl Fn() + 'static) -> impl Fn() + 'static {
    move || {
        open.set(false);
        run();
    }
}

fn quick_kind(kind: u8) -> crate::rules::QuickRule {
    use crate::rules::QuickRule;
    match kind % 8 {
        0 => QuickRule::Replace,
        1 => QuickRule::Remote,
        2 => QuickRule::Local,
        3 => QuickRule::Delay,
        4 => QuickRule::Break,
        5 => QuickRule::BreakRes,
        6 => QuickRule::Header,
        _ => QuickRule::Cors,
    }
}

fn fill_break_rule(controls: &Controls, id: Option<u64>, rules_text: Signal<String>) {
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

fn flow_pinned(controls: &Controls, id: Option<u64>) -> bool {
    id.is_some_and(|id| {
        controls
            .hub
            .with_session(|session| session.get(id).is_some_and(|flow| flow.pinned))
    })
}

fn flow_cell(
    rows: Signal<Vec<RowLine>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    columns: Signal<ListColumns>,
    hover: CellHover,
    cell: &DataTableCell,
) -> AnyElement {
    let index = cell.row as usize;
    match cell.key.as_ref() {
        "id" => id_cell(rows, selected, baseline, index).into_element(),
        "method" => method_cell(rows, selected, baseline, columns, index).into_element(),
        "path" => path_cell(rows, selected, baseline, columns, hover, index).into_element(),
        "status" => status_text(rows, index).into_element(),
        "protocol" => protocol_cell(rows, index).into_element(),
        "gap" => time_cell(rows, hover, index).into_element(),
        "elapsed" => elapsed_cell(rows, index).into_element(),
        "host" => host_cell(rows, columns, hover, index).into_element(),
        "client_ip" => client_ip_cell(rows, columns, hover, index).into_element(),
        "size" => size_cell(rows, index).into_element(),
        _ => view().into_element(),
    }
}

fn flow_header(cell: &DataTableHeaderCell, columns: Signal<ListColumns>) -> View {
    let key = cell.key.to_string();
    let align_right = matches!(key.as_str(), "gap" | "elapsed" | "size");
    let pad_left = match key.as_str() {
        "method" => 14.0,
        "id" => 6.0,
        _ => 4.0,
    };
    let row = view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(px(pad_left))
        .pr(px(if align_right { 6.0 } else { 4.0 }))
        .flex_row()
        .items_center()
        .overflow_hidden();
    let row = if align_right { row.justify_end() } else { row };
    row.child(
        text(move || -> SharedString { flow_column_title(&key, columns.get().full_url).into() })
            .number_of_lines(1)
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(fg_muted()),
    )
}

fn row_edge_mark(
    rows: Signal<Vec<RowLine>>,
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
    rows: Signal<Vec<RowLine>>,
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
        .pr(px(2.0))
        .border_l(px(2.0))
        .border_l_color(move || {
            if columns.get().id {
                AdaptiveColor::TRANSPARENT
            } else {
                row_edge_mark(rows, selected, baseline, index)
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
                            .rounded(px(99.0))
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
    rows: Signal<Vec<RowLine>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    index: usize,
) -> impl IntoElement {
    view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(px(4.0))
        .pr(px(4.0))
        .border_l(px(2.0))
        .border_l_color(move || row_edge_mark(rows, selected, baseline, index))
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
            .number_of_lines(1)
            .text_size(px(12.0))
            .text_color(fg_muted()),
        )
}

fn status_text(rows: Signal<Vec<RowLine>>, index: usize) -> Text {
    mono(move || {
        rows.with(|rows| {
            rows.get(index)
                .map(|row| list_status(row.status, row.failed))
                .unwrap_or_default()
        })
    })
    .pl(px(4.0))
    .pr(px(4.0))
    .number_of_lines(1)
    .text_size(px(12.0))
    .font_weight(FontWeight::SEMIBOLD)
    .text_color(move || {
        let row = rows.with(|rows| rows.get(index).cloned());
        match row {
            Some(row) => http_status_fg(row.status, row.failed),
            None => fg_muted(),
        }
    })
}

fn time_cell(rows: Signal<Vec<RowLine>>, hover: CellHover, index: usize) -> impl IntoElement {
    let clock_rows = rows;
    view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(px(4.0))
        .pr(px(6.0))
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
            .number_of_lines(1)
            .text_size(px(12.0))
            .text_color(fg_muted()),
        )
}

fn elapsed_cell(rows: Signal<Vec<RowLine>>, index: usize) -> impl IntoElement {
    view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(px(4.0))
        .pr(px(6.0))
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
            .number_of_lines(1)
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
    rows: Signal<Vec<RowLine>>,
    columns: Signal<ListColumns>,
    hover: CellHover,
    index: usize,
) -> impl IntoElement {
    let tip_rows = rows;
    view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(px(4.0))
        .pr(px(4.0))
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
    rows: Signal<Vec<RowLine>>,
    columns: Signal<ListColumns>,
    hover: CellHover,
    index: usize,
) -> impl IntoElement {
    let tip_rows = rows;
    view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(px(4.0))
        .pr(px(4.0))
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

fn protocol_cell(rows: Signal<Vec<RowLine>>, index: usize) -> Text {
    mono(move || {
        rows.with(|rows| {
            rows.get(index)
                .map(|row| row.protocol.clone())
                .unwrap_or_default()
        })
    })
    .pl(px(4.0))
    .pr(px(4.0))
    .number_of_lines(1)
    .text_size(px(12.0))
    .text_color(fg_secondary())
}

fn size_cell(rows: Signal<Vec<RowLine>>, index: usize) -> impl IntoElement {
    view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(px(4.0))
        .pr(px(6.0))
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
            .number_of_lines(1)
            .text_size(px(12.0))
            .text_color(fg_muted()),
        )
}

fn path_cell(
    rows: Signal<Vec<RowLine>>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    columns: Signal<ListColumns>,
    hover: CellHover,
    index: usize,
) -> impl IntoElement {
    let tip_rows = rows;
    view()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .pl(px(4.0))
        .pr(px(4.0))
        .border_l(move || {
            let cols = columns.get();
            if !cols.id && !cols.method {
                px(2.0)
            } else {
                px(0.0)
            }
        })
        .border_l_color(move || row_edge_mark(rows, selected, baseline, index))
        .overflow_hidden()
        .flex_row()
        .items_center()
        .child(
            mono(move || {
                let full_url = columns.get().full_url;
                rows.with(|rows| {
                    rows.get(index)
                        .map(|row| row_target_text(row, full_url).to_string())
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
                    .map(|row| row_target_text(row, full_url).to_string())
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
        .max_w(px(420.0))
        .px_2p5()
        .py_1p5()
        .rounded(px(6.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
        .child(
            mono(full)
                .max_w(px(396.0))
                .text_size(px(12.0))
                .text_color(fg_primary()),
        )
}

fn detail_pane(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    compose: Compose,
    filter: Signal<String>,
    baseline: Signal<Option<u64>>,
    folded: Signal<u8>,
    pane_px: Rc<Cell<f32>>,
) -> impl IntoElement {
    scroll()
        .flex_1()
        .min_w(px(0.0))
        .bg(bg_canvas())
        .on_layout(move |bounds: &Bounds<Points>| {
            let width = bounds.size.width.0;
            pane_px.set(width);
            if hex_cols_for_pane(width) != hex_cols_for_pane(shown.pane_width.get_untracked()) {
                shown.pane_width.set(width);
            }
        })
        .child(
            when(
                move || shown.phase.get() == Phase::Ready,
                move || {
                    ready_detail(
                        shown,
                        controls.clone(),
                        selected,
                        compose,
                        filter,
                        baseline,
                        folded,
                    )
                },
            )
            .otherwise(move || {
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
                                match shown.phase.get() {
                                    Phase::Gone => {
                                        t(lang, "这条记录已经不在了", "That flow is gone").into()
                                    }
                                    _ => t(lang, "还没有选择", "Nothing selected").into(),
                                }
                            }
                        })
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
                        .text_size(px(12.0))
                        .text_color(fg_muted()),
                    )
            }),
        )
}

fn ready_detail(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    compose: Compose,
    filter: Signal<String>,
    baseline: Signal<Option<u64>>,
    folded: Signal<u8>,
) -> impl IntoElement {
    let no_form = signal(Vec::<HeaderLine>::new());
    view()
        .p_3p5()
        .gap_3p5()
        .child(summary(
            shown,
            controls.clone(),
            selected,
            compose,
            filter,
            baseline,
        ))
        .child(when(move || compose.composing.get(), {
            let controls = controls.clone();
            move || compose_box(compose, controls.clone(), selected)
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
            folded,
            FOLD_RES_HEADERS,
        ))
        .child(when(
            move || {
                !shown.request_body.get().is_empty()
                    || !shown.request_note.get().is_empty()
                    || !shown.request_form.with(Vec::is_empty)
            },
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
                    false,
                    shown,
                    controls.clone(),
                    selected,
                    baseline,
                    folded,
                    FOLD_REQ_BODY,
                )
            },
        ))
        .child(when(
            move || !shown.query.with(Vec::is_empty),
            move || header_section("查询参数", "Query", shown.query, folded, FOLD_QUERY),
        ))
        .child(header_section(
            "请求头",
            "Request headers",
            shown.request_headers,
            folded,
            FOLD_REQ_HEADERS,
        ))
        .child(when(
            move || !shown.diff.get().is_empty(),
            move || diff_card(shown, folded),
        ))
        .child(when(
            move || !shown.messages.with(Vec::is_empty),
            move || header_section("消息", "Messages", shown.messages, folded, FOLD_MESSAGES),
        ))
}

fn summary(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    compose: Compose,
    filter: Signal<String>,
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
                            .px_2()
                            .py_0p5()
                            .rounded(px(5.0))
                            .bg(accent_blue_soft())
                            .child(
                                mono(move || shown.rules.get())
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(accent_blue_text()),
                            )
                    },
                ))
                .child(view().flex_1())
                .child(detail_actions(
                    shown, controls, selected, compose, filter, baseline,
                )),
        )
        .child(
            view()
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
        .child(
            mono(move || shown.metrics.get())
                .selectable(true)
                .on_press_up(dismiss_keyboard)
                .number_of_lines(1)
                .text_overflow(EllipsizeMode::Tail)
                .text_size(px(11.5))
                .text_color(fg_muted()),
        )
        .child(when(
            move || !shown.error.get().is_empty(),
            move || {
                view()
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

fn header_section(
    zh: &'static str,
    en: &'static str,
    lines: Signal<Vec<HeaderLine>>,
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
                                .text_size(px(10.5))
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
                        |line, index| {
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
                                        .w(px(176.0))
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
                                        .selectable(true)
                                        .text_size(px(12.0))
                                        .text_color(fg_primary()),
                                )
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
    embed_image: bool,
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    folded: Signal<u8>,
    bit: u8,
) -> impl IntoElement {
    let image_on = move || embed_image && shown.pretty.get() && !shown.preview.get().is_empty();
    let doc_on = move || shown.doc_preview.get() && doc.with(Option::is_some);
    view()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .overflow_hidden()
        .child(
            view()
                .flex_row()
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
                        .min_w(px(0.0))
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .cursor(Cursor::Pointer)
                        .on_press(move || toggle_card(folded, bit))
                        .child(fold_mark(folded, bit))
                        .child(section_title(tr(zh, en))),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .child(when(
                            move || doc.with(Option::is_some),
                            move || doc_preview_chip(shown),
                        ))
                        .child(pretty_switch(shown, controls, selected, baseline)),
                ),
        )
        .child(when(
            move || !card_folded(folded, bit) && !note.get().is_empty(),
            move || {
                view().px_3().pt_3().child(
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
                    .child(scroll().size_full().px_3p5().py_2p5().child(doc_body(doc)))
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
                        let lines = if height_hex.with(Vec::is_empty) {
                            body_line_count(&height_body.get())
                        } else {
                            let count = height_hex.with(Vec::len);
                            let cols = hex_cols_for_pane(shown.pane_width.get()).max(1);
                            count.div_ceil(cols).max(1)
                        };
                        px(body_box_height(lines))
                    })
                    .overflow_hidden()
                    .border_t(move || if divided { px(1.0) } else { px(0.0) })
                    .border_t_color(border_subtle())
                    .child(
                        scroll().size_full().child(
                            mono(move || {
                                let bytes = hex.with(|bytes| bytes.clone());
                                if bytes.is_empty() {
                                    body.get()
                                } else {
                                    let cols = hex_cols_for_pane(shown.pane_width.get());
                                    hex_display_rows(&bytes, cols)
                                }
                            })
                            .px_3p5()
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
                    .child(code_body(body, code))
            },
        ))
        .child(when(
            move || !card_folded(folded, bit) && !doc_on() && !form.with(Vec::is_empty),
            move || {
                each(
                    form,
                    |line| line.key,
                    |line, index| {
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
                                    .w(px(176.0))
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
                                    .selectable(true)
                                    .text_size(px(12.0))
                                    .text_color(fg_primary()),
                            )
                    },
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

fn code_body(body: Signal<String>, code: Signal<String>) -> impl IntoElement {
    view()
        .w_full()
        .h(move || px(editor_box_height(&body.get())))
        .bg(bg_code())
        .child(
            editor()
                .value(move || SharedString::from(body.get()))
                .read_only(true)
                .language(move || SharedString::from(code.get()))
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

fn doc_preview_chip(shown: Shown) -> impl IntoElement {
    view()
        .h(px(22.0))
        .px_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || {
            if shown.doc_preview.get() {
                accent_blue()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if shown.doc_preview.get() {
                accent_blue()
            } else {
                bg_card()
            }
        })
        .flex_row()
        .items_center()
        .cursor(Cursor::Pointer)
        .on_press(move || {
            shown.doc_preview.set(!shown.doc_preview.get());
        })
        .child(
            text(tr("预览", "Preview"))
                .number_of_lines(1)
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if shown.doc_preview.get() {
                        AdaptiveColor::rgb(0xffffff, 0xffffff)
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

fn pretty_switch(
    shown: Shown,
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
            controls.clone(),
            selected,
            baseline,
            true,
            "美化",
            "Pretty",
        ))
        .child(pretty_chip(
            shown, controls, selected, baseline, false, "原文", "Raw",
        ))
}

fn pretty_chip(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    on: bool,
    zh: &'static str,
    en: &'static str,
) -> impl IntoElement {
    let langs = ui_langs();
    view()
        .h(px(22.0))
        .px_2()
        .flex_row()
        .items_center()
        .border_r(move || if on { px(1.0) } else { px(0.0) })
        .border_r_color(border_control())
        .bg(move || {
            if !shown.doc_preview.get() && shown.pretty.get() == on {
                accent_blue()
            } else {
                bg_card()
            }
        })
        .cursor(Cursor::Pointer)
        .on_press(move || {
            let was_preview = shown.doc_preview.get();
            if was_preview {
                shown.doc_preview.set(false);
            }
            if !was_preview && shown.pretty.get() == on {
                return;
            }
            shown.pretty.set(on);
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
                    if !shown.doc_preview.get() && shown.pretty.get() == on {
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
    compose: Compose,
    filter: Signal<String>,
    baseline: Signal<Option<u64>>,
) -> impl IntoElement {
    let copy_url = controls.clone();
    let copy_curl = controls.clone();
    let copy_fetch = controls.clone();
    let copy_python = controls.clone();
    let copy_types = controls.clone();
    let copy_request = controls.clone();
    let copy_response = controls.clone();
    let save = controls.clone();
    let focus_controls = controls.clone();
    let mute_controls = controls.clone();
    let pin_controls = controls.clone();
    let base_controls = controls.clone();
    let replay_controls = controls.clone();
    let edit_controls = controls.clone();
    let send_controls = controls.clone();
    let pretty_controls = controls;
    let pinned = shown.pinned;
    let pretty = shown.pretty;
    let baseline_mark = baseline;
    let selected_mark = selected;
    menu()
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(8.0))
        .trigger(detail_more_trigger())
        .child(
            view()
                .w(px(196.0))
                .p_1()
                .gap_0p5()
                .rounded(px(10.0))
                .border_1()
                .border_color(border_strong())
                .bg(bg_card())
                .shadow_md()
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
                .child(action_item(tr("类型", "Types"), move || {
                    copy_snippet(&copy_types, selected.get(), Snippet::Types);
                }))
                .child(action_item(tr("复制请求", "Copy request"), move || {
                    copy_part(&copy_request, selected.get(), CopyPart::Request);
                }))
                .child(action_item(
                    tr("复制响应", "Copy response"),
                    move || {
                        copy_part(&copy_response, selected.get(), CopyPart::Response);
                    },
                ))
                .child(action_item(tr("另存", "Save"), move || {
                    export_response(save.clone(), selected.get());
                }))
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
                .child(marked_action(
                    move || pretty.get(),
                    {
                        let langs = ui_langs();
                        move || -> SharedString {
                            if pretty.get() {
                                t(ui_lang(langs), "原文", "Raw").into()
                            } else {
                                t(ui_lang(langs), "美化", "Pretty").into()
                            }
                        }
                    },
                    {
                        let langs = ui_langs();
                        move || {
                            shown.pretty.set(!shown.pretty.get());
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
                .child(action_item(tr("重放", "Replay"), move || {
                    replay_flow(&replay_controls, selected.get(), ReplayOverride::default());
                }))
                .child(action_item(tr("编辑重发", "Edit"), move || {
                    begin_compose(&edit_controls, selected.get(), compose);
                }))
                .child(when(
                    move || compose.composing.get() && !compose.response.get(),
                    move || {
                        let controls = send_controls.clone();
                        action_item(tr("发送", "Send"), move || {
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
                        })
                    },
                )),
        )
}

fn detail_more_trigger() -> impl IntoElement {
    view()
        .h(px(26.0))
        .px_2()
        .gap_1()
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
        .child(
            text(tr("更多", "More"))
                .number_of_lines(1)
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg_secondary()),
        )
        .child(glyph(|| Glyph::Chevron, || color(fg_secondary())))
}

fn action_item(
    label: impl IntoReactive<SharedString>,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    menu_item()
        .on_select(on_select)
        .h(px(30.0))
        .px_2p5()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .hover(|style| style.bg(bg_control_hover()))
        .active(|style| style.opacity(0.75))
        .cursor(Cursor::Pointer)
        .child(control_text(label).text_color(fg_secondary()))
}

fn marked_action(
    marked: impl Fn() -> bool + 'static + Clone,
    label: impl IntoReactive<SharedString>,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    let dot = marked.clone();
    let dot_fill = marked.clone();
    let label_on = marked;
    menu_item()
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
                .rounded(px(99.0))
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
        .px_3()
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
                .child(
                    view()
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded(px(99.0))
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
                    mono({
                        let langs = ui_langs();
                        move || {
                            let lang = ui_lang(langs);
                            if listen.get().is_empty() {
                                t(lang, "监听尚未开始", "Not listening").into()
                            } else {
                                format!("{} {}", t(lang, "监听", "Listening"), listen.get())
                            }
                        }
                    })
                    .number_of_lines(1)
                    .text_size(px(11.5))
                    .text_color(fg_muted()),
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
            mono({
                let langs = ui_langs();
                move || {
                    format!(
                        "{} · {} {}",
                        memory.get(),
                        t(ui_lang(langs), "已丢弃", "dropped"),
                        dropped.get()
                    )
                }
            })
            .flex_shrink_0()
            .text_size(px(11.5))
            .text_color(fg_muted()),
        )
}

fn cert_qr(
    listen: Signal<String>,
    lan_url: Signal<String>,
    lan_choice: Signal<String>,
    lan_addrs: Signal<Vec<LanAddr>>,
    qr_path: Signal<String>,
) -> impl IntoElement {
    popover()
        .placement(Placement::Top)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(8.0))
        .trigger(qr_trigger(lan_choice))
        .child(qr_panel(listen, lan_url, lan_choice, lan_addrs, qr_path))
}

fn qr_trigger(choice: Signal<String>) -> impl IntoElement {
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
        .bg(bg_control())
        .hover(|style| style.bg(bg_control_hover()))
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.75))
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded(px(99.0))
                .bg(accent_blue()),
        )
        .child(
            text({
                let langs = ui_langs();
                move || -> SharedString {
                    format!("{} {}", t(ui_lang(langs), "扫码", "Scan"), choice.get()).into()
                }
            })
            .number_of_lines(1)
            .text_size(px(11.5))
            .text_color(fg_secondary()),
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
        .w(px(300.0))
        .p_3()
        .gap_2()
        .items_center()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
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
        .cursor(Cursor::Pointer)
        .on_press(move || {
            let ip = addr.with(|item| item.ip.to_string());
            publish_lan(&listen.get(), &ip, lan_addrs, lan_choice, lan_url, qr_path);
        })
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded(px(99.0))
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

fn toolbar_menu(
    installed: Signal<bool>,
    proxy_on: Signal<bool>,
    install: Rc<impl Fn() + 'static>,
    remove: Rc<impl Fn() + 'static>,
    toggle_proxy: impl Fn() + 'static,
    export_har: impl Fn() + 'static,
    copy_env: impl Fn() + 'static,
    clear: impl Fn() + 'static,
    import_controls: Arc<Controls>,
    rows: Signal<Vec<RowLine>>,
    flow_total: Signal<usize>,
    filter: Signal<String>,
    pretty: Signal<bool>,
    share_id: Signal<String>,
) -> impl IntoElement {
    menu()
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(8.0))
        .trigger(more_trigger(installed, proxy_on))
        .child(more_menu(
            installed,
            proxy_on,
            install,
            remove,
            toggle_proxy,
            export_har,
            copy_env,
            clear,
            import_controls,
            rows,
            flow_total,
            filter,
            pretty,
            share_id,
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
    rows: Signal<Vec<RowLine>>,
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
    rows: Signal<Vec<RowLine>>,
    flow_total: Signal<usize>,
    filter: Signal<String>,
) -> impl IntoElement {
    view()
        .h(px(CONTROL_H))
        .px_2p5()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(CONTROL_RADIUS))
        .bg(bg_control())
        .border_1()
        .border_color(border_control())
        .child(
            mono(move || SharedString::from(flow_count_now(rows, flow_total, filter)))
                .number_of_lines(1)
                .text_size(px(CONTROL_TEXT))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_secondary()),
        )
}

fn more_trigger(installed: Signal<bool>, proxy_on: Signal<bool>) -> impl IntoElement {
    let dot_installed = installed;
    let dot_proxy = proxy_on;
    let label_proxy = proxy_on;
    view()
        .h(px(CONTROL_H))
        .px_2p5()
        .gap_1p5()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(move || {
            if proxy_on.get() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if proxy_on.get() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if proxy_on.get() {
                    accent_blue_soft()
                } else {
                    bg_control_hover()
                }
            })
        })
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.75))
        .child(when(
            move || dot_proxy.get() || !dot_installed.get(),
            move || {
                view().w(px(6.0)).h(px(6.0)).rounded(px(99.0)).bg(move || {
                    if dot_proxy.get() {
                        accent_blue()
                    } else {
                        status_warn_fg()
                    }
                })
            },
        ))
        .child(control_text(tr("更多", "More")).text_color(move || {
            if label_proxy.get() {
                accent_blue_text()
            } else {
                fg_secondary()
            }
        }))
        .child(glyph(
            || Glyph::Chevron,
            move || {
                color(if label_proxy.get() {
                    accent_blue_text()
                } else {
                    fg_secondary()
                })
            },
        ))
}

fn more_menu(
    installed: Signal<bool>,
    proxy_on: Signal<bool>,
    install: Rc<impl Fn() + 'static>,
    remove: Rc<impl Fn() + 'static>,
    toggle_proxy: impl Fn() + 'static,
    export_har: impl Fn() + 'static,
    copy_env: impl Fn() + 'static,
    clear: impl Fn() + 'static,
    import_controls: Arc<Controls>,
    rows: Signal<Vec<RowLine>>,
    flow_total: Signal<usize>,
    filter: Signal<String>,
    pretty: Signal<bool>,
    share_id: Signal<String>,
) -> impl IntoElement {
    let export_har = Rc::new(export_har);
    let export_all = Rc::clone(&export_har);
    let export_filtered = Rc::clone(&export_har);
    view()
        .w(px(280.0))
        .p_1()
        .gap_0p5()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
        .child(menu_action_meta(
            Glyph::Trash,
            tr("清空", "Clear"),
            move || SharedString::from(flow_count_now(rows, flow_total, filter)),
            clear,
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
        })
        .child(menu_action(Glyph::List, tr("导入 HAR", "Import HAR"), {
            let controls = import_controls.clone();
            move || prompt_import_har(controls.clone())
        }))
        .child(menu_action(
            Glyph::Prompt,
            tr("复制代理环境", "Copy proxy env"),
            copy_env,
        ))
        .child(menu_rule())
        .child(proxy_menu_item(proxy_on, toggle_proxy))
        .child(menu_rule())
        .child(cert_menu_status(installed))
        .child(
            when(
                move || installed.get(),
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
            }),
        )
        .child(menu_rule())
        .child(marked_action(
            move || pretty.get(),
            tr("默认美化", "Pretty by default"),
            move || {
                let next = !pretty.get();
                pretty.set(next);
                if let Err(err) = save_config(|config| config.default_pretty = next) {
                    set_status(
                        &import_controls,
                        StatusMsg::PrefUnsaved {
                            name: PrefName::Pretty,
                            err: err.to_string(),
                        },
                    );
                }
            },
        ))
        .child(lang_choice(None, "跟随系统", "Follow system"))
        .child(lang_choice(Some(Lang::Zh), "中文", "中文"))
        .child(lang_choice(Some(Lang::En), "English", "English"))
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
                .flex_shrink_0()
                .number_of_lines(1)
                .text_size(px(11.0))
                .text_color(fg_muted()),
        )
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
                let paint = color(if mark.get() == choice {
                    accent_blue()
                } else {
                    bg_card()
                });
                Painter::new()
                    .fill_color(paint)
                    .fill_svg("M6.5 6.5 m-3 0 a3 3 0 1 0 6 0 a3 3 0 1 0 -6 0")
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
        .gap_1p5()
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
                    accent_blue_soft()
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
        .child(view().w(px(6.0)).h(px(6.0)).rounded(px(99.0)).bg(move || {
            if dot_on.get() {
                accent_blue()
            } else {
                fg_muted()
            }
        }))
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
        )
}

fn cert_menu_status(installed: Signal<bool>) -> impl IntoElement {
    let dot_installed = installed;
    view()
        .h(px(28.0))
        .px_2p5()
        .gap_1p5()
        .flex_row()
        .items_center()
        .child(glyph(|| Glyph::Badge, || color(fg_muted())))
        .child(view().w(px(6.0)).h(px(6.0)).rounded(px(99.0)).bg(move || {
            if dot_installed.get() {
                status_ok_fg()
            } else {
                status_warn_fg()
            }
        }))
        .child(
            control_text({
                let langs = ui_langs();
                move || -> SharedString {
                    if installed.get() {
                        t(ui_lang(langs), "证书已安装", "Certificate installed").into()
                    } else {
                        t(ui_lang(langs), "证书未安装", "Certificate missing").into()
                    }
                }
            })
            .text_color(fg_muted()),
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

fn row_lines(hub: &Hub, filter: &str, lang: Lang) -> Vec<RowLine> {
    hub.with_session(|session| {
        session
            .matching_with_previous_start(filter)
            .into_iter()
            .filter_map(|(id, previous)| {
                let flow = session.get(id)?;
                let failed = flow.error.is_some() && flow.status.is_none();
                Some(RowLine {
                    id,
                    method: flow.method.clone(),
                    status: flow.status,
                    time: present::format_gap(previous, flow.started),
                    clock: present::format_clock(flow.started),
                    elapsed: present::format_elapsed(flow.started, flow.finished_at),
                    elapsed_slow: flow.finished_at.is_some()
                        && present::is_slow(
                            flow.started,
                            flow.finished_at,
                            std::time::SystemTime::now(),
                        ),
                    host: flow.host.clone(),
                    client_ip: if flow.client_ip.is_empty() {
                        "—".to_string()
                    } else {
                        flow.client_ip.clone()
                    },
                    path: list_path(&flow.path, flow.error.as_deref(), failed, lang),
                    url: list_url(&flow.url, &flow.path, flow.error.as_deref(), failed, lang),
                    protocol: flow.protocol.label_in(lang).to_string(),
                    size: format_bytes(flow.response_bytes as usize),
                    failed,
                    pinned: flow.pinned,
                })
            })
            .collect()
    })
}

fn publish_detail(shown: &Shown, hub: &Hub, id: Option<u64>, lang: Lang, baseline: Option<u64>) {
    let Some(id) = id else {
        shown.phase.set(Phase::Empty);
        shown.request_hex.set(Vec::new());
        shown.response_hex.set(Vec::new());
        shown.preview.set(String::new());
        shown.preview_w.set(0.0);
        shown.preview_h.set(0.0);
        shown.image_caption.set(String::new());
        shown.diff.set(String::new());
        shown.diff_old.set(String::new());
        shown.diff_new.set(String::new());
        shown.request_language.set(String::new());
        shown.response_language.set(String::new());
        shown.request_doc.set(None);
        shown.response_doc.set(None);
        shown.request_form.set(Vec::new());
        shown.rules.set(String::new());
        shown.pinned.set(false);
        return;
    };
    let snapshot = hub.with_session(|session| session.hydrated(id));
    let Some(flow) = snapshot else {
        shown.phase.set(Phase::Gone);
        shown.request_hex.set(Vec::new());
        shown.response_hex.set(Vec::new());
        shown.preview.set(String::new());
        shown.preview_w.set(0.0);
        shown.preview_h.set(0.0);
        shown.image_caption.set(String::new());
        shown.diff.set(String::new());
        shown.diff_old.set(String::new());
        shown.diff_new.set(String::new());
        shown.request_language.set(String::new());
        shown.response_language.set(String::new());
        shown.request_doc.set(None);
        shown.response_doc.set(None);
        shown.request_form.set(Vec::new());
        shown.rules.set(String::new());
        shown.pinned.set(false);
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
    );
    let response = display_body(
        &flow.response_body,
        flow.response_truncated,
        flow.response_expired,
        flow.response_encoding_error.as_deref(),
        response_type,
        pretty,
        lang,
    );
    let response_hex = response.hex;
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
    shown.query.set(
        present::query_pairs(&flow.url)
            .into_iter()
            .enumerate()
            .map(|(index, (name, value))| HeaderLine {
                key: index as u32,
                name: name.clone(),
                value: annotated(&name, &value),
            })
            .collect(),
    );
    shown.method.set(flow.method);
    shown.url.set(flow.url);
    shown.protocol.set(flow.protocol.label_in(lang).to_string());
    shown.rules.set(flow.rule_hit.clone());
    shown.status.set(flow.status);
    shown.error.set(flow.error.clone().unwrap_or_default());
    shown
        .request_headers
        .set(header_lines(&flow.request_headers, pretty));
    shown
        .response_headers
        .set(header_lines(&flow.response_headers, pretty));
    let form = if pretty {
        std::str::from_utf8(&flow.request_body)
            .ok()
            .filter(|text| present::is_form(request_type, text))
            .map(|text| {
                present::form_pairs(text)
                    .into_iter()
                    .enumerate()
                    .map(|(index, (name, value))| HeaderLine {
                        key: index as u32,
                        name: name.clone(),
                        value: annotated(&name, &value),
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
                }
            })
            .collect::<Vec<_>>();
        if flow.messages_truncated {
            let key = lines.len() as u32;
            lines.push(HeaderLine {
                key,
                name: "…".into(),
                value: t(lang, "消息已截断", "Messages truncated").into(),
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
    present::sse_events(text)
        .into_iter()
        .enumerate()
        .map(|(index, event)| HeaderLine {
            key: index as u32,
            name: event.event,
            value: event.data,
        })
        .collect()
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
                let key = lines.len() as u32;
                lines.push(HeaderLine {
                    key,
                    name: row_name,
                    value: shown,
                });
            }
            continue;
        }
        let key = lines.len() as u32;
        lines.push(HeaderLine {
            key,
            name: name.clone(),
            value: annotated(name, value),
        });
    }
    lines
}

fn annotated(name: &str, value: &str) -> String {
    match present::annotate_value(name, value) {
        Some(hint) => format!("{value}\n{hint}"),
        None => value.to_string(),
    }
}

struct BodyText {
    text: String,
    note: String,
    /// Set for a binary body. The view breaks it into 16 or 32 byte rows.
    hex: Vec<u8>,
}

fn display_body(
    body: &[u8],
    truncated: bool,
    expired: Option<usize>,
    encoding_error: Option<&str>,
    content_type: Option<&str>,
    pretty: bool,
    lang: Lang,
) -> BodyText {
    let mut presented = body_text(body, truncated, expired, encoding_error, lang);
    if pretty && std::str::from_utf8(body).is_ok() {
        presented.hex.clear();
    }
    if pretty {
        apply_pretty(&mut presented, body, content_type, lang);
    }
    presented
}

fn apply_pretty(presented: &mut BodyText, body: &[u8], content_type: Option<&str>, lang: Lang) {
    let Ok(text) = std::str::from_utf8(body) else {
        return;
    };
    let pretty = present::pretty_body(content_type, text);
    const DISPLAY_CAP: usize = 256 * 1024;
    if pretty.len() > DISPLAY_CAP {
        let end = pretty.floor_char_boundary(DISPLAY_CAP);
        presented.text = pretty[..end].to_string();
        let marker = t(lang, "界面只显示", "Showing only");
        if !presented.note.contains(marker) {
            if !presented.note.is_empty() {
                presented.note.push('\n');
            }
            presented.note.push_str(t(
                lang,
                "界面只显示前 256KB。",
                "Showing only the first 256 KB.",
            ));
        }
    } else {
        presented.text = pretty;
    }
}

fn body_text(
    body: &[u8],
    truncated: bool,
    expired: Option<usize>,
    encoding_error: Option<&str>,
    lang: Lang,
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
    let text = if body.is_empty() {
        String::new()
    } else if let Ok(text) = std::str::from_utf8(body) {
        const DISPLAY_CAP: usize = 256 * 1024;
        if text.len() > DISPLAY_CAP {
            if !note.is_empty() {
                note.push('\n');
            }
            let end = text.floor_char_boundary(DISPLAY_CAP);
            let rest = text.len() - end;
            note.push_str(&match lang {
                Lang::Zh => format!("界面只显示前 256KB，记录里还有 {rest} 字节。"),
                Lang::En => format!("Showing only the first 256 KB; {rest} more bytes are stored."),
            });
            text[..end].to_string()
        } else {
            text.to_string()
        }
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
    BodyText { text, note, hex }
}

fn open_controls() -> Arc<Controls> {
    match Ca::open_default().and_then(|ca| Hub::new(Arc::new(ca))) {
        Ok(hub) => {
            apply_saved_prefs(&hub);
            Arc::new(Controls {
                hub: Arc::new(hub),
                capture: Mutex::new(None),
                status: Mutex::new(StatusMsg::Idle),
                running: Mutex::new(false),
                listen: Mutex::new(String::new()),
                listen_port: Mutex::new(load_listen_port()),
                ca_installed: Mutex::new(ca::is_installed()),
                use_system_proxy: Mutex::new(load_use_system_proxy()),
                proxy_applied: Mutex::new(false),
                ops: Mutex::new(()),
                listen_all: Mutex::new(load_listen_all()),
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
                ca_installed: Mutex::new(false),
                use_system_proxy: Mutex::new(load_use_system_proxy()),
                proxy_applied: Mutex::new(false),
                ops: Mutex::new(()),
                listen_all: Mutex::new(load_listen_all()),
            })
        }
    }
}

fn install_ca(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let ca = controls.hub.ca.clone();
        match ca::install(&ca) {
            Ok(_) => set_ca_installed(&controls, true),
            Err(err) => {
                set_ca_installed(&controls, ca::is_installed());
                set_status(&controls, StatusMsg::InstallFailed(format!("{err:#}")));
            }
        }
    });
}

fn remove_ca(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let ca = controls.hub.ca.clone();
        match ca::remove(&ca) {
            Ok(_) => set_ca_installed(&controls, ca::is_installed()),
            Err(err) => {
                set_ca_installed(&controls, ca::is_installed());
                set_status(&controls, StatusMsg::RemoveFailed(format!("{err:#}")));
            }
        }
    });
}

fn set_ca_installed(controls: &Controls, installed: bool) {
    *controls
        .ca_installed
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = installed;
}

fn set_status(controls: &Controls, message: StatusMsg) {
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
    controls.hub.end_share();
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

fn saved_config() -> config::Config {
    config::load_mode(&ca::data_dir(), config::is_dev_mode())
}

fn save_config(change: impl FnOnce(&mut config::Config)) -> std::io::Result<()> {
    config::update_mode(&ca::data_dir(), config::is_dev_mode(), change)
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
    Types,
}

fn copy_proxy_env(controls: &Controls) {
    let port = lock_port(&controls.listen_port);
    let cert = controls.hub.ca.cert_path();
    set_clipboard(present::proxy_env(port, &cert.display().to_string()));
    set_status(controls, StatusMsg::CopiedEnv);
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

/// `ready_detail` padding and the card border, between the pane and the hex area.
const HEX_AREA_CHROME: f32 = 14.0 * 2.0 + 2.0;
/// Horizontal padding of the hex text (`px_3p5` on each side).
const HEX_TEXT_PAD: f32 = 28.0;

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

fn export_response(controls: Arc<Controls>, id: Option<u64>) {
    let Some(flow) = selected_flow(&controls, id) else {
        return;
    };
    if flow.response_body.is_empty() {
        set_status(&controls, StatusMsg::EmptyResponse);
        return;
    }
    let ext = present::content_type(&flow.response_headers)
        .and_then(present::image_extension)
        .unwrap_or_else(|| body_extension(&flow.response_headers, &flow.response_body));
    let dialog = nsh_core::file_dialog::SaveDialog::new()
        .file_name(format!("{}.{}", flow.id, ext))
        .filter(nsh_core::file_dialog::FileFilter::new(
            ext.to_ascii_uppercase(),
            [ext],
        ));
    let body = flow.response_body;
    spawn(async move {
        match pick_save_path(dialog).await {
            Ok(Some(path)) => {
                if let Some(parent) = path.parent()
                    && !parent.as_os_str().is_empty()
                    && let Err(err) = std::fs::create_dir_all(parent)
                {
                    set_status(&controls, StatusMsg::SaveFailed(err.to_string()));
                    return;
                }
                if let Err(err) = std::fs::write(&path, &body) {
                    set_status(&controls, StatusMsg::SaveFailed(err.to_string()));
                    return;
                }
                let text = path.display().to_string();
                set_clipboard(text.clone());
                set_status(&controls, StatusMsg::Saved(text));
            }
            Ok(None) => {}
            Err(err) => set_status(&controls, StatusMsg::SaveFailed(err)),
        }
    });
}

/// `XDG_DOWNLOAD_DIR="$HOME/Downloads"` from `user-dirs.dirs`.
#[cfg_attr(not(test), allow(dead_code))]
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
    let dir = ca::data_dir().join("preview");
    let path = dir.join(format!("{id}.{ext}"));
    if let Ok(meta) = std::fs::metadata(&path)
        && meta.len() == flow.response_body.len() as u64
    {
        return path.to_string_lossy().into_owned();
    }
    if std::fs::create_dir_all(&dir).is_err() || std::fs::write(&path, &flow.response_body).is_err()
    {
        return String::new();
    }
    path.to_string_lossy().into_owned()
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
    compose: Compose,
    selected: Signal<Option<u64>>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let release = controls.clone();
    let drop_controls = controls;
    view()
        .h(px(40.0))
        .px_3()
        .gap_2()
        .flex_row()
        .items_center()
        .bg(status_warn_bg())
        .border_b_1()
        .border_b_color(status_warn_fg())
        .child(
            text({
                let langs = ui_langs();
                move || -> SharedString {
                    match paused.get() {
                        Some((id, BreakSide::Response)) => format!(
                            "{} #{id} {}",
                            t(ui_lang(langs), "流量", "Flow"),
                            t(ui_lang(langs), "停在响应断点", "is paused on the response")
                        )
                        .into(),
                        Some((id, BreakSide::Request)) => format!(
                            "{} #{id} {}",
                            t(ui_lang(langs), "流量", "Flow"),
                            t(ui_lang(langs), "停在请求断点", "is paused on the request")
                        )
                        .into(),
                        None => "".into(),
                    }
                }
            })
            .flex_1()
            .number_of_lines(1)
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(status_warn_fg()),
        )
        .child(detail_button(tr("放行", "Continue"), move || {
            let edited = compose.composing.get()
                && paused.get().is_some_and(|(id, side)| {
                    selected.get() == Some(id)
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
            if !release.hub.resolve_break(decision) {
                set_status(&release, StatusMsg::BreakDone);
            }
        }))
        .child(detail_button(tr("丢弃", "Drop"), move || {
            let _ = drop_controls.hub.resolve_break(BreakDecision {
                drop: true,
                headers: None,
                body: None,
                status: None,
            });
        }))
}

fn compose_box(
    compose: Compose,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
) -> impl IntoElement {
    view()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .overflow_hidden()
        .child(
            view()
                .flex_row()
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
                                .h(px(24.0))
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
                                        .text_size(px(11.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(fg_secondary()),
                                ),
                        )
                        .child(when(
                            move || !compose.response.get(),
                            move || {
                                let controls = controls.clone();
                                view()
                                    .h(px(24.0))
                                    .px_2p5()
                                    .rounded(px(6.0))
                                    .bg(accent_blue())
                                    .hover(|style| style.bg(accent_blue_hover()))
                                    .active(|style| style.opacity(0.85))
                                    .cursor(Cursor::Pointer)
                                    .flex_row()
                                    .items_center()
                                    .on_press(move || {
                                        replay_flow(
                                            &controls,
                                            selected.get(),
                                            ReplayOverride {
                                                method: Some(compose.method.get()),
                                                url: Some(compose.url.get()),
                                                headers: Some(parse_header_text(
                                                    &compose.headers.get(),
                                                )),
                                                body: Some(compose.body.get().into_bytes()),
                                            },
                                        );
                                    })
                                    .child(
                                        text(tr("发送", "Send"))
                                            .number_of_lines(1)
                                            .text_size(px(11.5))
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
                                    .child(view().w(px(88.0)).flex_shrink_0().child(compose_input(
                                        compose.status,
                                        "200",
                                        30.0,
                                        false,
                                    )))
                            },
                        ))
                        .child(when(
                            move || !compose.response.get(),
                            move || {
                                view()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .flex_row()
                                    .gap_1p5()
                                    .child(view().w(px(88.0)).flex_shrink_0().child(compose_input(
                                        compose.method,
                                        "POST",
                                        30.0,
                                        false,
                                    )))
                                    .child(view().flex_1().min_w(px(0.0)).child(compose_input_on(
                                        compose.url,
                                        tr(
                                            "https://example.com/path，或粘贴 curl",
                                            "https://example.com/path, or paste curl",
                                        ),
                                        30.0,
                                        false,
                                        move |text| apply_url_edit(compose, text),
                                    )))
                            },
                        )),
                )
                .child(compose_editor(
                    compose.headers,
                    tr("一行一个 Name: value", "One Name: value per line"),
                    120.0,
                    "plaintext",
                ))
                .child(compose_editor(
                    compose.body,
                    tr("body", "body"),
                    160.0,
                    "json",
                )),
        )
}

fn compose_editor(
    value: Signal<String>,
    placeholder: impl Fn() -> String + 'static,
    height: f32,
    language: &'static str,
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
                .on_change(move |text: &SharedString| value.set(text.to_string()))
                .size_full(),
        )
}

fn compose_input(
    value: Signal<String>,
    placeholder: impl IntoReactive<String> + 'static,
    height: f32,
    multiline: bool,
) -> impl IntoElement {
    compose_input_on(value, placeholder, height, multiline, move |text| {
        value.set(text.to_string());
    })
}

fn compose_input_on(
    value: Signal<String>,
    placeholder: impl IntoReactive<String> + 'static,
    height: f32,
    multiline: bool,
    on_change: impl Fn(&str) + 'static,
) -> impl IntoElement {
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

fn edited_status(compose: Compose) -> Option<u16> {
    if !compose.response.get() {
        return None;
    }
    let code = compose.status.get().trim().parse::<u16>().ok()?;
    (100..600).contains(&code).then_some(code)
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
                .rounded(px(99.0))
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

fn prompt_import_har(controls: Arc<Controls>) {
    spawn(async move {
        let dialog = nsh_core::file_dialog::OpenDialog::files()
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

fn begin_compose(controls: &Controls, id: Option<u64>, compose: Compose) {
    let Some(flow) = selected_flow(controls, id) else {
        set_status(controls, StatusMsg::NeedSelection);
        return;
    };
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

fn clear_captured(
    controls: &Controls,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    compose: Compose,
) {
    controls.hub.clear_session();
    selected.set(None);
    baseline.set(None);
    reset_compose(compose);
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
    rows: Signal<Vec<RowLine>>,
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
    let Some(id) = id else {
        set_status(controls, StatusMsg::NeedSelection);
        return;
    };
    let hub = controls.hub.clone();
    let controls = Arc::clone(controls);
    hub.replay_later(id, edit, move |result| match result {
        Ok(new_id) => set_status(&controls, StatusMsg::Replayed(new_id)),
        Err(err) => set_status(&controls, StatusMsg::ReplayFailed(format!("{err:#}"))),
    });
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

fn export_har_file(controls: Arc<Controls>, filter: &str) {
    let value = controls.hub.export_har_matching(filter);
    let bytes = match serde_json::to_vec_pretty(&value) {
        Ok(bytes) => bytes,
        Err(err) => {
            set_status(&controls, StatusMsg::ExportFailed(err.to_string()));
            return;
        }
    };
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let dialog = nsh_core::file_dialog::SaveDialog::new()
        .file_name(format!("zniff-{secs}.har"))
        .filter(nsh_core::file_dialog::FileFilter::new("HAR", ["har"]));
    spawn(async move {
        match pick_save_path(dialog).await {
            Ok(Some(path)) => {
                if let Some(parent) = path.parent()
                    && !parent.as_os_str().is_empty()
                    && let Err(err) = std::fs::create_dir_all(parent)
                {
                    set_status(&controls, StatusMsg::ExportFailed(err.to_string()));
                    return;
                }
                if let Err(err) = std::fs::write(&path, &bytes) {
                    set_status(&controls, StatusMsg::ExportFailed(err.to_string()));
                    return;
                }
                let text = path.display().to_string();
                set_clipboard(text.clone());
                set_status(&controls, StatusMsg::Exported(text));
            }
            Ok(None) => {}
            Err(err) => set_status(&controls, StatusMsg::ExportFailed(err)),
        }
    });
}

fn export_html_file(controls: Arc<Controls>, filter: &str, zh: bool) {
    let flows = controls
        .hub
        .with_session(|session| session.flows_matching(filter));
    let json = share::document(&flows, SystemTime::now(), filter);
    let html = share::page(&json, "", zh);
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let dialog = nsh_core::file_dialog::SaveDialog::new()
        .file_name(format!("zniff-{secs}.html"))
        .filter(nsh_core::file_dialog::FileFilter::new(
            "HTML",
            ["html", "htm"],
        ));
    spawn(async move {
        match pick_save_path(dialog).await {
            Ok(Some(path)) => {
                if let Some(parent) = path.parent()
                    && !parent.as_os_str().is_empty()
                    && let Err(err) = std::fs::create_dir_all(parent)
                {
                    set_status(&controls, StatusMsg::ExportFailed(err.to_string()));
                    return;
                }
                if let Err(err) = std::fs::write(&path, &html) {
                    set_status(&controls, StatusMsg::ExportFailed(err.to_string()));
                    return;
                }
                let text = path.display().to_string();
                set_clipboard(text.clone());
                set_status(&controls, StatusMsg::Exported(text));
            }
            Ok(None) => {}
            Err(err) => set_status(&controls, StatusMsg::ExportFailed(err)),
        }
    });
}

fn begin_share(controls: &Controls, minutes: u64, filter: &str, share_id: Signal<String>) {
    if !lock_bool(&controls.running) {
        set_status(controls, StatusMsg::ShareNeedsCapture);
        return;
    }
    let (id, _expires) = controls
        .hub
        .begin_share(Duration::from_secs(minutes.saturating_mul(60)), filter);
    let (base, local_only) = share_base(controls);
    let url = format!("{base}/s/{id}");
    share_id.set(id);
    set_clipboard(url.clone());
    set_status(
        controls,
        StatusMsg::Shared {
            url,
            minutes,
            local_only,
        },
    );
}

fn end_share(controls: &Controls, share_id: Signal<String>) {
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
    push_saved_profile(
        saved,
        &effective_name,
        filter,
        bypass,
        rules,
        upstream,
        rules_enabled,
        breaks_enabled,
        show_tunnel_failures,
    )
}

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

fn export_profiles_file(
    controls: Arc<Controls>,
    preferred_name: String,
    profiles: Vec<config::SavedProfile>,
) {
    let toml_text = match config::export_profiles_toml(&profiles) {
        Ok(text) => text,
        Err(err) => {
            set_status(&controls, StatusMsg::ExportFailed(err));
            return;
        }
    };
    let count = profiles.len();
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let file_name = profile_export_filename(&preferred_name, &profiles, secs);
    let dialog = nsh_core::file_dialog::SaveDialog::new()
        .file_name(file_name)
        .filter(nsh_core::file_dialog::FileFilter::new("TOML", ["toml"]));
    spawn(async move {
        match pick_save_path(dialog).await {
            Ok(Some(path)) => {
                if let Some(parent) = path.parent()
                    && !parent.as_os_str().is_empty()
                    && let Err(err) = std::fs::create_dir_all(parent)
                {
                    set_status(&controls, StatusMsg::ExportFailed(err.to_string()));
                    return;
                }
                if let Err(err) = std::fs::write(&path, &toml_text) {
                    set_status(&controls, StatusMsg::ExportFailed(err.to_string()));
                    return;
                }
                set_clipboard(toml_text);
                set_status(
                    &controls,
                    StatusMsg::ProfilesExported {
                        count,
                        path: path.display().to_string(),
                    },
                );
            }
            Ok(None) => {}
            Err(err) => set_status(&controls, StatusMsg::ExportFailed(err)),
        }
    });
}

#[cfg_attr(not(test), allow(dead_code))]
fn parse_profiles_from_text_or_path(
    input: &str,
) -> std::result::Result<Vec<config::SavedProfile>, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("empty clipboard".to_string());
    }
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
    config::parse_profiles_toml(trimmed)
}

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
    let count = incoming.len();
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

/// `file:` URL of a `.har`, or nothing. Encoded names are decoded. A Windows
/// `file:///C:/...` drops the extra slash. Other schemes are ignored.
fn dropped_har_path(uri: &str) -> Option<String> {
    dropped_ext_path(uri, "har")
}

/// `file:` URL of a `.toml` profile file, or nothing.
fn dropped_toml_path(uri: &str) -> Option<String> {
    dropped_ext_path(uri, "toml")
}

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

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

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
    hub.set_show_tunnel_failures(saved.show_tunnel_failures);
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
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("1.1.1.1:80").ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if usable_v4(ip) => Some(ip),
        _ => None,
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

fn png_gray(zlib_data: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut ihdr = Vec::new();
    ihdr.extend(width.to_be_bytes());
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 0, 0, 0, 0]);
    let mut out = vec![137, 80, 78, 71, 13, 10, 26, 10];
    out.extend(png_chunk(b"IHDR", &ihdr));
    out.extend(png_chunk(b"IDAT", zlib_data));
    out.extend(png_chunk(b"IEND", &[]));
    out
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(4 + data.len());
    body.extend_from_slice(kind);
    body.extend_from_slice(data);
    let crc = crc32(&body);
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(body);
    out.extend(crc.to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn format_bytes(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
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
        let shown = super::body_text(&raw, false, None, None, super::Lang::Zh);
        assert_eq!(shown.text.len(), 256 * 1024 - 2);
        assert!(shown.text.is_char_boundary(shown.text.len()));
        assert!(shown.text.chars().all(|ch| ch == 'a'));
        let rest = raw.len() - shown.text.len();
        assert!(shown.note.contains(&rest.to_string()), "{}", shown.note);

        let ascii = vec![b'x'; 256 * 1024 + 10];
        let ascii_shown = super::body_text(&ascii, false, None, None, super::Lang::En);
        assert_eq!(ascii_shown.text.len(), 256 * 1024);
        assert!(
            ascii_shown.note.contains("10 more bytes"),
            "{}",
            ascii_shown.note
        );

        let expired_shown = super::body_text(&[], false, Some(65536), None, super::Lang::Zh);
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
            clock: "12:00:00.000".to_string(),
            elapsed: "10 ms".to_string(),
            elapsed_slow: false,
            host: "api.example.com".to_string(),
            client_ip: "127.0.0.1".to_string(),
            path: "/v1/users".to_string(),
            url: "https://api.example.com/v1/users".to_string(),
            protocol: "HTTP/2".to_string(),
            size: "128 B".to_string(),
            failed: false,
            pinned: false,
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
        assert_eq!(super::adaptive_filter_panel_width(900.0, 560.0), 680.0);
        assert_eq!(super::adaptive_filter_panel_width(1200.0, 560.0), 792.0);
        assert_eq!(super::adaptive_filter_panel_width(1448.0, 560.0), 1040.0);
        assert_eq!(super::adaptive_filter_panel_width(1448.0, 644.0), 981.0);
        assert_eq!(super::adaptive_rules_editor_height(0.0), 105.0);
        assert_eq!(super::adaptive_rules_editor_height(520.0), 105.0);
        assert_eq!(super::adaptive_rules_editor_height(700.0), 240.0);
        assert_eq!(super::adaptive_rules_editor_height(1000.0), 540.0);
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
        };
        let mut widths = std::collections::HashMap::new();
        widths.insert("path".to_string(), 400);
        let cols = super::flow_table_columns(shown, &widths);
        let keys: Vec<&str> = cols.iter().map(|column| column.key.as_ref()).collect();
        assert_eq!(keys, ["method", "path", "status", "gap", "elapsed"]);
        assert!(cols[0].fixed && cols[1].fixed);
        assert!(cols[2..].iter().all(|column| !column.fixed));
        assert_eq!(cols[0].width.unwrap().0, 76.0);
        assert_eq!(cols[1].width.unwrap().0, 400.0);
        assert_eq!(cols[1].min_width.unwrap().0, 80.0);
        assert!(
            cols.iter()
                .all(|column| { column.movable == Some(false) && column.resizable == Some(true) })
        );
        let hidden_path = super::ListColumns {
            path: false,
            ..shown
        };
        let cols = super::flow_table_columns(hidden_path, &widths);
        assert_eq!(cols[0].key.as_ref(), "method");
        assert!(cols.iter().all(|column| column.key.as_ref() != "path"));
        assert_eq!(cols.iter().filter(|column| column.fixed).count(), 1);

        let default_shown = super::ListColumns::from_config(&crate::config::Config::default());
        let default_cols =
            super::flow_table_columns(default_shown, &std::collections::HashMap::new());
        let default_keys: Vec<&str> = default_cols.iter().map(|c| c.key.as_ref()).collect();
        assert_eq!(
            default_keys,
            ["id", "method", "path", "status", "gap", "elapsed", "size"]
        );
        assert!(default_cols[0].fixed && default_cols[1].fixed && default_cols[2].fixed);
        let default_sum: f32 = default_cols.iter().map(|c| c.width.unwrap().0).sum();
        assert_eq!(default_sum, crate::config::LIST_WIDTH_DEFAULT as f32);

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

        // When `saved` has a profile and the user is editing a new unnamed setup,
        // `profiles_for_export` includes both the new setup and the existing saved profile.
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
        assert_eq!(with_new_unnamed.len(), 2);
        assert_eq!(with_new_unnamed[0].name, "host:staging.dev");
        assert_eq!(with_new_unnamed[1].name, "Dev");
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
                &saved, name, filter, "", rules, "", rules_on, true, tunnels, host, "", from, "",
                false, 0,
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
        assert!(!ready(
            "test",
            "host:a.test",
            "break a.test/api",
            true,
            false,
            "",
            "not valid"
        ));
    }
}
