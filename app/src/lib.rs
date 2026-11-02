//! zniff: a desktop HTTP(S) capture app. The window is an nsh Rust view.
//! The proxy runs on its own tokio runtime in this process.

mod ca;
mod client_hello;
mod decompress;
mod proxy;
mod session;
mod sysproxy;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nsh_base::theme;
use nsh_core::ui::prelude::*;
use nsh_core::EllipsizeMode;

use crate::ca::Ca;
use crate::proxy::{Capture, Hub};

pub use sysproxy::restore_if_dirty;

struct Controls {
    hub: Arc<Hub>,
    capture: Mutex<Option<Capture>>,
    status: Mutex<String>,
    running: Mutex<bool>,
    listen: Mutex<String>,
    ca_label: Mutex<String>,
}

/// The app's UI set (`nsh.json` `"rust"`).
#[native_module("zniff/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

const COL_METHOD: f32 = 68.0;
const COL_STATUS: f32 = 48.0;
const COL_PROTOCOL: f32 = 84.0;

#[derive(Clone, PartialEq)]
struct RowLine {
    id: u64,
    method: String,
    status: Option<u16>,
    protocol: String,
    host: String,
    path: String,
    failed: bool,
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
    request_note: Signal<String>,
    response_note: Signal<String>,
}

/// The root view: a flow list and the selected flow.
#[component]
pub fn App() -> impl IntoElement {
    let status = signal(String::from("未开始"));
    let running = signal(false);
    let ca_label = signal(ca_text());
    let memory = signal(String::from("0 B"));
    let dropped = signal(0_u64);
    let filter = signal(String::new());
    let selected = signal(None::<u64>);
    let rows = signal(Vec::<RowLine>::new());
    let listen = signal(String::new());
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
        request_note: signal(String::new()),
        response_note: signal(String::new()),
    };
    let controls = open_controls();

    let controls_tick = controls.clone();
    let filter_tick = filter;
    let selected_tick = selected;
    let rows_tick = rows;
    let shown_tick = shown;
    let seen_gen = stored(0_u64);
    let seen_filter = stored(String::new());
    let seen_selected = stored(None::<u64>);
    set_interval(Duration::from_millis(16), move || {
        status.set(lock_string(&controls_tick.status));
        running.set(*controls_tick.running.lock().unwrap_or_else(|err| err.into_inner()));
        listen.set(lock_string(&controls_tick.listen));
        ca_label.set(lock_string(&controls_tick.ca_label));
        let generation = controls_tick.hub.generation();
        let filter_now = filter_tick.get();
        let selected_now = selected_tick.get();
        if generation != seen_gen.get()
            || filter_now != seen_filter.get()
            || selected_now != seen_selected.get()
        {
            seen_gen.replace(generation);
            seen_filter.replace(filter_now.clone());
            seen_selected.replace(selected_now);
            let lines = row_lines(&controls_tick.hub, &filter_now);
            rows_tick.set(lines);
            memory.set(format_bytes(
                controls_tick.hub.with_session(|session| session.memory_bytes()),
            ));
            dropped.set(controls_tick.hub.with_session(|session| session.dropped()));
            publish_detail(&shown_tick, &controls_tick.hub, selected_now);
        }
    });

    let controls_start = controls.clone();
    let controls_ca = controls.clone();
    let controls_remove = controls.clone();

    view()
        .flex_1()
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .child(toolbar(
            status,
            running,
            ca_label,
            move || install_ca(controls_ca.clone()),
            move || remove_ca(controls_remove.clone()),
            move || toggle_capture(controls_start.clone()),
        ))
        .child(
            view()
                .flex_1()
                .flex_row()
                .child(flow_list(rows, selected, filter))
                .child(detail_pane(shown)),
        )
        .child(footer(listen, memory, dropped))
}

fn toolbar(
    status: Signal<String>,
    running: Signal<bool>,
    ca_label: Signal<String>,
    install: impl Fn() + 'static,
    remove: impl Fn() + 'static,
    toggle: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .px_3()
        .h(px(52.0))
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(text("zniff").text_size(px(16.0)).font_weight(FontWeight::SEMIBOLD))
        .child(
            text(move || status.get())
                .flex_1()
                .number_of_lines(1)
                .text_size(px(13.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(chip(move || ca_label.get()))
        .child(quiet_button("安装证书", install))
        .child(quiet_button("移除证书", remove))
        .child(primary_button(
            move || -> SharedString {
                if running.get() {
                    "停止".into()
                } else {
                    "开始".into()
                }
            },
            move || running.get(),
            toggle,
        ))
}

fn flow_list(
    rows: Signal<Vec<RowLine>>,
    selected: Signal<Option<u64>>,
    filter: Signal<String>,
) -> impl IntoElement {
    view()
        .w(px(560.0))
        .flex_shrink_0()
        .border_r_1()
        .border_r_color(move || theme().border)
        .child(
            view().px_3().py_2().child(
                input()
                    .placeholder("过滤主机名")
                    .placeholder_color(move || color(theme().muted_foreground))
                    .w_full()
                    .h(px(32.0))
                    .px_3()
                    .rounded(px(6.0))
                    .bg(move || theme().input)
                    .border_1()
                    .border_color(move || theme().border)
                    .text_size(px(13.0))
                    .on_change(move |text: &str| filter.set(text.to_string())),
            ),
        )
        .child(column_header())
        .child(view().flex_1().child(
            when(
                move || rows.with(Vec::is_empty),
                move || list_empty(filter),
            )
            .otherwise(move || flow_rows(rows, selected)),
        ))
}

fn list_empty(filter: Signal<String>) -> impl IntoElement {
    empty_copy(move || -> SharedString {
        if filter.get().is_empty() {
            "还没有流量".into()
        } else {
            "没有匹配的主机".into()
        }
    })
}

fn flow_rows(rows: Signal<Vec<RowLine>>, selected: Signal<Option<u64>>) -> impl IntoElement {
    virtual_list(
        move || rows.with(Vec::len),
        move |at| flow_row(rows, selected, at),
    )
    .keys(move || rows.with(|rows| rows.iter().map(|row| row.id).collect::<Vec<_>>()))
    .estimated_item_size(px(34.0))
    .flex_1()
}

fn column_header() -> impl IntoElement {
    view()
        .h(px(28.0))
        .px_3()
        .gap_2()
        .flex_row()
        .items_center()
        .bg(move || theme().muted)
        .border_b_1()
        .border_b_color(move || theme().border)
        .border_l(px(2.0))
        .border_l_color(move || theme().muted)
        .child(header_label("方法").w(px(COL_METHOD)))
        .child(header_label("状态").w(px(COL_STATUS)))
        .child(header_label("协议").w(px(COL_PROTOCOL)))
        .child(header_label("主机").flex_1())
        .child(header_label("路径").flex_1())
}

fn header_label(label: &'static str) -> Text {
    text(label)
        .number_of_lines(1)
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || theme().muted_foreground)
}

fn flow_row(rows: Signal<Vec<RowLine>>, selected: Signal<Option<u64>>, at: Position) -> impl IntoElement {
    let row_id = move || rows.with(|rows| rows.get(at.index()).map(|row| row.id));
    view()
        .h(px(34.0))
        .px_3()
        .gap_2()
        .flex_row()
        .items_center()
        .border_b_1()
        .border_b_color(move || theme().border)
        .border_l(px(2.0))
        .border_l_color(move || {
            if selected.get() == row_id() {
                theme().primary
            } else {
                theme().background
            }
        })
        .overflow_hidden()
        .cursor(Cursor::Pointer)
        .bg(move || {
            if selected.get() == row_id() {
                theme().accent
            } else {
                theme().background
            }
        })
        .hover(|style| {
            style.bg(move || {
                if selected.get() == row_id() {
                    theme().accent
                } else {
                    theme().muted
                }
            })
        })
        .on_press(move || selected.set(row_id()))
        .child(row_text(rows, at, |row| row.method.clone(), COL_METHOD, false))
        .child(status_text(rows, at))
        .child(row_text(rows, at, |row| row.protocol.clone(), COL_PROTOCOL, false))
        .child(flex_text(rows, at, |row| row.host.clone()))
        .child(flex_text(rows, at, |row| row.path.clone()))
}

fn row_text(
    rows: Signal<Vec<RowLine>>,
    at: Position,
    pick: fn(&RowLine) -> String,
    width: f32,
    flex: bool,
) -> Text {
    let mut cell = text(move || rows.with(|rows| rows.get(at.index()).map(pick).unwrap_or_default()))
        .number_of_lines(1)
        .ellipsize_mode(EllipsizeMode::Middle)
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM);
    cell = if flex {
        cell.flex_1().min_w(px(0.0))
    } else {
        cell.w(px(width)).flex_shrink_0()
    };
    cell
}

fn flex_text(rows: Signal<Vec<RowLine>>, at: Position, pick: fn(&RowLine) -> String) -> Text {
    text(move || rows.with(|rows| rows.get(at.index()).map(pick).unwrap_or_default()))
        .flex_1()
        .min_w(px(0.0))
        .number_of_lines(1)
        .ellipsize_mode(EllipsizeMode::Tail)
        .text_size(px(12.0))
}

fn status_text(rows: Signal<Vec<RowLine>>, at: Position) -> Text {
    text(move || {
        rows.with(|rows| {
            rows.get(at.index())
                .map(|row| {
                    row.status
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "…".into())
                })
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
            Some(row) if row.failed || row.status.is_some_and(|code| code >= 400) => theme().destructive,
            Some(row) if row.status.is_some_and(|code| (200..300).contains(&code)) => theme().primary,
            _ => theme().muted_foreground,
        }
    })
}

fn detail_pane(shown: Shown) -> impl IntoElement {
    scroll().flex_1().child(
        when(
            move || shown.phase.get() == Phase::Ready,
            move || ready_detail(shown),
        )
        .otherwise(move || {
            empty_copy(move || -> SharedString {
                match shown.phase.get() {
                    Phase::Gone => "这条 flow 已经因为内存上限被丢掉。".into(),
                    _ => "选一条流量查看请求头、响应头和 body。".into(),
                }
            })
        }),
    )
}

fn ready_detail(shown: Shown) -> impl IntoElement {
    view()
        .child(summary(shown))
        .child(header_section("请求头", shown.request_headers))
        .child(body_section("请求 body", shown.request_body, shown.request_note))
        .child(header_section("响应头", shown.response_headers))
        .child(body_section("响应 body", shown.response_body, shown.response_note))
}

fn summary(shown: Shown) -> impl IntoElement {
    view()
        .px_3()
        .py_3()
        .gap_2()
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    view()
                        .px_2()
                        .py_1()
                        .rounded(px(6.0))
                        .bg(move || theme().secondary)
                        .child(
                            text(move || shown.method.get())
                                .text_size(px(12.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().secondary_foreground),
                        ),
                )
                .child(
                    text(move || match shown.status.get() {
                        Some(code) => code.to_string(),
                        None => "等待响应".into(),
                    })
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || match shown.status.get() {
                        Some(code) if code >= 400 => theme().destructive,
                        Some(code) if (200..300).contains(&code) => theme().primary,
                        _ => theme().muted_foreground,
                    }),
                )
                .child(
                    text(move || shown.protocol.get())
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            mono(move || shown.url.get())
                .selectable(true)
                .text_size(px(13.0)),
        )
        .child(
            when(
                move || !shown.error.get().is_empty(),
                move || {
                    text(move || shown.error.get())
                        .selectable(true)
                        .text_size(px(12.0))
                        .text_color(move || theme().destructive)
                },
            ),
        )
}

fn header_section(title: &'static str, lines: Signal<Vec<HeaderLine>>) -> impl IntoElement {
    view()
        .px_3()
        .py_3()
        .gap_2()
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(section_title(title))
        .child(
            when(
                move || lines.with(Vec::is_empty),
                || muted_line("无"),
            )
            .otherwise(move || {
                each(
                    lines,
                    |line| line.key,
                    |line, _| {
                        view()
                            .flex_row()
                            .gap_3()
                            .py_2()
                            .border_b_1()
                            .border_b_color(move || theme().border)
                            .child(
                                mono(move || line.with(|line| line.name.clone()))
                                    .w(px(168.0))
                                    .flex_shrink_0()
                                    .number_of_lines(1)
                                    .ellipsize_mode(EllipsizeMode::Tail)
                                    .text_size(px(12.0))
                                    .text_color(move || theme().muted_foreground),
                            )
                            .child(
                                mono(move || line.with(|line| line.value.clone()))
                                    .flex_1()
                                    .selectable(true)
                                    .text_size(px(12.0)),
                            )
                    },
                )
            }),
        )
}

fn body_section(title: &'static str, body: Signal<String>, note: Signal<String>) -> impl IntoElement {
    view()
        .px_3()
        .py_3()
        .gap_2()
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(section_title(title))
        .child(
            when(
                move || !note.get().is_empty(),
                move || {
                    text(move || note.get())
                        .text_size(px(12.0))
                        .text_color(move || theme().destructive)
                },
            ),
        )
        .child(
            view()
                .bg(move || theme().muted)
                .rounded(px(8.0))
                .px_3()
                .py_2()
                .child(
                    mono(move || {
                        let text = body.get();
                        if text.is_empty() {
                            "（空）".into()
                        } else {
                            text
                        }
                    })
                    .selectable(true)
                    .text_size(px(12.0)),
                ),
        )
}

fn section_title(label: &'static str) -> Text {
    text(label)
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || theme().muted_foreground)
}

fn muted_line(label: &'static str) -> Text {
    text(label)
        .text_size(px(12.0))
        .text_color(move || theme().muted_foreground)
}

fn empty_copy(label: impl IntoReactive<SharedString> + 'static) -> impl IntoElement {
    view()
        .flex_1()
        .items_center()
        .justify_center()
        .py_3()
        .child(
            text(label)
                .text_size(px(13.0))
                .text_color(move || theme().muted_foreground),
        )
}

fn footer(listen: Signal<String>, memory: Signal<String>, dropped: Signal<u64>) -> impl IntoElement {
    view()
        .h(px(28.0))
        .px_3()
        .flex_row()
        .items_center()
        .border_t_1()
        .border_t_color(move || theme().border)
        .child(
            text(move || {
                if listen.get().is_empty() {
                    "监听尚未开始".into()
                } else {
                    format!("监听 {}", listen.get())
                }
            })
            .flex_1()
            .text_size(px(12.0))
            .text_color(move || theme().muted_foreground),
        )
        .child(
            text(move || format!("{} · 已丢弃 {}", memory.get(), dropped.get()))
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
}

fn chip(label: impl IntoReactive<SharedString>) -> impl IntoElement {
    view()
        .px_2()
        .py_1()
        .rounded(px(6.0))
        .bg(move || theme().muted)
        .child(
            text(label)
                .number_of_lines(1)
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
}

fn quiet_button(label: &'static str, on_press: impl Fn() + 'static) -> impl IntoElement {
    view()
        .px_3()
        .py_2()
        .rounded(px(8.0))
        .bg(move || theme().secondary)
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.75))
        .on_press(on_press)
        .child(
            text(label)
                .text_size(px(13.0))
                .text_color(move || theme().secondary_foreground),
        )
}

fn primary_button(
    label: impl IntoReactive<SharedString>,
    danger: impl Fn() -> bool + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .px_3()
        .py_2()
        .rounded(px(8.0))
        .bg(move || {
            if danger() {
                theme().destructive
            } else {
                theme().primary
            }
        })
        .cursor(Cursor::Pointer)
        .active(|style| style.opacity(0.75))
        .on_press(on_press)
        .child(
            text(label)
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().primary_foreground),
        )
}

fn mono(content: impl IntoReactive<SharedString>) -> Text {
    text(content).font_family(move || theme().font_mono.clone().unwrap_or_else(|| "Menlo".into()))
}

fn row_lines(hub: &Hub, filter: &str) -> Vec<RowLine> {
    hub.with_session(|session| {
        session
            .ids_matching(filter)
            .into_iter()
            .filter_map(|id| {
                let flow = session.get(id)?;
                Some(RowLine {
                    id,
                    method: flow.method.clone(),
                    status: flow.status,
                    protocol: flow.protocol.label().to_string(),
                    host: flow.host.clone(),
                    path: flow.path.clone(),
                    failed: flow.error.is_some() && flow.status.is_none(),
                })
            })
            .collect()
    })
}

fn publish_detail(shown: &Shown, hub: &Hub, id: Option<u64>) {
    let Some(id) = id else {
        shown.phase.set(Phase::Empty);
        return;
    };
    let snapshot = hub.with_session(|session| session.get(id).cloned());
    let Some(flow) = snapshot else {
        shown.phase.set(Phase::Gone);
        return;
    };
    let request = body_text(
        &flow.request_body,
        flow.request_truncated,
        flow.request_encoding_error.as_deref(),
    );
    let response = body_text(
        &flow.response_body,
        flow.response_truncated,
        flow.response_encoding_error.as_deref(),
    );
    shown.method.set(flow.method);
    shown.url.set(flow.url);
    shown.protocol.set(flow.protocol.label().to_string());
    shown.status.set(flow.status);
    shown.error.set(flow.error.unwrap_or_default());
    shown.request_headers.set(header_lines(&flow.request_headers));
    shown.response_headers.set(header_lines(&flow.response_headers));
    shown.request_body.set(request.text);
    shown.response_body.set(response.text);
    shown.request_note.set(request.note);
    shown.response_note.set(response.note);
    shown.phase.set(Phase::Ready);
}

fn header_lines(headers: &[(String, String)]) -> Vec<HeaderLine> {
    headers
        .iter()
        .enumerate()
        .map(|(index, (name, value))| HeaderLine {
            key: index as u32,
            name: name.clone(),
            value: value.clone(),
        })
        .collect()
}

struct BodyText {
    text: String,
    note: String,
}

fn body_text(body: &[u8], truncated: bool, encoding_error: Option<&str>) -> BodyText {
    let mut note = String::new();
    if let Some(error) = encoding_error {
        note.push_str("解压失败，以下是原始字节。");
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
            note.push_str(&format!(
                "界面只显示前 256KB，记录里还有 {} 字节。",
                text.len() - DISPLAY_CAP
            ));
            text[..DISPLAY_CAP].to_string()
        } else {
            text.to_string()
        }
    } else {
        const HEX_CAP: usize = 4096;
        let shown = body.len().min(HEX_CAP);
        let mut out = String::new();
        for (index, byte) in body[..shown].iter().enumerate() {
            if index > 0 && index % 16 == 0 {
                out.push('\n');
            }
            out.push_str(&format!("{byte:02x} "));
        }
        if body.len() > shown {
            if !note.is_empty() {
                note.push('\n');
            }
            note.push_str(&format!("十六进制只显示前 {shown} 字节，共 {} 字节。", body.len()));
        }
        out
    };
    if truncated {
        if !note.is_empty() {
            note.push('\n');
        }
        note.push_str("body 已超过 1 MB，记录被截断，转发仍继续。");
    }
    BodyText { text, note }
}

fn open_controls() -> Arc<Controls> {
    match Ca::open_default().and_then(|ca| Hub::new(Arc::new(ca))) {
        Ok(hub) => Arc::new(Controls {
            hub: Arc::new(hub),
            capture: Mutex::new(None),
            status: Mutex::new("未开始".into()),
            running: Mutex::new(false),
            listen: Mutex::new(String::new()),
            ca_label: Mutex::new(ca_text()),
        }),
        Err(err) => {
            // A hub is required for the view. If the CA cannot be created,
            // build one in a temp directory so the window can still explain
            // the failure.
            let dir = std::env::temp_dir().join("zniff-fallback");
            let ca = Arc::new(Ca::open(&dir).expect("临时证书目录"));
            let hub = Hub::new(ca).expect("hub");
            Arc::new(Controls {
                hub: Arc::new(hub),
                capture: Mutex::new(None),
                status: Mutex::new(format!("证书目录不可用: {err:#}")),
                running: Mutex::new(false),
                listen: Mutex::new(String::new()),
                ca_label: Mutex::new("证书未安装".into()),
            })
        }
    }
}

fn ca_text() -> String {
    if ca::is_installed() {
        "证书已安装".into()
    } else {
        "证书未安装".into()
    }
}

fn install_ca(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let ca = controls.hub.ca.clone();
        match ca::install(&ca) {
            Ok(message) => set_ca(&controls, message),
            Err(err) => set_ca(&controls, format!("安装失败: {err:#}")),
        }
    });
}

fn remove_ca(controls: Arc<Controls>) {
    std::thread::spawn(move || {
        let ca = controls.hub.ca.clone();
        match ca::remove(&ca) {
            Ok(message) => set_ca(&controls, message),
            Err(err) => set_ca(&controls, format!("移除失败: {err:#}")),
        }
    });
}

fn set_ca(controls: &Controls, message: String) {
    *controls.ca_label.lock().unwrap_or_else(|err| err.into_inner()) = message;
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
    {
        let mut status = controls.status.lock().unwrap_or_else(|err| err.into_inner());
        *status = "正在启动代理…".into();
    }
    let capture = match Capture::spawn(controls.hub.clone()) {
        Ok(capture) => capture,
        Err(err) => {
            *controls.status.lock().unwrap_or_else(|err| err.into_inner()) =
                format!("代理没有起来: {err:#}");
            return;
        }
    };
    let port = capture.addr.port();
    *controls.listen.lock().unwrap_or_else(|err| err.into_inner()) =
        capture.addr.to_string();
    match sysproxy::enable(port) {
        Ok(sysproxy::EnableOutcome::Applied) => {
            *controls.status.lock().unwrap_or_else(|err| err.into_inner()) =
                format!("抓包中 · 127.0.0.1:{port}");
            *controls.running.lock().unwrap_or_else(|err| err.into_inner()) = true;
            *controls.capture.lock().unwrap_or_else(|err| err.into_inner()) = Some(capture);
        }
        Ok(sysproxy::EnableOutcome::Manual { reason }) => {
            *controls.status.lock().unwrap_or_else(|err| err.into_inner()) = reason;
            *controls.running.lock().unwrap_or_else(|err| err.into_inner()) = true;
            *controls.capture.lock().unwrap_or_else(|err| err.into_inner()) = Some(capture);
        }
        Err(err) => {
            capture.stop();
            *controls.listen.lock().unwrap_or_else(|err| err.into_inner()) = String::new();
            *controls.status.lock().unwrap_or_else(|err| err.into_inner()) =
                format!("系统代理没有改成: {err:#}");
        }
    }
}

fn stop_capture(controls: &Controls) {
    if let Some(capture) = controls
        .capture
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
    {
        capture.stop();
    }
    match sysproxy::disable() {
        Ok(()) => {
            *controls.status.lock().unwrap_or_else(|err| err.into_inner()) = "已停止，系统代理已恢复".into();
        }
        Err(err) => {
            *controls.status.lock().unwrap_or_else(|err| err.into_inner()) =
                format!("代理已停，恢复系统代理失败: {err:#}");
        }
    }
    *controls.running.lock().unwrap_or_else(|err| err.into_inner()) = false;
}

fn lock_string(slot: &Mutex<String>) -> String {
    slot.lock().unwrap_or_else(|err| err.into_inner()).clone()
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

