#[allow(unused_imports)]
use super::*;

pub(crate) fn publish_detail(
    shown: &Shown,
    hub: &Hub,
    id: Option<u64>,
    lang: Lang,
    baseline: Option<u64>,
) {
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

pub(crate) fn message_lines(flow: &crate::session::Flow, lang: Lang) -> Vec<HeaderLine> {
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

pub(crate) fn header_lines(headers: &[(String, String)], pretty: bool) -> Vec<HeaderLine> {
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
pub(crate) const BODY_DISPLAY_CAP: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TextWindow {
    pub(crate) offset: usize,
    pub(crate) end: usize,
    pub(crate) total: usize,
}

impl TextWindow {
    pub(crate) fn has_prev(self) -> bool {
        self.offset > 0
    }

    pub(crate) fn has_more(self) -> bool {
        self.end < self.total
    }
}

pub(crate) struct BodyText {
    pub(crate) text: String,
    pub(crate) note: String,
    /// Set for a binary body. The view breaks it into 16 or 32 byte rows.
    pub(crate) hex: Vec<u8>,
    pub(crate) window: TextWindow,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn display_body(
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

pub(crate) fn apply_pretty(
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

pub(crate) fn window_text(text: &str, offset: usize) -> (TextWindow, &str) {
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

pub(crate) fn window_note(lang: Lang, window: TextWindow) -> String {
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

pub(crate) fn is_window_note_line(line: &str) -> bool {
    line.starts_with("界面只显示前 ")
        || line.starts_with("界面显示第 ")
        || line.starts_with("Showing only the first ")
        || line.starts_with("Showing bytes ")
}

pub(crate) fn replace_window_note(note: &mut String, extra: Option<String>) {
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

pub(crate) fn body_text(
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
