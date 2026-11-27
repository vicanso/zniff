//! State helpers for composing requests/responses, resetting inspector state, and closing sequence views.

#[allow(unused_imports)]
use super::*;

pub(crate) fn apply_url_edit(compose: Compose, text: &str) {
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

pub(crate) fn looks_like_curl(text: &str) -> bool {
    let text = text.trim_start();
    text == "curl"
        || text.starts_with("curl ")
        || text.starts_with("curl\t")
        || text.starts_with("curl\n")
        || text.starts_with("curl\r")
}

pub(crate) fn parse_status_code(text: &str) -> Option<u16> {
    let code = text.split_whitespace().next()?.parse::<u16>().ok()?;
    (100..600).contains(&code).then_some(code)
}

pub(crate) fn edited_status(compose: Compose) -> Option<u16> {
    if !compose.response.get() {
        return None;
    }
    parse_status_code(&compose.status.get())
}

fn restore_or_reset_new_draft(compose: Compose) {
    let (m, u, h, b) = compose.draft_saved.get();
    if !(m.trim().is_empty() && u.trim().is_empty() && h.trim().is_empty() && b.trim().is_empty()) {
        compose.method.set(if m.trim().is_empty() {
            "GET".to_string()
        } else {
            m
        });
        compose.url.set(u);
        compose.headers.set(h);
        compose.body.set(b);
    } else {
        compose.method.set("GET".to_string());
        compose.url.set(String::new());
        compose.headers.set(String::new());
        compose.body.set(String::new());
    }
    compose.response.set(false);
    compose.paused_id.set(None);
    compose.status.set(String::new());
    compose.composing.set(true);
}

pub(crate) fn begin_new_request(compose: Compose, selected: Signal<Option<u64>>) {
    selected.set(None);
    restore_or_reset_new_draft(compose);
}

pub(crate) fn begin_compose(controls: &Controls, id: Option<u64>, compose: Compose) {
    compose.inline_mock.close();
    if refuse_relay(controls, id) {
        return;
    }
    if id.is_none() {
        restore_or_reset_new_draft(compose);
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
    let is_paused_flow = controls
        .hub
        .paused()
        .is_some_and(|(paused_id, _)| id == Some(paused_id));
    compose.response.set(response);
    compose
        .paused_id
        .set(if is_paused_flow { id } else { None });
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

pub(crate) fn header_text(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn reset_compose(compose: Compose) {
    compose.composing.set(false);
    compose.response.set(false);
    compose.paused_id.set(None);
    compose.method.set(String::new());
    compose.url.set(String::new());
    compose.headers.set(String::new());
    compose.body.set(String::new());
    compose.status.set(String::new());
    compose.inline_mock.close();
}

pub(crate) fn clear_shown(shown: Shown) {
    shown.phase.set(Phase::Empty);
    shown.method.set(String::new());
    shown.url.set(String::new());
    shown.protocol.set(String::new());
    shown.rules.set(String::new());
    shown.rule_hits.set(Vec::new());
    shown.gql_badge.set(String::new());
    shown.status.set(None);
    shown.error.set(String::new());
    shown.request_headers.set(Vec::new());
    shown.response_headers.set(Vec::new());
    shown.request_body.set(String::new());
    shown.response_body.set(String::new());
    shown.request_body_bytes.set(0);
    shown.response_body_bytes.set(0);
    shown.request_hex.set(Vec::new());
    shown.response_hex.set(Vec::new());
    shown.request_language.set(String::new());
    shown.response_language.set(String::new());
    shown.request_doc.set(None);
    shown.response_doc.set(None);
    shown.req_doc_preview.set(false);
    shown.res_doc_preview.set(false);
    shown.request_json.set(None);
    shown.response_json.set(None);
    let tree_default = shown.default_json_tree.get() && shown.pretty.get();
    shown.req_json_tree.set(tree_default);
    shown.res_json_tree.set(tree_default);
    shown.request_form.set(Vec::new());
    shown.request_note.set(String::new());
    shown.response_note.set(String::new());
    shown.hook_cards.set(Vec::new());
    shown.req_hook_cards.set(Vec::new());
    shown.res_hook_cards.set(Vec::new());
    shown.req_search.set(String::new());
    shown.res_search.set(String::new());
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
    shown.llm_stream.set(None);
    shown.msg_llm_view.set(true);
    shown.diff.set(String::new());
    shown.diff_old.set(String::new());
    shown.diff_new.set(String::new());
    shown.diff_baseline_label.set(String::new());
    shown.pinned.set(false);
    shown.is_marker.set(false);
    shown.relay.set(String::new());
    shown.comment.set(String::new());
    shown.editing_comment.set(false);
    shown.repeating.set(false);
    shown.req_headers_loading.set(false);
    shown.res_headers_loading.set(false);
    shown.req_body_loading.set(false);
    shown.res_body_loading.set(false);
    shown.messages_loading.set(false);
    shown.diff_loading.set(false);
}

/// Only hides the panels. The panel's views and memos are still mounted until
/// the next frame, so clearing their data here would make them read signals
/// that are being torn down. [`release_sequence_data`] frees the data later.
pub(crate) fn close_sequence_diagram(seq_builder: SeqBuilder) {
    seq_builder.open.set(false);
    seq_builder.redact_open.set(false);
}

/// Frees the sequence-diagram data once both panels are closed. Runs from the
/// periodic tick, after the closed panels have been unmounted.
pub(crate) fn release_sequence_data(seq_builder: SeqBuilder) {
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
