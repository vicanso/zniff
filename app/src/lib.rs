//! zniff: a desktop HTTP(S) capture app. The window is an nsh Rust view.
//! The proxy runs on its own tokio runtime in this process.

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
mod sysproxy;
mod ws;

use std::net::{Ipv4Addr, SocketAddr};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nsh_base::{
    DiffLayout, context_menu, diff_view, editor, input, menu, menu_item, popover, theme, tooltip,
};
use nsh_core::prelude::*;
use nsh_core::EllipsizeMode;

use crate::ca::Ca;
use crate::i18n::{
    direction_name, lang_from_tag, localize_error, parse_pref, t, ws_kind, Lang, PrefName, Side,
    StatusMsg,
};
use crate::icon::{glyph, glyph_px, Glyph};
use crate::proxy::{BreakDecision, BreakSide, Capture, Hub, ReplayOverride};
use crate::session::Direction;

pub use sysproxy::restore_if_dirty;

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
const COL_METHOD: f32 = 72.0;
const COL_STATUS: f32 = 56.0;
const COL_PROTOCOL: f32 = 84.0;
const COL_TIME: f32 = 72.0;
const COL_HOST: f32 = 120.0;
const COL_SIZE: f32 = 72.0;
/// Same width as the header's 「列」 control, so the columns line up with it.
const COL_PICKER: f32 = 44.0;
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

fn bg_col_header() -> AdaptiveColor {
    AdaptiveColor::rgb(0xf1f5f9, 0x151b26)
}

fn bg_row_alt() -> AdaptiveColor {
    AdaptiveColor::rgb(0xfafbfd, 0x121722)
}

fn bg_row_hover() -> AdaptiveColor {
    AdaptiveColor::rgb(0xf1f5f9, 0x1a2232)
}

fn bg_row_selected() -> AdaptiveColor {
    AdaptiveColor::rgb(0xeff6ff, 0x172542)
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
    host: String,
    path: String,
    protocol: String,
    size: String,
    failed: bool,
    slow: bool,
    pinned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ListColumns {
    status: bool,
    protocol: bool,
    path: bool,
    gap: bool,
    host: bool,
    size: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListColumn {
    Status,
    Protocol,
    Path,
    Gap,
    Host,
    Size,
}

impl ListColumns {
    fn from_config(config: &config::Config) -> Self {
        Self {
            status: config.show_status,
            protocol: config.show_protocol,
            path: config.show_path,
            gap: config.show_gap,
            host: config.show_host,
            size: config.show_size,
        }
    }

    fn on(self, column: ListColumn) -> bool {
        match column {
            ListColumn::Status => self.status,
            ListColumn::Protocol => self.protocol,
            ListColumn::Path => self.path,
            ListColumn::Gap => self.gap,
            ListColumn::Host => self.host,
            ListColumn::Size => self.size,
        }
    }

    fn toggle(self, column: ListColumn) -> Self {
        let mut next = self;
        match column {
            ListColumn::Status => next.status = !next.status,
            ListColumn::Protocol => next.protocol = !next.protocol,
            ListColumn::Path => next.path = !next.path,
            ListColumn::Gap => next.gap = !next.gap,
            ListColumn::Host => next.host = !next.host,
            ListColumn::Size => next.size = !next.size,
        }
        next
    }

    fn write(self, config: &mut config::Config) {
        config.show_status = self.status;
        config.show_protocol = self.protocol;
        config.show_path = self.path;
        config.show_gap = self.gap;
        config.show_host = self.host;
        config.show_size = self.size;
    }
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
    status: Signal<Option<u16>>,
    error: Signal<String>,
    request_headers: Signal<Vec<HeaderLine>>,
    response_headers: Signal<Vec<HeaderLine>>,
    request_body: Signal<String>,
    response_body: Signal<String>,
    request_language: Signal<String>,
    response_language: Signal<String>,
    request_note: Signal<String>,
    response_note: Signal<String>,
    metrics: Signal<String>,
    query: Signal<Vec<HeaderLine>>,
    preview: Signal<String>,
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
    let filter = signal(String::new());
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
        status: signal(None),
        error: signal(String::new()),
        request_headers: signal(Vec::new()),
        response_headers: signal(Vec::new()),
        request_body: signal(String::new()),
        response_body: signal(String::new()),
        request_language: signal(String::new()),
        response_language: signal(String::new()),
        request_note: signal(String::new()),
        response_note: signal(String::new()),
        metrics: signal(String::new()),
        query: signal(Vec::new()),
        preview: signal(String::new()),
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
    let import_path = signal(String::new());
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

    let controls_tick = controls.clone();
    let filter_tick = filter;
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
    set_interval(Duration::from_millis(16), move || {
        status.set(lock_status(&controls_tick));
        running.set(*controls_tick.running.lock().unwrap_or_else(|err| err.into_inner()));
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
        let selected_now = selected_tick.get();
        let lang_now = ui_lang(langs);
        let baseline_now = baseline_tick.get();
        if generation != seen_gen.get()
            || filter_now != seen_filter.get()
            || selected_now != seen_selected.get()
            || lang_now != seen_lang.get()
            || baseline_now != seen_baseline.get()
        {
            seen_gen.replace(generation);
            seen_filter.replace(filter_now.clone());
            seen_selected.replace(selected_now);
            seen_lang.replace(lang_now);
            seen_baseline.replace(baseline_now);
            let lines = row_lines(&controls_tick.hub, &filter_now, lang_now);
            total_tick.set(controls_tick.hub.with_session(|session| session.len()));
            rows_tick.set(lines);
            memory.set(format_bytes(
                controls_tick.hub.with_session(|session| session.memory_bytes()),
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

    view()
        .flex_1()
        .bg(bg_canvas())
        .text_color(fg_primary())
        .on_file_drop(move |event: &FileDropEvent| {
            import_dropped(&controls_drop, event, ui_lang(drop_langs));
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
                controls_clear.hub.clear_session();
                selected_clear.set(None);
                baseline_clear.set(None);
                reset_compose(compose_clear);
            },
            move || export_har_file(&controls_export),
            move || copy_proxy_env(&controls_env),
            move || toggle_capture(controls_start.clone()),
            import_path,
            controls_import,
            rows,
            flow_total,
            filter,
            shown.pretty,
        ))
        .child(when(
            move || paused.get().is_some(),
            move || {
                break_bar(paused, compose, selected, controls_break.clone())
            },
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
                    bypass_text,
                    rules_text,
                    upstream_text,
                    list_width,
                    columns,
                    controls_list,
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
                )),
        )
        .child(footer(
            listen,
            running,
            memory,
            dropped,
            port_text,
            lan_url,
            lan_choice,
            lan_addrs,
            qr_path,
            listen_all,
            controls,
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
    import_path: Signal<String>,
    import_controls: Arc<Controls>,
    rows: Signal<Vec<RowLine>>,
    flow_total: Signal<usize>,
    filter: Signal<String>,
    pretty: Signal<bool>,
) -> impl IntoElement {
    let install = Rc::new(install);
    let remove = Rc::new(remove);
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
        .child(toolbar_menu(
            ca_installed,
            use_system_proxy,
            install,
            remove,
            toggle_proxy,
            export_har,
            copy_env,
            clear,
            import_path,
            import_controls,
            rows,
            flow_total,
            filter,
            pretty,
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

fn clamp_list_px(width: f32) -> f32 {
    let rounded = if width.is_finite() && width > 0.0 {
        width.round().min(u32::MAX as f32) as u32
    } else {
        0
    };
    config::clamp_list_width(rounded) as f32
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
        .on_hover_in(move || hot.set(true))
        .on_hover_out(move || hot.set(false))
        .on_responder_grant(move |_event: &ResponderEvent| {
            origin.set(width_grant.get());
        })
        .on_responder_move(move |event: &ResponderEvent| {
            width_move.set(clamp_list_px(origin.get() + event.gesture.delta.x.0));
        })
        .on_responder_release(move |_event: &ResponderEvent| {
            store_list_width(width_release.get(), &controls_release);
        })
        .on_responder_terminate(move |_event: &ResponderEvent| {
            store_list_width(width_cancel.get(), &controls_cancel);
        })
        .child(
            view()
                .w(move || if hot.get() { px(3.0) } else { px(1.0) })
                .h_full()
                .bg(move || if hot.get() { accent_blue() } else { border_strong() }),
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
    bypass_text: Signal<String>,
    rules_text: Signal<String>,
    upstream_text: Signal<String>,
    width: Signal<f32>,
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    let bypass_controls = controls.clone();
    let rules_controls = controls.clone();
    let column_controls = controls.clone();
    let upstream_controls = controls;
    view()
        .w(move || px(width.get()))
        .flex_shrink_0()
        .overflow_hidden()
        .bg(bg_list())
        .child(
            view()
                .w(move || px(width.get()))
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
                    view().flex_1().min_w(px(0.0)).child(side_input(
                        filter,
                        tr(
                            "主机、路径、GET、404、-host、mute:、pin:",
                            "host, path, GET, 404, -host, mute:, pin:",
                        ),
                        30.0,
                        false,
                        move |text| filter.set(text.to_string()),
                    )),
                )
                .child(capture_config(
                    bypass_text,
                    rules_text,
                    upstream_text,
                    bypass_controls,
                    rules_controls,
                    upstream_controls,
                    width,
                )),
        )
        .child(column_header(width, columns, column_controls))
        .child(view().flex_1().child(
            when(
                move || rows.with(Vec::is_empty),
                move || list_empty(filter),
            )
            .otherwise(move || flow_rows(rows, selected, compose, width, columns)),
        ))
}

fn side_input(
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
        .text_size(px(12.5))
        .text_color(fg_primary())
        .multiline(multiline)
        .on_change(on_change)
}

fn capture_config(
    bypass_text: Signal<String>,
    rules_text: Signal<String>,
    upstream_text: Signal<String>,
    bypass_controls: Arc<Controls>,
    rules_controls: Arc<Controls>,
    upstream_controls: Arc<Controls>,
    width: Signal<f32>,
) -> impl IntoElement {
    let bypass_for_panel = bypass_text;
    let rules_for_panel = rules_text;
    let upstream_for_panel = upstream_text;
    popover()
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(8.0))
        .trigger(config_trigger(
            bypass_for_panel,
            rules_for_panel,
            upstream_for_panel,
        ))
        .child(config_panel(
            bypass_for_panel,
            rules_for_panel,
            upstream_for_panel,
            bypass_controls,
            rules_controls,
            upstream_controls,
            width,
        ))
}

fn config_set(bypass: Signal<String>, rules: Signal<String>, upstream: Signal<String>) -> bool {
    !bypass.get().trim().is_empty()
        || !rules.get().trim().is_empty()
        || !upstream.get().trim().is_empty()
}

fn config_trigger(
    bypass_text: Signal<String>,
    rules_text: Signal<String>,
    upstream_text: Signal<String>,
) -> impl IntoElement {
    let border_bypass = bypass_text;
    let border_rules = rules_text;
    let border_upstream = upstream_text;
    let dot_bypass = bypass_text;
    let dot_rules = rules_text;
    let dot_upstream = upstream_text;
    let label_bypass = bypass_text;
    let label_rules = rules_text;
    let label_upstream = upstream_text;
    view()
        .h(px(30.0))
        .px_2p5()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_1p5()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(move || {
            if config_set(border_bypass, border_rules, border_upstream) {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(bg_control())
        .hover(|style| style.bg(bg_control_hover()))
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.75))
        .child(when(
            move || config_set(dot_bypass, dot_rules, dot_upstream),
            move || {
                view()
                    .w(px(6.0))
                    .h(px(6.0))
                    .rounded(px(99.0))
                    .bg(status_ok_fg())
            },
        ))
        .tooltip(label_tip("配置", "Config"))
        .child(glyph(
            || Glyph::Sliders,
            move || {
                color(if config_set(label_bypass, label_rules, label_upstream) {
                    accent_blue_text()
                } else {
                    fg_secondary()
                })
            },
        ))
}

fn config_panel(
    bypass_text: Signal<String>,
    rules_text: Signal<String>,
    upstream_text: Signal<String>,
    bypass_controls: Arc<Controls>,
    rules_controls: Arc<Controls>,
    upstream_controls: Arc<Controls>,
    width: Signal<f32>,
) -> impl IntoElement {
    view()
        .w(move || px((width.get() - LIST_INSET - LIST_PAD_END).max(200.0)))
        .p_3()
        .gap_3()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
        .child(config_field(
            "直通",
            "Bypass",
            side_input(
                bypass_text,
                tr(
                    "example.com *.pin.dev 或 only:api.example.com",
                    "example.com *.pin.dev or only:api.example.com",
                ),
                30.0,
                false,
                move |text| {
                    bypass_text.set(text.to_string());
                    bypass_controls.hub.set_bypass(text);
                    if let Err(err) = save_config(|config| config.bypass_hosts = text.to_string()) {
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
        .child(config_field(
            "规则",
            "Rules",
            side_input(
                rules_text,
                tr(
                    "map-local、break-res、replace host res a -> b",
                    "map-local, break-res, replace host res a -> b",
                ),
                64.0,
                true,
                {
                    let rules_controls = rules_controls.clone();
                    move |text| {
                        rules_text.set(text.to_string());
                        rules_controls.hub.set_rules(text);
                        if let Err(err) = save_config(|config| config.rules = text.to_string()) {
                            set_status(
                                &rules_controls,
                                StatusMsg::PrefUnsaved {
                                    name: PrefName::Rules,
                                    err: err.to_string(),
                                },
                            );
                        }
                    }
                },
            ),
        ))
        .child(replace_row(rules_text, rules_controls))
        .child(config_field(
            "上游",
            "Upstream",
            side_input(
                upstream_text,
                tr("host:port，空着直连", "host:port, empty means direct"),
                30.0,
                false,
                move |text| {
                    upstream_text.set(text.to_string());
                    upstream_controls.hub.set_upstream(text);
                    if let Err(err) = save_config(|config| config.upstream_proxy = text.to_string()) {
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
}

fn replace_row(rules_text: Signal<String>, controls: Arc<Controls>) -> impl IntoElement {
    let pattern = signal(String::new());
    let from = signal(String::new());
    let to = signal(String::new());
    let response = signal(true);
    view()
        .gap_1()
        .w_full()
        .child(
            text(tr("添加替换", "Add replace"))
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_muted()),
        )
        .child(
            view()
                .flex_row()
                .gap_1()
                .child(view().flex_1().min_w(px(0.0)).child(side_input(
                    pattern,
                    tr("example.com/api", "example.com/api"),
                    28.0,
                    false,
                    move |text| pattern.set(text.to_string()),
                )))
                .child(
                    view()
                        .w(px(52.0))
                        .h(px(28.0))
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(border_control())
                        .bg(bg_control())
                        .cursor(Cursor::Pointer)
                        .on_press(move || response.set(!response.get()))
                        .child(
                            text(move || {
                                if response.get() {
                                    tr("响应", "Res")()
                                } else {
                                    tr("请求", "Req")()
                                }
                            })
                            .text_size(px(11.0))
                            .text_color(fg_secondary()),
                        ),
                ),
        )
        .child(
            view()
                .flex_row()
                .gap_1()
                .child(view().flex_1().min_w(px(0.0)).child(side_input(
                    from,
                    tr("原文", "from"),
                    28.0,
                    false,
                    move |text| from.set(text.to_string()),
                )))
                .child(view().flex_1().min_w(px(0.0)).child(side_input(
                    to,
                    tr("替换成", "to"),
                    28.0,
                    false,
                    move |text| to.set(text.to_string()),
                )))
                .child(
                    view()
                        .w(px(52.0))
                        .h(px(28.0))
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .bg(accent_blue())
                        .cursor(Cursor::Pointer)
                        .on_press(move || {
                            let Some(next) = crate::rules::append_replace_line(
                                &rules_text.get(),
                                &pattern.get(),
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
                            pattern.set(String::new());
                            from.set(String::new());
                            to.set(String::new());
                        })
                        .child(
                            text(tr("添加", "Add"))
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(white()),
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
                        t(lang, "点击右上角「开始」捕获 HTTP(S) 请求", "Press Start to capture HTTP(S)").into()
                    } else {
                        t(lang, "试试调整上方的过滤关键词", "Try a different filter").into()
                    }
                }
            })
            .text_size(px(12.0))
            .text_color(fg_muted()),
        )
}

fn flow_rows(
    rows: Signal<Vec<RowLine>>,
    selected: Signal<Option<u64>>,
    compose: Compose,
    width: Signal<f32>,
    columns: Signal<ListColumns>,
) -> impl IntoElement {
    virtual_list(
        move || rows.with(Vec::len),
        move |at| flow_row(rows, selected, compose, width, columns, at),
    )
    .keys(move || rows.with(|rows| rows.iter().map(|row| row.id).collect::<Vec<_>>()))
    .estimated_item_size(px(34.0))
    // Newest stays at the bottom. Follow it while the viewport is still on
    // that last row; scrolling further up keeps the visible rows put.
    .follow_end(px(36.0))
    .flex_1()
}

fn column_header(
    width: Signal<f32>,
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    context_menu()
        .offset(px(4.0))
        .margin(px(8.0))
        .trigger(column_header_row(width, columns, controls.clone()))
        .child(column_picker_panel(columns, controls))
}

fn column_header_row(
    width: Signal<f32>,
    columns: Signal<ListColumns>,
    controls: Arc<Controls>,
) -> impl IntoElement {
    view()
        .w(move || px(width.get()))
        .flex_shrink_0()
        .h(px(28.0))
        .px_3()
        .gap_2()
        .flex_row()
        .items_center()
        .bg(bg_col_header())
        .border_b_1()
        .border_b_color(border_strong())
        .border_l(px(2.0))
        .border_l_color(bg_col_header())
        .child(column_labels(columns))
        .child(column_picker(columns, controls))
}

fn column_labels(columns: Signal<ListColumns>) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .overflow_hidden()
        .flex_row()
        .items_center()
        .gap_2()
        .child(header_label("方法", "Method").w(px(COL_METHOD)).flex_shrink_0())
        .child(when(move || columns.get().path, move || {
            header_label("路径", "Path").flex_1().min_w(px(0.0))
        }))
        .child(when(move || columns.get().status, move || {
            header_label("状态", "Status").w(px(COL_STATUS)).flex_shrink_0()
        }))
        .child(when(move || columns.get().protocol, move || {
            header_label("协议", "Protocol").w(px(COL_PROTOCOL)).flex_shrink_0()
        }))
        .child(when(move || columns.get().gap, move || {
            header_label("间隔", "Gap").w(px(COL_TIME)).flex_shrink_0()
        }))
        .child(when(move || columns.get().host, move || {
            header_label("主机", "Host").w(px(COL_HOST)).flex_shrink_0()
        }))
        .child(when(move || columns.get().size, move || {
            header_label("大小", "Size").w(px(COL_SIZE)).flex_shrink_0()
        }))
}

fn column_picker(columns: Signal<ListColumns>, controls: Arc<Controls>) -> impl IntoElement {
    view()
        .w(px(COL_PICKER))
        .flex_shrink_0()
        .child(column_menu(columns, controls))
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
        .w(px(COL_PICKER))
        .h(px(22.0))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .hover(|style| style.bg(bg_control_hover()))
        .cursor(Cursor::Pointer)
        .tooltip(label_tip("列", "Columns"))
        .child(glyph(|| Glyph::Columns, || color(fg_secondary())))
}

fn column_picker_panel(columns: Signal<ListColumns>, controls: Arc<Controls>) -> impl IntoElement {
    view()
        .w(px(168.0))
        .p_1()
        .gap_0p5()
        .rounded(px(10.0))
        .border_1()
        .border_color(border_strong())
        .bg(bg_card())
        .shadow_md()
        .child(column_toggle(columns, controls.clone(), ListColumn::Status, "状态", "Status"))
        .child(column_toggle(columns, controls.clone(), ListColumn::Protocol, "协议", "Protocol"))
        .child(column_toggle(columns, controls.clone(), ListColumn::Path, "路径", "Path"))
        .child(column_toggle(columns, controls.clone(), ListColumn::Gap, "间隔", "Gap"))
        .child(column_toggle(columns, controls.clone(), ListColumn::Host, "主机", "Host"))
        .child(column_toggle(columns, controls, ListColumn::Size, "大小", "Size"))
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

fn header_label(zh: &'static str, en: &'static str) -> Text {
    text(tr(zh, en))
        .number_of_lines(1)
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg_muted())
}

fn flow_row(
    rows: Signal<Vec<RowLine>>,
    selected: Signal<Option<u64>>,
    compose: Compose,
    width: Signal<f32>,
    columns: Signal<ListColumns>,
    at: Position,
) -> impl IntoElement {
    let row_id = move || rows.with(|rows| rows.get(at.index()).map(|row| row.id));
    let is_selected = move || selected.get() == row_id();
    view()
        .w(move || px(width.get()))
        .h(px(34.0))
        .px_3()
        .gap_2()
        .flex_row()
        .items_center()
        .border_b_1()
        .border_b_color(border_subtle())
        .border_l(px(2.0))
        .border_l_color(move || {
            if is_selected() {
                accent_blue()
            } else if at.index() % 2 == 1 {
                bg_row_alt()
            } else {
                bg_list()
            }
        })
        .overflow_hidden()
        .cursor(Cursor::Pointer)
        .bg(move || {
            if is_selected() {
                bg_row_selected()
            } else if at.index() % 2 == 1 {
                bg_row_alt()
            } else {
                bg_list()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if is_selected() {
                    bg_row_selected()
                } else {
                    bg_row_hover()
                }
            })
        })
        .on_press(move || {
            let next = row_id();
            if selected.get() != next {
                reset_compose(compose);
            }
            selected.set(next);
        })
        .child(row_cells(rows, columns, at))
        .child(view().w(px(COL_PICKER)).flex_shrink_0())
}

fn row_cells(rows: Signal<Vec<RowLine>>, columns: Signal<ListColumns>, at: Position) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(0.0))
        .overflow_hidden()
        .flex_row()
        .items_center()
        .gap_2()
        .child(method_cell(rows, at))
        .child(when(move || columns.get().path, move || path_cell(rows, at)))
        .child(when(move || columns.get().status, move || status_text(rows, at)))
        .child(when(move || columns.get().protocol, move || protocol_cell(rows, at)))
        .child(when(move || columns.get().gap, move || time_cell(rows, at)))
        .child(when(move || columns.get().host, move || host_cell(rows, at)))
        .child(when(move || columns.get().size, move || size_cell(rows, at)))
}

fn method_cell(rows: Signal<Vec<RowLine>>, at: Position) -> impl IntoElement {
    view()
        .w(px(COL_METHOD))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .child(
            view()
                .px_1p5()
                .py_0p5()
                .rounded(px(4.0))
                .bg(move || {
                    rows.with(|rows| {
                        rows.get(at.index())
                            .map(|row| method_bg(&row.method))
                            .unwrap_or_else(bg_control)
                    })
                })
                .child(
                    mono(move || {
                        rows.with(|rows| {
                            rows.get(at.index())
                                .map(|row| row.method.clone())
                                .unwrap_or_default()
                        })
                    })
                    .number_of_lines(1)
                    .text_size(px(11.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(move || {
                        rows.with(|rows| {
                            rows.get(at.index())
                                .map(|row| method_fg(&row.method))
                                .unwrap_or_else(fg_secondary)
                        })
                    }),
                ),
        )
}

fn status_text(rows: Signal<Vec<RowLine>>, at: Position) -> Text {
    mono(move || {
        rows.with(|rows| {
            rows.get(at.index())
                .map(|row| list_status(row.status, row.failed))
                .unwrap_or_default()
        })
    })
    .w(px(COL_STATUS))
    .flex_shrink_0()
    .number_of_lines(1)
    .text_size(px(12.0))
    .font_weight(FontWeight::SEMIBOLD)
    .text_color(move || {
        let row = rows.with(|rows| rows.get(at.index()).cloned());
        match row {
            Some(row) => http_status_fg(row.status, row.failed),
            None => fg_muted(),
        }
    })
}

fn time_cell(rows: Signal<Vec<RowLine>>, at: Position) -> Text {
    let tip_rows = rows;
    mono(move || {
        rows.with(|rows| {
            rows.get(at.index())
                .map(|row| row.time.clone())
                .unwrap_or_default()
        })
    })
    .w(px(COL_TIME))
    .flex_shrink_0()
    .number_of_lines(1)
    .text_size(px(12.0))
    .text_color(move || {
        if rows.with(|rows| rows.get(at.index()).is_some_and(|row| row.slow)) {
            status_warn_fg()
        } else {
            fg_muted()
        }
    })
    .tooltip(cell_tip(move || {
        tip_rows.with(|rows| {
            rows.get(at.index())
                .map(|row| row.clock.clone())
                .unwrap_or_default()
        })
    }))
}

fn host_cell(rows: Signal<Vec<RowLine>>, at: Position) -> impl IntoElement {
    let tip_rows = rows;
    view()
        .w(px(COL_HOST))
        .flex_shrink_0()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .gap_1()
        .tooltip(cell_tip(move || {
            tip_rows.with(|rows| {
                rows.get(at.index())
                    .map(|row| row.host.clone())
                    .unwrap_or_default()
            })
        }))
        .child(when(
            move || rows.with(|rows| rows.get(at.index()).is_some_and(|row| row.pinned)),
            || {
                view()
                    .w(px(6.0))
                    .h(px(6.0))
                    .rounded(px(99.0))
                    .flex_shrink_0()
                    .bg(accent_blue())
            },
        ))
        .child(
            mono(move || {
                rows.with(|rows| {
                    rows.get(at.index())
                        .map(|row| row.host.clone())
                        .unwrap_or_default()
                })
            })
            .flex_1()
            .min_w(px(0.0))
            .number_of_lines(1)
            .ellipsize_mode(EllipsizeMode::Middle)
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(fg_primary()),
        )
}

fn protocol_cell(rows: Signal<Vec<RowLine>>, at: Position) -> Text {
    mono(move || {
        rows.with(|rows| {
            rows.get(at.index())
                .map(|row| row.protocol.clone())
                .unwrap_or_default()
        })
    })
    .w(px(COL_PROTOCOL))
    .flex_shrink_0()
    .number_of_lines(1)
    .text_size(px(12.0))
    .text_color(fg_secondary())
}

fn size_cell(rows: Signal<Vec<RowLine>>, at: Position) -> Text {
    mono(move || {
        rows.with(|rows| {
            rows.get(at.index())
                .map(|row| row.size.clone())
                .unwrap_or_default()
        })
    })
    .w(px(COL_SIZE))
    .flex_shrink_0()
    .number_of_lines(1)
    .text_size(px(12.0))
    .text_color(fg_muted())
}

fn path_cell(rows: Signal<Vec<RowLine>>, at: Position) -> Text {
    let tip_rows = rows;
    mono(move || {
        rows.with(|rows| {
            rows.get(at.index())
                .map(|row| row.path.clone())
                .unwrap_or_default()
        })
    })
    .flex_1()
    .min_w(px(0.0))
    .number_of_lines(1)
    .ellipsize_mode(EllipsizeMode::Middle)
    .text_size(px(12.0))
    .text_color(move || {
        if rows.with(|rows| rows.get(at.index()).is_some_and(|row| row.failed)) {
            status_err_fg()
        } else {
            fg_secondary()
        }
    })
    .tooltip(cell_tip(move || {
        tip_rows.with(|rows| {
            rows.get(at.index())
                .map(|row| row.path.clone())
                .unwrap_or_default()
        })
    }))
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

/// A reactive bubble. A plain string tooltip is captured once, and a virtual row reuses its cell.
fn cell_tip(full: impl Fn() -> String + 'static) -> impl IntoTooltip {
    tooltip().child(
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
            ),
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
) -> impl IntoElement {
    scroll().flex_1().min_w(px(0.0)).bg(bg_canvas()).child(
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
                                Phase::Gone => t(lang, "这条记录已经不在了", "That flow is gone").into(),
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
        .child(when(
            move || compose.composing.get(),
            move || compose_box(compose),
        ))
        .child(when(
            move || !shown.query.with(Vec::is_empty),
            move || header_section("查询参数", "Query", shown.query, folded, FOLD_QUERY),
        ))
        .child(when(
            move || !shown.diff.get().is_empty(),
            move || diff_card(shown, folded),
        ))
        .child(when(
            move || !shown.messages.with(Vec::is_empty),
            move || header_section("消息", "Messages", shown.messages, folded, FOLD_MESSAGES),
        ))
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
                        shown.response_note,
                        shown.response_language,
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
        .child(header_section(
            "请求头",
            "Request headers",
            shown.request_headers,
            folded,
            FOLD_REQ_HEADERS,
        ))
        .child(when(
            move || !shown.request_body.get().is_empty() || !shown.request_note.get().is_empty(),
            move || {
                body_section(
                    "请求 body",
                    "Request body",
                    shown.request_body,
                    shown.request_note,
                    shown.request_language,
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
                .text_size(px(11.5))
                .text_color(fg_muted()),
        )
        .child(
            when(
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
                                .text_size(px(12.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(status_err_fg()),
                        )
                },
            ),
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

fn header_section(
    zh: &'static str,
    en: &'static str,
    lines: Signal<Vec<HeaderLine>>,
    folded: Signal<u8>,
    bit: u8,
) -> impl IntoElement {
    view()
        .gap_1p5()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .px_1()
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
                view()
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(border_strong())
                    .bg(bg_card())
                    .overflow_hidden()
                    .child(
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
                                    .border_t(move || {
                                        if index.get() > 0 {
                                            px(1.0)
                                        } else {
                                            px(0.0)
                                        }
                                    })
                                    .border_t_color(border_subtle())
                                    .child(
                                        mono(move || line.with(|line| line.name.clone()))
                                            .w(px(176.0))
                                            .flex_shrink_0()
                                            .selectable(true)
                                            .number_of_lines(1)
                                            .ellipsize_mode(EllipsizeMode::Tail)
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
                    }),
                    )
            },
        ))
}

fn diff_card(shown: Shown, folded: Signal<u8>) -> impl IntoElement {
    view()
        .gap_1p5()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .px_1()
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
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(border_strong())
                    .h(px(320.0))
                    .overflow_hidden()
                    .child(
                        diff_view()
                            .old_value(move || SharedString::from(shown.diff_old.get()))
                            .new_value(move || SharedString::from(shown.diff_new.get()))
                            .layout(DiffLayout::Unified)
                            .language("plaintext")
                            .soft_wrap(true)
                            .font_size(Points(12.0))
                            .line_height(Points(18.0))
                            .size_full(),
                    )
            },
        ))
}

fn body_section(
    zh: &'static str,
    en: &'static str,
    body: Signal<String>,
    note: Signal<String>,
    code: Signal<String>,
    embed_image: bool,
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    folded: Signal<u8>,
    bit: u8,
) -> impl IntoElement {
    let image_on = move || embed_image && shown.pretty.get() && !shown.preview.get().is_empty();
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
                .child(pretty_switch(shown, controls, selected, baseline)),
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
        .child(when(move || !card_folded(folded, bit) && image_on(), move || {
            view()
                .px_3()
                .py_3()
                .gap_1p5()
                .child(
                    view()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(border_strong())
                        .bg(bg_code())
                        .overflow_hidden()
                        .child(
                            image(move || shown.preview.get())
                                .w_full()
                                .h(px(200.0))
                                .resize_mode(ResizeMode::Contain),
                        ),
                )
                .child(when(
                    move || !shown.image_caption.get().is_empty(),
                    move || muted_line(move || shown.image_caption.get()),
                ))
        }))
        .child(when(
            move || !card_folded(folded, bit) && !body.get().is_empty() && code.get().is_empty(),
            move || {
                let divided = image_on() || !note.get().is_empty();
                view()
                    .bg(bg_code())
                    .px_3p5()
                    .py_2p5()
                    .border_t(move || if divided { px(1.0) } else { px(0.0) })
                    .border_t_color(border_subtle())
                    .child(
                        mono(move || body.get())
                            .selectable(true)
                            .text_size(px(12.0))
                            .text_color(fg_primary()),
                    )
            },
        ))
        .child(when(
            move || !card_folded(folded, bit) && !body.get().is_empty() && !code.get().is_empty(),
            move || code_body(body, code),
        ))
}

fn editor_box_height(text: &str) -> f32 {
    let lines = text.lines().count().max(1) as f32;
    (lines * 18.0 + 16.0).clamp(96.0, 360.0)
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
            shown, controls.clone(), selected, baseline, true, "美化", "Pretty",
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
            if shown.pretty.get() == on {
                accent_blue()
            } else {
                bg_card()
            }
        })
        .cursor(Cursor::Pointer)
        .on_press(move || {
            if shown.pretty.get() == on {
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
                    if shown.pretty.get() == on {
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
                .child(action_item(tr("复制响应", "Copy response"), move || {
                    copy_part(&copy_response, selected.get(), CopyPart::Response);
                }))
                .child(action_item(tr("另存", "Save"), move || {
                    export_response(&save, selected.get());
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
                    move || baseline_mark.get().is_some_and(|id| selected_mark.get() == Some(id)),
                    tr("基准", "Baseline"),
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
                .w(px(6.0))
                .h(px(6.0))
                .rounded(px(99.0))
                .bg(move || {
                    if dot() {
                        accent_blue()
                    } else {
                        bg_card()
                    }
                }),
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
    text(label)
        .text_size(px(12.0))
        .text_color(fg_muted())
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
        .child(qr_panel(
            listen, lan_url, lan_choice, lan_addrs, qr_path,
        ))
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
    let picked_bg = picked.clone();
    let picked_dot = picked.clone();
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
            publish_lan(
                &listen.get(),
                &ip,
                lan_addrs,
                lan_choice,
                lan_url,
                qr_path,
            );
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
                .placeholder(present::DEFAULT_PORT.to_string())
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
    import_path: Signal<String>,
    import_controls: Arc<Controls>,
    rows: Signal<Vec<RowLine>>,
    flow_total: Signal<usize>,
    filter: Signal<String>,
    pretty: Signal<bool>,
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
            import_path,
            import_controls,
            rows,
            flow_total,
            filter,
            pretty,
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
        .h(px(22.0))
        .px_2()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .rounded(px(6.0))
        .bg(bg_control())
        .border_1()
        .border_color(border_control())
        .child(
            mono(move || SharedString::from(flow_count_now(rows, flow_total, filter)))
                .number_of_lines(1)
                .text_size(px(11.5))
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
                view()
                    .w(px(6.0))
                    .h(px(6.0))
                    .rounded(px(99.0))
                    .bg(move || {
                        if dot_proxy.get() {
                            accent_blue()
                        } else {
                            status_warn_fg()
                        }
                    })
            },
        ))
        .child(
            control_text(tr("更多", "More")).text_color(move || {
                if label_proxy.get() {
                    accent_blue_text()
                } else {
                    fg_secondary()
                }
            }),
        )
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
    import_path: Signal<String>,
    import_controls: Arc<Controls>,
    rows: Signal<Vec<RowLine>>,
    flow_total: Signal<usize>,
    filter: Signal<String>,
    pretty: Signal<bool>,
) -> impl IntoElement {
    view()
        .w(px(248.0))
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
        .child(menu_action(Glyph::Download, tr("导出", "Export"), export_har))
        .child(import_menu(import_path, import_controls.clone()))
        .child(menu_action(
            Glyph::Prompt,
            tr("复制代理环境", "Copy proxy env"),
            copy_env,
        ))
        .child(menu_rule())
        .child(proxy_menu_item(proxy_on, toggle_proxy))
        .child(menu_rule())
        .child(cert_menu_status(installed))
        .child(when(
            move || installed.get(),
            move || {
                let remove = remove.clone();
                menu_action(Glyph::Badge, tr("移除证书", "Remove certificate"), move || {
                    remove.as_ref()()
                })
            },
        ).otherwise(move || {
            let install = install.clone();
            menu_action(Glyph::Badge, tr("安装证书", "Install certificate"), move || {
                install.as_ref()()
            })
        }))
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
                Painter::new().fill_color(paint).fill_svg(
                    "M6.5 6.5 m-3 0 a3 3 0 1 0 6 0 a3 3 0 1 0 -6 0",
                )
            })
            .w(px(13.0))
            .h(px(13.0))
            .flex_shrink_0(),
        )
        .child(
            control_text(tr(zh, en)).text_color(move || {
                if mark.get() == choice {
                    accent_blue_text()
                } else {
                    fg_secondary()
                }
            }),
        )
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
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded(px(99.0))
                .bg(move || {
                    if dot_on.get() {
                        accent_blue()
                    } else {
                        fg_muted()
                    }
                }),
        )
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
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded(px(99.0))
                .bg(move || {
                    if dot_installed.get() {
                        status_ok_fg()
                    } else {
                        status_warn_fg()
                    }
                }),
        )
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
    view()
        .h(px(1.0))
        .mx(px(6.0))
        .bg(border_strong())
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
            || white(),
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
        path.to_string()
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
                    host: flow.host.clone(),
                    path: list_path(&flow.path, flow.error.as_deref(), failed, lang),
                    protocol: flow.protocol.label().to_string(),
                    size: format_bytes(flow.response_bytes as usize),
                    failed,
                    slow: present::is_slow(
                        flow.started,
                        flow.finished_at,
                        std::time::SystemTime::now(),
                    ),
                    pinned: flow.pinned,
                })
            })
            .collect()
    })
}

fn publish_detail(shown: &Shown, hub: &Hub, id: Option<u64>, lang: Lang, baseline: Option<u64>) {
    let Some(id) = id else {
        shown.phase.set(Phase::Empty);
        shown.preview.set(String::new());
        shown.image_caption.set(String::new());
        shown.diff.set(String::new());
        shown.diff_old.set(String::new());
        shown.diff_new.set(String::new());
        shown.request_language.set(String::new());
        shown.response_language.set(String::new());
        shown.pinned.set(false);
        return;
    };
    let snapshot = hub.with_session(|session| session.get(id).cloned());
    let Some(flow) = snapshot else {
        shown.phase.set(Phase::Gone);
        shown.preview.set(String::new());
        shown.image_caption.set(String::new());
        shown.diff.set(String::new());
        shown.diff_old.set(String::new());
        shown.diff_new.set(String::new());
        shown.request_language.set(String::new());
        shown.response_language.set(String::new());
        shown.pinned.set(false);
        return;
    };
    let (diff_old, diff_new) = match baseline {
        Some(base) if base != id => hub
            .with_session(|session| session.get(base).cloned())
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
        flow.request_encoding_error.as_deref(),
        request_type,
        pretty,
        lang,
    );
    let response = display_body(
        &flow.response_body,
        flow.response_truncated,
        flow.response_encoding_error.as_deref(),
        response_type,
        pretty,
        lang,
    );
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
    shown.protocol.set(flow.protocol.label().to_string());
    shown.status.set(flow.status);
    shown.error.set(flow.error.clone().unwrap_or_default());
    shown.request_headers.set(header_lines(&flow.request_headers, pretty));
    shown.response_headers.set(header_lines(&flow.response_headers, pretty));
    shown.request_language.set(
        present::editor_language(request_type, &request.text)
            .unwrap_or("")
            .to_string(),
    );
    shown.response_language.set(
        present::editor_language(response_type, &response.text)
            .unwrap_or("")
            .to_string(),
    );
    shown.request_body.set(request.text);
    shown.response_body.set(response.text);
    shown.request_note.set(request.note);
    shown.response_note.set(response.note);
    shown.preview.set(preview);
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
        if pretty {
            if let Some(rows) = present::pretty_cookie_rows(name, value) {
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
}

fn display_body(
    body: &[u8],
    truncated: bool,
    encoding_error: Option<&str>,
    content_type: Option<&str>,
    pretty: bool,
    lang: Lang,
) -> BodyText {
    let mut presented = body_text(body, truncated, encoding_error, lang);
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
            presented
                .note
                .push_str(t(lang, "界面只显示前 256KB。", "Showing only the first 256 KB."));
        }
    } else {
        presented.text = pretty;
    }
}

fn body_text(body: &[u8], truncated: bool, encoding_error: Option<&str>, lang: Lang) -> BodyText {
    let mut note = String::new();
    if let Some(error) = encoding_error {
        note.push_str(t(
            lang,
            "解压失败，以下是原始字节。",
            "Decompression failed; showing the raw bytes. ",
        ));
        note.push_str(error);
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
        let out = hex_display(&body[..shown]);
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
    BodyText { text, note }
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
    *controls.status.lock().unwrap_or_else(|err| err.into_inner()) = message;
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
        let running = *controls.running.lock().unwrap_or_else(|err| err.into_inner());
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
    *controls
        .listen
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = String::new();
    let applied = lock_bool(&controls.proxy_applied);
    set_bool(&controls.proxy_applied, false);
    set_bool(&controls.running, false);
    if !applied {
        set_status(controls, StatusMsg::Stopped);
        return;
    }
    match sysproxy::disable() {
        Ok(()) => set_status(controls, StatusMsg::StoppedRestored),
        Err(err) => set_status(controls, StatusMsg::StopRestoreFailed(format!("{err:#}"))),
    }
}

fn toggle_system_proxy(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let _ops = lock_ops(&controls);
        let on = !lock_bool(&controls.use_system_proxy);
        set_bool(&controls.use_system_proxy, on);
        let running = lock_bool(&controls.running);
        if let Err(err) = store_use_system_proxy(on) {
            if !running {
                set_status(&controls, StatusMsg::ProxyOptionUnsaved(err.to_string()));
                return;
            }
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
    config::load(&ca::data_dir())
}

fn save_config(change: impl FnOnce(&mut config::Config)) -> std::io::Result<()> {
    config::update(&ca::data_dir(), change)
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
    if flow.host.is_empty() {
        return;
    }
    filter.set(flow.host);
}

fn mute_host(controls: &Controls, id: Option<u64>, filter: Signal<String>) {
    let Some(flow) = selected_flow(controls, id) else {
        return;
    };
    let host = flow.host.to_ascii_lowercase();
    if host.is_empty() || host.split_whitespace().nth(1).is_some() {
        return;
    }
    let token = if host.contains(':') {
        format!("mute:{host}")
    } else {
        format!("-{host}")
    };
    let current = filter.get();
    if current
        .split_whitespace()
        .any(|part| part.eq_ignore_ascii_case(&token))
    {
        return;
    }
    if current.trim().is_empty() {
        filter.set(token);
    } else {
        filter.set(format!("{current} {token}"));
    }
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
            let binary = !flow.request_body.is_empty()
                && std::str::from_utf8(&flow.request_body).is_err();
            (present::curl_command(&flow), StatusMsg::CopiedCurl { binary })
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

/// Display rows are `0001  ` plus 16 bytes. Copy uses `hex_dump`, which has no row numbers.
fn hex_display(body: &[u8]) -> String {
    let mut out = String::with_capacity(body.len() * 4);
    for (index, byte) in body.iter().enumerate() {
        if index % 16 == 0 {
            if index > 0 {
                out.push('\n');
            }
            let row = index / 16 + 1;
            out.push_str(&format!("{row:04}  "));
        }
        out.push_str(&format!("{byte:02x} "));
    }
    out
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

fn export_response(controls: &Controls, id: Option<u64>) {
    let Some(flow) = selected_flow(controls, id) else {
        return;
    };
    if flow.response_body.is_empty() {
        set_status(controls, StatusMsg::EmptyResponse);
        return;
    }
    let ext = present::content_type(&flow.response_headers)
        .and_then(present::image_extension)
        .unwrap_or_else(|| body_extension(&flow.response_headers, &flow.response_body));
    let dir = download_dir().join("zniff");
    if let Err(err) = std::fs::create_dir_all(&dir) {
        set_status(controls, StatusMsg::SaveFailed(err.to_string()));
        return;
    }
    let path = dir.join(format!("{}.{}", flow.id, ext));
    if let Err(err) = std::fs::write(&path, &flow.response_body) {
        set_status(controls, StatusMsg::SaveFailed(err.to_string()));
        return;
    }
    let text = path.display().to_string();
    set_clipboard(text.clone());
    set_status(controls, StatusMsg::Saved(text));
}

fn home_dir() -> std::path::PathBuf {
    let key = if cfg!(target_os = "windows") {
        "USERPROFILE"
    } else {
        "HOME"
    };
    std::env::var(key)
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var("HOME").ok())
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// macOS and Windows use `~/Downloads`. Linux prefers `XDG_DOWNLOAD_DIR`.
fn download_dir() -> std::path::PathBuf {
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(dir) = read_xdg_download() {
        return dir;
    }
    home_dir().join("Downloads")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn read_xdg_download() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let text = std::fs::read_to_string(
        std::path::PathBuf::from(&home).join(".config/user-dirs.dirs"),
    )
    .ok()?;
    xdg_download_dir(&home, &text)
}

/// `XDG_DOWNLOAD_DIR="$HOME/Downloads"` from `user-dirs.dirs`.
/// Linux calls this. Other platforms keep it for the parser test.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
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
    let flow = controls.hub.with_session(|session| session.get(id).cloned());
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
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() == flow.response_body.len() as u64 {
            return path.to_string_lossy().into_owned();
        }
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

fn compose_box(compose: Compose) -> impl IntoElement {
    view()
        .gap_1p5()
        .child(view().px_1().child(section_title({
            let langs = ui_langs();
            move || -> SharedString {
                if compose.response.get() {
                    t(ui_lang(langs), "编辑响应", "Edit response").into()
                } else {
                    t(ui_lang(langs), "编辑重发", "Edit and resend").into()
                }
            }
        })))
        .child(
            view()
                .flex_row()
                .gap_1p5()
                .child(when(
                    move || compose.response.get(),
                    move || {
                        view().w(px(72.0)).flex_shrink_0().child(compose_input(
                            compose.status,
                            "200",
                            30.0,
                            false,
                        ))
                    },
                ))
                .child(
                    view()
                        .w(px(88.0))
                        .flex_shrink_0()
                        .child(compose_input(compose.method, "POST", 30.0, false)),
                )
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(compose_input_on(
                            compose.url,
                            tr(
                                "https://example.com/path，或粘贴 curl",
                                "https://example.com/path, or paste curl",
                            ),
                            30.0,
                            false,
                            move |text| apply_url_edit(compose, text),
                        )),
                ),
        )
        .child(compose_editor(
            compose.headers,
            tr("一行一个 Name: value", "One Name: value per line"),
            120.0,
            "plaintext",
        ))
        .child(compose_editor(compose.body, tr("body", "body"), 180.0, "json"))
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
    if looks_like_curl(text) {
        if let Some(parsed) = present::parse_curl(text) {
            compose.method.set(parsed.method);
            compose.url.set(parsed.url);
            compose.headers.set(header_text(&parsed.headers));
            compose.body.set(parsed.body);
            return;
        }
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

fn import_menu(path_text: Signal<String>, controls: Arc<Controls>) -> impl IntoElement {
    let import_controls = controls;
    view()
        .gap_1()
        .px_1()
        .py_0p5()
        .child(
            input()
                .value(path_text)
                .placeholder("import.har")
                .placeholder_color(move || color(fg_muted()))
                .w_full()
                .h(px(28.0))
                .px_2()
                .rounded(px(6.0))
                .bg(bg_code())
                .border_1()
                .border_color(border_control())
                .focus(|style| style.border_color(accent_blue()))
                .text_size(px(12.0))
                .text_color(fg_primary())
                .on_change(move |text: &str| path_text.set(text.to_string())),
        )
        .child(menu_action(
            Glyph::List,
            tr("导入 HAR", "Import HAR"),
            move || import_har_file(&import_controls, &path_text.get()),
        ))
}

fn begin_compose(controls: &Controls, id: Option<u64>, compose: Compose) {
    let Some(flow) = selected_flow(controls, id) else {
        set_status(controls, StatusMsg::NeedSelection);
        return;
    };
    let response = controls.hub.paused().is_some_and(|(paused_id, side)| {
        id == Some(paused_id) && side == BreakSide::Response
    });
    compose.response.set(response);
    compose.method.set(flow.method);
    compose.url.set(flow.url);
    if response {
        compose.status.set(flow.status.map(|code| code.to_string()).unwrap_or_default());
        compose.headers.set(header_text(&flow.response_headers));
        compose.body.set(String::from_utf8_lossy(&flow.response_body).into_owned());
    } else {
        compose.status.set(String::new());
        compose.headers.set(header_text(&flow.request_headers));
        compose.body.set(String::from_utf8_lossy(&flow.request_body).into_owned());
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

fn export_har_file(controls: &Controls) {
    let value = controls.hub.export_har();
    let dir = ca::data_dir().join("exports");
    if let Err(err) = std::fs::create_dir_all(&dir) {
        set_status(controls, StatusMsg::ExportFailed(err.to_string()));
        return;
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("zniff-{secs}.har"));
    let bytes = match serde_json::to_vec_pretty(&value) {
        Ok(bytes) => bytes,
        Err(err) => {
            set_status(controls, StatusMsg::ExportFailed(err.to_string()));
            return;
        }
    };
    if let Err(err) = std::fs::write(&path, bytes) {
        set_status(controls, StatusMsg::ExportFailed(err.to_string()));
        return;
    }
    set_clipboard(path.display().to_string());
    set_status(controls, StatusMsg::Exported(path.display().to_string()));
}

fn import_dropped(controls: &Controls, event: &FileDropEvent, lang: Lang) {
    let mut found = false;
    for uri in &event.uris {
        let Some(path) = dropped_har_path(uri) else {
            continue;
        };
        found = true;
        import_har_file(controls, &path);
    }
    if !found {
        set_status(
            controls,
            StatusMsg::ImportFailed(
                t(lang, "放下的文件不是 HAR", "Dropped file is not a HAR").to_string(),
            ),
        );
    }
}

/// `file:` URL of a `.har`, or nothing. Encoded names are decoded. A Windows
/// `file:///C:/...` drops the extra slash. Other schemes are ignored.
fn dropped_har_path(uri: &str) -> Option<String> {
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
    let har = std::path::Path::new(&path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("har"));
    har.then_some(path)
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
        && let Some(found) = addrs.iter().find(|addr| addr.ip == ip && lan_rank(addr) < 9)
    {
        return Some(found.clone());
    }
    if let Some(found) = addrs.iter().filter(|addr| lan_rank(addr) < 9).min_by(|left, right| {
        lan_rank(left)
            .cmp(&lan_rank(right))
            .then(left.name.cmp(&right.name))
            .then(left.ip.octets().cmp(&right.ip.octets()))
    }) {
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
            let mut address = &adapter.IpAddressList as *const windows_sys::Win32::NetworkManagement::IpHelper::IP_ADDR_STRING;
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
    let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
    let raw = bytes[..end].iter().map(|byte| *byte as u8).collect::<Vec<_>>();
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

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::{LanAddr, cert_url, choose_lan, usable_v4};

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
        assert_eq!(
            absolute,
            Some(std::path::PathBuf::from("/srv/downloads"))
        );
        assert!(super::xdg_download_dir("/home/ada", "XDG_DOWNLOAD_DIR=\"\"\n").is_none());
        assert!(
            super::xdg_download_dir("/home/ada", "XDG_DESKTOP_DIR=\"$HOME/Desktop\"\n").is_none()
        );
    }

    #[test]
    fn hex_display_numbers_each_row_and_copy_does_not() {
        let body: Vec<u8> = (0..20).collect();
        let shown = super::hex_display(&body);
        let mut lines = shown.lines();
        let first = lines.next().unwrap();
        let second = lines.next().unwrap();
        assert!(first.starts_with("0001  "), "{first}");
        assert!(second.starts_with("0002  "), "{second}");
        assert!(first.split_whitespace().skip(1).count() == 16);
        let copied = super::hex_dump(&body);
        assert!(!copied.contains("0001"), "{copied}");
        assert!(copied.starts_with("00 01 02 "));
    }

    #[test]
    fn body_text_cuts_on_a_char_boundary() {
        let mut raw = vec![b'a'; 256 * 1024 - 2];
        raw.extend_from_slice("不".as_bytes());
        raw.extend(std::iter::repeat_n(b'b', 40));
        let shown = super::body_text(&raw, false, None, super::Lang::Zh);
        assert_eq!(shown.text.len(), 256 * 1024 - 2);
        assert!(shown.text.is_char_boundary(shown.text.len()));
        assert!(shown.text.chars().all(|ch| ch == 'a'));
        let rest = raw.len() - shown.text.len();
        assert!(shown.note.contains(&rest.to_string()), "{}", shown.note);

        let ascii = vec![b'x'; 256 * 1024 + 10];
        let ascii_shown = super::body_text(&ascii, false, None, super::Lang::En);
        assert_eq!(ascii_shown.text.len(), 256 * 1024);
        assert!(ascii_shown.note.contains("10 more bytes"), "{}", ascii_shown.note);
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
        assert!(config.show_status && config.show_path && config.show_gap && config.show_host);
        assert!(!config.show_protocol && !config.show_size);
        let columns = super::ListColumns::from_config(&config)
            .toggle(super::ListColumn::Protocol)
            .toggle(super::ListColumn::Host);
        columns.write(&mut config);
        assert!(config.show_protocol);
        assert!(!config.show_host);
        assert!(config.show_path);
        assert_eq!(super::ListColumns::from_config(&config), columns);
        assert_eq!(super::clamp_list_px(10.0), crate::config::LIST_WIDTH_MIN as f32);
        assert_eq!(super::clamp_list_px(5_000.0), crate::config::LIST_WIDTH_MAX as f32);
        assert_eq!(super::clamp_list_px(640.6), 641.0);
        assert_eq!(super::clamp_list_px(-20.0), crate::config::LIST_WIDTH_MIN as f32);
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
            super::list_path("/", Some("证书被拒绝"), true, super::Lang::Zh),
            "证书被拒绝"
        );
        assert_eq!(
            super::list_path(":443", Some("证书被拒绝"), true, super::Lang::En),
            "Certificate rejected"
        );
        assert_eq!(
            super::list_path("/", None, true, super::Lang::En),
            ""
        );
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
    }
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

