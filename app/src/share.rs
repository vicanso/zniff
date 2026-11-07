//! A snapshot HTML file, and one live share link.
//!
//! The link is a random id on the proxy port. It expires, and a new share
//! replaces the previous id. Requests for it are not recorded as flows.

use std::time::{SystemTime, UNIX_EPOCH};

use ring::rand::{SecureRandom, SystemRandom};
use serde::Serialize;

use crate::session::{Direction, Flow};

/// Newest flows kept in one share document.
const FLOW_CAP: usize = 400;
/// Characters of one body included in the share.
const BODY_CHARS: usize = 16_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveShare {
    pub id: String,
    pub expires: SystemTime,
    /// Filter text from the box at the moment the link was created.
    /// Later polls keep using it, including on flows that arrive after that.
    pub filter: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharePath {
    Page,
    Data,
}

pub fn new_id() -> String {
    let mut bytes = [0_u8; 16];
    if SystemRandom::new().fill(&mut bytes).is_err() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        bytes.copy_from_slice(&nanos.to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

pub fn valid_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 8 | 13 | 18 | 23) || byte.is_ascii_hexdigit())
}

/// Origin-form `/s/{id}` or `/s/{id}/flows.json`.
pub fn parse_share_path(path: &str) -> Option<(String, SharePath)> {
    let rest = path.strip_prefix("/s/")?;
    if let Some(id) = rest.strip_suffix("/flows.json") {
        return valid_id(id).then(|| (id.to_string(), SharePath::Data));
    }
    valid_id(rest).then(|| (rest.to_string(), SharePath::Page))
}

pub fn grant_ok(share: &LiveShare, id: &str, now: SystemTime) -> bool {
    share.id == id && now < share.expires
}

#[derive(Serialize)]
struct ShareDoc {
    expires: u64,
    filter: String,
    omitted: usize,
    flows: Vec<ShareFlow>,
}

#[derive(Serialize)]
struct ShareFlow {
    id: u64,
    method: String,
    status: Option<u16>,
    error: Option<String>,
    protocol: String,
    host: String,
    path: String,
    url: String,
    started: String,
    gap: String,
    elapsed: String,
    slow: bool,
    size: String,
    request_bytes: u64,
    response_bytes: u64,
    ttfb_ms: Option<u128>,
    total_ms: Option<u128>,
    pinned: bool,
    rule_hit: String,
    query: Vec<(String, String)>,
    request_headers: Vec<(String, String)>,
    response_headers: Vec<(String, String)>,
    request_body: String,
    request_body_pretty: String,
    response_body: String,
    response_body_pretty: String,
    messages: Vec<String>,
}

pub fn document(flows: &[Flow], expires: SystemTime, filter: &str) -> String {
    let omitted = flows.len().saturating_sub(FLOW_CAP);
    let mut previous = None;
    let flows = flows
        .iter()
        .skip(omitted)
        .map(|flow| {
            let item = share_flow(flow, previous);
            previous = Some(flow.started);
            item
        })
        .collect();
    let doc = ShareDoc {
        expires: expires
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0),
        filter: filter.to_string(),
        omitted,
        flows,
    };
    serde_json::to_string(&doc).unwrap_or_else(|_| "{}".into())
}

fn format_bytes_short(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if (bytes as f64) < MB {
        format!("{:.1} KB", bytes as f64 / KB)
    } else {
        format!("{:.2} MB", bytes as f64 / MB)
    }
}

fn share_flow(flow: &Flow, previous: Option<SystemTime>) -> ShareFlow {
    let req_ct = crate::present::content_type(&flow.request_headers);
    let res_ct = crate::present::content_type(&flow.response_headers);
    let (request_body, request_body_pretty) = share_body_pair(
        &flow.request_body,
        flow.request_truncated,
        flow.request_expired,
        req_ct,
    );
    let (response_body, response_body_pretty) = share_body_pair(
        &flow.response_body,
        flow.response_truncated,
        flow.response_expired,
        res_ct,
    );
    let ttfb_ms = flow
        .response_at
        .and_then(|at| at.duration_since(flow.started).ok())
        .map(|d| d.as_millis());
    let total_ms = flow
        .finished_at
        .and_then(|at| at.duration_since(flow.started).ok())
        .map(|d| d.as_millis());
    let messages = if !flow.messages.is_empty() {
        flow.messages
            .iter()
            .take(40)
            .map(|message| {
                let side = match message.direction {
                    Direction::Request => "→",
                    Direction::Response => "←",
                };
                let text = clip_chars(&message.text, 2_000);
                format!("{side} {} {text}", message.kind)
            })
            .collect()
    } else if res_ct.is_some_and(|ct| ct.to_ascii_lowercase().contains("event-stream"))
        && let Ok(text) = std::str::from_utf8(&flow.response_body)
    {
        crate::present::sse_events(text)
            .into_iter()
            .take(40)
            .map(|ev| format!("← [{}] {}", ev.event, clip_chars(&ev.data, 2_000)))
            .collect()
    } else {
        Vec::new()
    };

    ShareFlow {
        id: flow.id,
        method: flow.method.clone(),
        status: flow.status,
        error: flow.error.clone(),
        protocol: flow.protocol.label().to_string(),
        host: flow.host.clone(),
        path: flow.path.clone(),
        url: flow.url.clone(),
        started: crate::present::format_clock(flow.started),
        gap: crate::present::format_gap(previous, flow.started),
        elapsed: crate::present::format_elapsed(flow.started, flow.finished_at),
        slow: flow.finished_at.is_some()
            && crate::present::is_slow(
                flow.started,
                flow.finished_at,
                flow.finished_at.unwrap_or(flow.started),
            ),
        size: format_bytes_short(flow.response_bytes),
        request_bytes: flow.request_bytes,
        response_bytes: flow.response_bytes,
        ttfb_ms,
        total_ms,
        pinned: flow.pinned,
        rule_hit: flow.rule_hit.clone(),
        query: crate::present::query_pairs(&flow.url),
        request_headers: flow.request_headers.clone(),
        response_headers: flow.response_headers.clone(),
        request_body,
        request_body_pretty,
        response_body,
        response_body_pretty,
        messages,
    }
}

fn share_body_pair(
    bytes: &[u8],
    truncated: bool,
    expired: Option<usize>,
    content_type: Option<&str>,
) -> (String, String) {
    if bytes.is_empty() {
        if let Some(exp) = expired {
            let msg = format!(
                "[large body ({}) reclaimed]",
                format_bytes_short(exp as u64)
            );
            return (msg, String::new());
        }
        return if truncated {
            ("…".into(), String::new())
        } else {
            (String::new(), String::new())
        };
    }
    let raw = share_body(bytes, truncated);
    if let Ok(text) = std::str::from_utf8(bytes) {
        let pretty_full = crate::present::pretty_body(content_type, text);
        if pretty_full != text {
            let mut pretty = clip_chars(&pretty_full, BODY_CHARS);
            if pretty_full.chars().count() > BODY_CHARS || truncated {
                pretty.push_str("\n…");
            }
            return (raw, pretty);
        }
    }
    (raw, String::new())
}

fn share_body(bytes: &[u8], truncated: bool) -> String {
    if bytes.is_empty() {
        return if truncated {
            "…".into()
        } else {
            String::new()
        };
    }
    let text = String::from_utf8_lossy(bytes);
    let broken = text.chars().filter(|ch| *ch == '\u{FFFD}').count();
    if broken > 0 && bytes.len() > 64 && broken.saturating_mul(8) > bytes.len() {
        return format!("[{} bytes]", bytes.len());
    }
    let mut out = clip_chars(&text, BODY_CHARS);
    if text.chars().count() > BODY_CHARS || truncated {
        out.push_str("\n…");
    }
    out
}

fn clip_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// `live` is the polling path. An empty live path is a file that does not refresh.
pub fn page(json: &str, live: &str, zh: bool) -> String {
    let json = json.replace('<', "\\u003c");
    let live = live.replace('<', "");
    let title = if zh { "zniff 分享" } else { "zniff share" };
    format!(
        r##"<!DOCTYPE html>
<html lang="{lang}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
  :root {{
    --bg-canvas: #0d1017;
    --bg-chrome: #131822;
    --bg-list: #10141d;
    --bg-list-header: #151b26;
    --bg-row-even: #131823;
    --bg-row-hover: #1a2130;
    --bg-row-selected: #172554;
    --bg-card: #131822;
    --bg-code: #0f131b;
    --bg-control: #1c2331;
    --bg-control-hover: #252e40;
    --border-strong: #222938;
    --border-subtle: #19202e;
    --border-control: #2b3448;
    --fg-primary: #f1f5f9;
    --fg-secondary: #cbd5e1;
    --fg-muted: #7e8ba3;
    --accent-blue: #3b82f6;
    --accent-blue-soft: #172554;
    --accent-blue-border: #1e40af;
    --accent-blue-text: #93c5fd;
    --ok-fg: #34d399; --ok-bg: #0a2e26;
    --warn-fg: #fbbf24; --warn-bg: #33240b;
    --err-fg: #fb7185; --err-bg: #361320;
    --m-get-fg: #38bdf8; --m-get-bg: #0c2a42;
    --m-post-fg: #34d399; --m-post-bg: #0a2e26;
    --m-put-fg: #fbbf24; --m-put-bg: #33240b;
    --m-del-fg: #fb7185; --m-del-bg: #361320;
    --m-con-fg: #a78bfa; --m-con-bg: #241842;
  }}
  html[data-theme="light"] {{
    --bg-canvas: #f8fafc;
    --bg-chrome: #ffffff;
    --bg-list: #ffffff;
    --bg-list-header: #f1f5f9;
    --bg-row-even: #f8fafc;
    --bg-row-hover: #eff6ff;
    --bg-row-selected: #dbeafe;
    --bg-card: #ffffff;
    --bg-code: #f8fafc;
    --bg-control: #f1f5f9;
    --bg-control-hover: #e2e8f0;
    --border-strong: #e2e8f0;
    --border-subtle: #f1f5f9;
    --border-control: #cbd5e1;
    --fg-primary: #0f172a;
    --fg-secondary: #334155;
    --fg-muted: #64748b;
    --accent-blue: #2563eb;
    --accent-blue-soft: #dbeafe;
    --accent-blue-border: #93c5fd;
    --accent-blue-text: #1d4ed8;
    --ok-fg: #059669; --ok-bg: #d1fae5;
    --warn-fg: #d97706; --warn-bg: #fef3c7;
    --err-fg: #e11d48; --err-bg: #ffe4e6;
    --m-get-fg: #0284c7; --m-get-bg: #e0f2fe;
    --m-post-fg: #059669; --m-post-bg: #d1fae5;
    --m-put-fg: #d97706; --m-put-bg: #fef3c7;
    --m-del-fg: #e11d48; --m-del-bg: #ffe4e6;
    --m-con-fg: #7c3aed; --m-con-bg: #ede9fe;
  }}
  * {{ box-sizing: border-box; }}
  html, body {{ margin: 0; height: 100%; overflow: hidden; background: var(--bg-canvas); color: var(--fg-primary); font: 13px/1.45 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; -webkit-font-smoothing: antialiased; }}
  .mono {{ font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; }}
  .shell {{ display: flex; flex-direction: column; height: 100vh; width: 100vw; }}
  .topbar {{ height: 44px; flex-shrink: 0; background: var(--bg-chrome); border-bottom: 1px solid var(--border-strong); padding: 0 12px; display: flex; align-items: center; justify-content: space-between; gap: 12px; }}
  .brand {{ display: flex; align-items: center; gap: 8px; min-width: 0; }}
  .logo {{ width: 22px; height: 22px; border-radius: 6px; background: var(--accent-blue); color: #fff; font-weight: 800; font-size: 12px; display: inline-flex; align-items: center; justify-content: center; flex-shrink: 0; }}
  .brand-name {{ font-weight: 700; font-size: 14px; letter-spacing: -0.01em; }}
  .status-dot {{ width: 7px; height: 7px; border-radius: 99px; background: var(--ok-fg); flex-shrink: 0; }}
  .status-dot.ended {{ background: var(--err-fg); }}
  .status-dot.snapshot {{ background: var(--accent-blue); }}
  .meta-text {{ color: var(--fg-secondary); font-size: 12.5px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }}
  .toolbar-right {{ display: flex; align-items: center; gap: 6px; flex-shrink: 0; }}
  .pill {{ height: 28px; padding: 0 10px; border-radius: 7px; background: var(--bg-control); border: 1px solid var(--border-control); color: var(--fg-primary); font-size: 12px; display: inline-flex; align-items: center; gap: 6px; cursor: pointer; user-select: none; }}
  .pill:hover {{ background: var(--bg-control-hover); }}
  .pill.active {{ background: var(--accent-blue-soft); border-color: var(--accent-blue-border); color: var(--accent-blue-text); }}
  .pill.static {{ cursor: default; font-weight: 600; color: var(--fg-secondary); }}
  .pill.static:hover {{ background: var(--bg-control); }}
  .workspace {{ flex: 1; min-height: 0; display: flex; overflow: hidden; }}
  .left-pane {{ width: 560px; min-width: 280px; max-width: 1100px; flex-shrink: 0; background: var(--bg-list); display: flex; flex-direction: column; min-height: 0; }}
  .splitter {{ width: 5px; flex-shrink: 0; background: var(--border-strong); cursor: col-resize; transition: background 0.12s; }}
  .splitter:hover, .splitter.dragging {{ background: var(--accent-blue); }}
  .filter-bar {{ padding: 8px; background: var(--bg-chrome); border-bottom: 1px solid var(--border-strong); display: flex; flex-direction: column; gap: 6px; flex-shrink: 0; }}
  .filter-input-wrap {{ position: relative; display: flex; align-items: center; }}
  .filter-input {{ width: 100%; height: 30px; padding: 0 28px 0 10px; border-radius: 7px; border: 1px solid var(--border-control); background: var(--bg-code); color: var(--fg-primary); font-size: 12.5px; outline: none; }}
  .filter-input:focus {{ border-color: var(--accent-blue); }}
  .filter-clear {{ position: absolute; right: 6px; width: 18px; height: 18px; border-radius: 4px; border: none; background: transparent; color: var(--fg-muted); cursor: pointer; display: none; align-items: center; justify-content: center; font-size: 14px; padding: 0; }}
  .filter-clear:hover {{ color: var(--fg-primary); background: var(--bg-control); }}
  .chips-row {{ display: flex; align-items: center; gap: 5px; flex-wrap: wrap; }}
  .chip {{ height: 22px; padding: 0 8px; border-radius: 5px; border: 1px solid var(--border-control); background: var(--bg-control); color: var(--fg-secondary); font-size: 11px; cursor: pointer; display: inline-flex; align-items: center; gap: 4px; user-select: none; }}
  .chip:hover {{ background: var(--bg-control-hover); }}
  .chip.active {{ background: var(--accent-blue-soft); border-color: var(--accent-blue-border); color: var(--accent-blue-text); }}
  .chip-dot {{ width: 5px; height: 5px; border-radius: 99px; background: var(--fg-muted); }}
  .chip.active .chip-dot {{ background: var(--accent-blue); }}
  .table-head {{ height: 28px; flex-shrink: 0; background: var(--bg-list-header); border-bottom: 1px solid var(--border-strong); display: grid; grid-template-columns: 38px 76px minmax(120px, 1fr) 48px 60px 56px 66px; align-items: center; font-size: 11px; font-weight: 600; color: var(--fg-muted); user-select: none; }}
  .th {{ padding: 0 4px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
  .th.id {{ padding-left: 6px; }}
  .th.method {{ padding-left: 14px; }}
  .th.num {{ text-align: right; padding-right: 6px; }}
  .table-body {{ flex: 1; overflow-y: auto; overflow-x: hidden; }}
  .row {{ height: 29px; display: grid; grid-template-columns: 38px 76px minmax(120px, 1fr) 48px 60px 56px 66px; align-items: center; border-bottom: 1px solid var(--border-subtle); border-left: 2px solid transparent; cursor: pointer; font-size: 12px; user-select: none; }}
  .row:nth-child(even) {{ background: var(--bg-row-even); }}
  .row:hover {{ background: var(--bg-row-hover); }}
  .row.selected {{ background: var(--bg-row-selected); border-left-color: var(--accent-blue); }}
  .cell {{ padding: 0 4px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
  .cell.id {{ color: var(--fg-muted); }}
  .cell.method {{ display: flex; align-items: center; gap: 3px; padding: 0 2px; }}
  .pin-slot {{ width: 8px; height: 8px; display: inline-flex; align-items: center; justify-content: center; flex-shrink: 0; }}
  .pin-dot {{ width: 6px; height: 6px; border-radius: 99px; background: var(--accent-blue); }}
  .m-badge {{ padding: 1px 5px; border-radius: 4px; font-size: 10.5px; font-weight: 700; line-height: 1.35; background: var(--bg-control); color: var(--fg-secondary); }}
  .m-GET {{ color: var(--m-get-fg); background: var(--m-get-bg); }}
  .m-POST {{ color: var(--m-post-fg); background: var(--m-post-bg); }}
  .m-PUT, .m-PATCH {{ color: var(--m-put-fg); background: var(--m-put-bg); }}
  .m-DELETE {{ color: var(--m-del-fg); background: var(--m-del-bg); }}
  .m-CONNECT {{ color: var(--m-con-fg); background: var(--m-con-bg); }}
  .cell.path {{ color: var(--fg-secondary); }}
  .cell.path.failed {{ color: var(--err-fg); }}
  .cell.status {{ font-weight: 600; color: var(--fg-muted); }}
  .s-ok {{ color: var(--ok-fg) !important; }}
  .s-warn {{ color: var(--warn-fg) !important; }}
  .s-err {{ color: var(--err-fg) !important; }}
  .cell.num {{ text-align: right; padding-right: 6px; color: var(--fg-muted); }}
  .cell.num.slow {{ color: var(--warn-fg); }}
  .right-pane {{ flex: 1; min-width: 320px; background: var(--bg-canvas); overflow-y: auto; padding: 10px; display: flex; flex-direction: column; gap: 10px; }}
  .empty-state {{ margin: auto; color: var(--fg-muted); font-size: 13px; text-align: center; padding: 32px; }}
  .card {{ background: var(--bg-card); border: 1px solid var(--border-strong); border-radius: 8px; overflow: hidden; flex-shrink: 0; }}
  .summary-card {{ padding: 10px 12px; display: flex; flex-direction: column; gap: 8px; }}
  .summary-top {{ display: flex; align-items: center; justify-content: space-between; gap: 8px; flex-wrap: wrap; }}
  .summary-badges {{ display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }}
  .s-badge {{ padding: 2px 7px; border-radius: 4px; font-size: 11px; font-weight: 700; }}
  .s-badge.ok {{ color: var(--ok-fg); background: var(--ok-bg); }}
  .s-badge.warn {{ color: var(--warn-fg); background: var(--warn-bg); }}
  .s-badge.err {{ color: var(--err-fg); background: var(--err-bg); }}
  .proto-badge {{ padding: 2px 7px; border-radius: 4px; font-size: 11px; background: var(--bg-control); color: var(--fg-secondary); }}
  .rule-badge {{ padding: 2px 7px; border-radius: 4px; font-size: 11px; background: var(--accent-blue-soft); color: var(--accent-blue-text); border: 1px solid var(--accent-blue-border); }}
  .url-box {{ padding: 7px 9px; border-radius: 6px; background: var(--bg-code); border: 1px solid var(--border-subtle); color: var(--fg-primary); font-size: 12px; word-break: break-all; line-height: 1.45; }}
  .err-box {{ padding: 6px 9px; border-radius: 6px; background: var(--err-bg); color: var(--err-fg); font-size: 12px; }}
  .metrics-line {{ color: var(--fg-muted); font-size: 11.5px; }}
  .card-head {{ height: 34px; padding: 0 12px; display: flex; align-items: center; justify-content: space-between; cursor: pointer; user-select: none; background: var(--bg-card); }}
  .card-head:hover {{ background: var(--bg-row-hover); }}
  .card-title-wrap {{ display: flex; align-items: center; gap: 6px; }}
  .card-arrow {{ font-size: 10px; color: var(--fg-muted); width: 12px; display: inline-block; transition: transform 0.12s; }}
  .card.folded .card-arrow {{ transform: rotate(-90deg); }}
  .card.folded .card-body {{ display: none; }}
  .card-title {{ font-size: 12px; font-weight: 600; color: var(--fg-primary); }}
  .count-badge {{ padding: 0 6px; height: 18px; border-radius: 99px; background: var(--bg-control); color: var(--fg-muted); font-size: 11px; font-weight: 600; display: inline-flex; align-items: center; }}
  .card-actions {{ display: flex; align-items: center; gap: 6px; }}
  .mini-btn {{ height: 22px; padding: 0 8px; border-radius: 5px; border: 1px solid var(--border-control); background: var(--bg-control); color: var(--fg-secondary); font-size: 11px; cursor: pointer; }}
  .mini-btn:hover {{ background: var(--bg-control-hover); color: var(--fg-primary); }}
  .mini-btn.active {{ background: var(--accent-blue-soft); border-color: var(--accent-blue-border); color: var(--accent-blue-text); }}
  .card-body {{ border-top: 1px solid var(--border-strong); }}
  .kv-row {{ display: grid; grid-template-columns: 180px minmax(0, 1fr); gap: 10px; padding: 6px 12px; font-size: 12px; line-height: 1.45; border-top: 1px solid var(--border-subtle); }}
  .kv-row:first-child {{ border-top: none; }}
  .kv-key {{ color: var(--fg-muted); word-break: break-all; font-weight: 500; }}
  .kv-val {{ color: var(--fg-primary); word-break: break-all; white-space: pre-wrap; }}
  .code-pre {{ margin: 0; padding: 10px 12px; background: var(--bg-code); color: var(--fg-secondary); font-size: 12px; line-height: 1.5; max-height: 320px; overflow: auto; white-space: pre-wrap; word-break: break-word; }}
  .bottombar {{ height: 26px; flex-shrink: 0; background: var(--bg-chrome); border-top: 1px solid var(--border-strong); padding: 0 12px; display: flex; align-items: center; justify-content: space-between; font-size: 11.5px; color: var(--fg-muted); }}
  @media (max-width: 860px) {{
    .workspace {{ flex-direction: column; }}
    .left-pane {{ width: 100% !important; max-width: none; height: 46%; border-bottom: 1px solid var(--border-strong); }}
    .splitter {{ display: none; }}
    .right-pane {{ height: 54%; min-width: 0; }}
    .kv-row {{ grid-template-columns: 120px minmax(0, 1fr); }}
  }}
</style>
</head>
<body>
<div class="shell">
  <header class="topbar">
    <div class="brand">
      <span class="logo">Z</span>
      <span class="brand-name">zniff</span>
      <span class="status-dot" id="status-dot"></span>
      <span class="meta-text" id="meta"></span>
    </div>
    <div class="toolbar-right">
      <span class="pill static mono" id="count-pill">0</span>
      <button class="pill active" id="pretty-btn" type="button"></button>
      <button class="pill" id="theme-btn" type="button"></button>
    </div>
  </header>

  <div class="workspace" id="workspace">
    <section class="left-pane" id="left-pane">
      <div class="filter-bar">
        <div class="filter-input-wrap">
          <input class="filter-input" id="filter-input" type="text" autocomplete="off" spellcheck="false">
          <button class="filter-clear" id="filter-clear" type="button" title="Clear">×</button>
        </div>
        <div class="chips-row" id="chips-row"></div>
      </div>
      <div class="table-head">
        <div class="th id" id="th-id"></div>
        <div class="th method" id="th-method"></div>
        <div class="th" id="th-path"></div>
        <div class="th" id="th-status"></div>
        <div class="th num" id="th-gap"></div>
        <div class="th num" id="th-elapsed"></div>
        <div class="th num" id="th-size"></div>
      </div>
      <div class="table-body" id="table-body"></div>
    </section>
    <div class="splitter" id="splitter"></div>
    <section class="right-pane" id="right-pane"></section>
  </div>

  <footer class="bottombar">
    <span id="footer-left"></span>
    <span id="footer-right"></span>
  </footer>
</div>
<script>
const LIVE = "{live}";
const INITIAL = {json};
const ZH = {zh_bool};
const I18N = ZH ? {{
  loading: "正在读取…",
  live: "实时分享",
  snapshot: "快照归档",
  ended: "分享已结束或已过期",
  expiresAt: "有效期至 ",
  frozenFilter: "分享过滤: ",
  omitted: "较早的记录已省略",
  filterPlaceholder: "过滤域名、路径、状态码、方法、type:json、body:、header:…",
  chipJson: "JSON",
  chipErr: "报错",
  chipPin: "星标",
  chipSlow: "慢请求",
  thId: "序号",
  thMethod: "方法",
  thPath: "路径",
  thStatus: "状态",
  thGap: "间隔",
  thElapsed: "耗时",
  thSize: "大小",
  pretty: "美化",
  raw: "原文",
  themeDark: "深色",
  themeLight: "浅色",
  emptyList: "暂无匹配的抓包记录",
  emptyDetail: "从左侧列表选择一条请求查看详情",
  copyUrl: "复制 URL",
  copyCurl: "复制 cURL",
  copy: "复制",
  copied: "已复制",
  resBody: "响应 body",
  resHeaders: "响应头",
  reqBody: "请求 body",
  queryParams: "查询参数",
  reqHeaders: "请求头",
  messages: "消息",
  ttfb: "首字节",
  total: "耗时",
  req: "请求",
  res: "响应",
  inProgress: "进行中",
  shortcutHint: "↑ / ↓ 切换选中请求 · 拖动中间分割条调整列宽"
}} : {{
  loading: "Loading…",
  live: "Live share",
  snapshot: "Snapshot",
  ended: "This share has ended or expired",
  expiresAt: "Expires ",
  frozenFilter: "Share filter: ",
  omitted: "Older flows omitted",
  filterPlaceholder: "Filter host, path, status, method, type:json, body:, header:…",
  chipJson: "JSON",
  chipErr: "Errors",
  chipPin: "Starred",
  chipSlow: "Slow",
  thId: "ID",
  thMethod: "Method",
  thPath: "Path",
  thStatus: "Status",
  thGap: "Gap",
  thElapsed: "Elapsed",
  thSize: "Size",
  pretty: "Pretty",
  raw: "Raw",
  themeDark: "Dark",
  themeLight: "Light",
  emptyList: "No matching flows",
  emptyDetail: "Select a flow on the left to inspect details",
  copyUrl: "Copy URL",
  copyCurl: "Copy cURL",
  copy: "Copy",
  copied: "Copied",
  resBody: "Response body",
  resHeaders: "Response headers",
  reqBody: "Request body",
  queryParams: "Query parameters",
  reqHeaders: "Request headers",
  messages: "Messages",
  ttfb: "TTFB",
  total: "Total",
  req: "Req",
  res: "Res",
  inProgress: "In progress",
  shortcutHint: "Use ↑ / ↓ to navigate flows · Drag splitter to resize"
}};

let state = {{
  data: INITIAL,
  ended: false,
  filter: "",
  selectedId: null,
  pretty: true,
  theme: window.matchMedia && window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark",
  folded: {{}},
}};

document.documentElement.setAttribute("data-theme", state.theme);
document.getElementById("th-id").textContent = I18N.thId;
document.getElementById("th-method").textContent = I18N.thMethod;
document.getElementById("th-path").textContent = I18N.thPath;
document.getElementById("th-status").textContent = I18N.thStatus;
document.getElementById("th-gap").textContent = I18N.thGap;
document.getElementById("th-elapsed").textContent = I18N.thElapsed;
document.getElementById("th-size").textContent = I18N.thSize;
document.getElementById("footer-left").textContent = I18N.shortcutHint;

const filterInput = document.getElementById("filter-input");
const filterClear = document.getElementById("filter-clear");
filterInput.placeholder = I18N.filterPlaceholder;

const CHIPS = [
  {{ token: "type:json", label: I18N.chipJson }},
  {{ token: "err", label: I18N.chipErr }},
  {{ token: "pin:", label: I18N.chipPin }},
  {{ token: "slow:", label: I18N.chipSlow }},
];

function esc(text) {{
  return String(text ?? "").replace(/[&<>"]/g, ch => ({{"&":"&amp;","<":"&lt;",">":"&gt;","\"":"&quot;"}}[ch]));
}}

function formatBytes(b) {{
  const n = Number(b || 0);
  if (n < 1024) return n + " B";
  if (n < 1024 * 1024) return (n / 1024).toFixed(1) + " KB";
  return (n / (1024 * 1024)).toFixed(2) + " MB";
}}

function statusClass(flow) {{
  const failed = !!flow.error && flow.status == null;
  if (failed || (flow.status != null && flow.status >= 400)) return "s-err";
  if (flow.status != null && flow.status >= 300) return "s-warn";
  if (flow.status != null && flow.status >= 200) return "s-ok";
  return "";
}}

function statusBadgeClass(flow) {{
  const failed = !!flow.error && flow.status == null;
  if (failed || (flow.status != null && flow.status >= 400)) return "s-badge err";
  if (flow.status != null && flow.status >= 300) return "s-badge warn";
  if (flow.status != null && flow.status >= 200) return "s-badge ok";
  return "proto-badge";
}}

function hasToken(filter, token) {{
  return filter.toLowerCase().split(/\s+/).includes(token.toLowerCase());
}}

function toggleToken(token) {{
  const parts = state.filter.trim().split(/\s+/).filter(Boolean);
  const idx = parts.findIndex(p => p.toLowerCase() === token.toLowerCase());
  if (idx >= 0) parts.splice(idx, 1);
  else parts.push(token);
  state.filter = parts.join(" ");
  filterInput.value = state.filter;
  renderAll();
}}

function matchesFlow(flow, rawFilter) {{
  const trimmed = (rawFilter || "").trim().toLowerCase();
  if (!trimmed) return true;
  const tokens = trimmed.replace(/\s*\|\s*/g, "|").split(/\s+/).filter(Boolean);
  const host = (flow.host || "").toLowerCase();
  const path = (flow.path || "").toLowerCase();
  const method = (flow.method || "").toUpperCase();
  const failed = !!flow.error && flow.status == null;
  const ct = ((flow.response_headers || []).concat(flow.request_headers || []).find(h => h[0].toLowerCase() === "content-type")?.[1] || "").toLowerCase();

  for (const tok of tokens) {{
    if (tok === "pin:") {{ if (!flow.pinned) return false; continue; }}
    if (tok === "slow:") {{ if (!flow.slow) return false; continue; }}
    if (tok.startsWith("mute:") || (tok.startsWith("-") && tok.length > 1 && !tok.includes(":"))) {{
      const ex = (tok.startsWith("mute:") ? tok.slice(5) : tok.slice(1)).split(/[|,]/).filter(Boolean);
      if (ex.some(x => host.includes(x) || path.includes(x))) return false;
      continue;
    }}
    if (tok.startsWith("host:") || tok.startsWith("domain:")) {{
      const terms = tok.replace(/^(host|domain):/, "").split(/[|,]/).filter(Boolean);
      if (terms.length && !terms.some(p => p.startsWith("*.") ? (host === p.slice(2) || host.endsWith("." + p.slice(2))) : host.includes(p))) return false;
      continue;
    }}
    if (tok.startsWith("path:")) {{
      const terms = tok.slice(5).split(/[|,]/).filter(Boolean);
      if (terms.length && !terms.some(p => path.startsWith(p) || path.replace(/^\//, "").startsWith(p))) return false;
      continue;
    }}
    if (tok.startsWith("type:") || tok.startsWith("ct:")) {{
      const terms = tok.replace(/^(type|ct):/, "").split(/[|,]/).filter(Boolean);
      if (terms.length && !terms.some(k => ct.includes(k))) return false;
      continue;
    }}
    if (tok.startsWith("header:")) {{
      const terms = tok.slice(7).split(/[|,]/).filter(Boolean);
      const allH = (flow.request_headers || []).concat(flow.response_headers || []);
      if (terms.length && !terms.some(k => allH.some(h => h[0].toLowerCase().includes(k) || h[1].toLowerCase().includes(k)))) return false;
      continue;
    }}
    if (tok.startsWith("body:")) {{
      const terms = tok.slice(5).split(/[|,]/).filter(Boolean);
      const reqB = (flow.request_body || "").toLowerCase();
      const resB = (flow.response_body || "").toLowerCase();
      if (terms.length && !terms.some(k => reqB.includes(k) || resB.includes(k))) return false;
      continue;
    }}
    const ors = tok.split(/[|,]/).filter(Boolean);
    if (ors.every(p => ["get","post","put","patch","delete","head","options","connect"].includes(p))) {{
      if (!ors.some(p => p.toUpperCase() === method)) return false;
      continue;
    }}
    if (ors.every(p => ["fail","err","error","失败"].includes(p) || /^[1-5]xx$/.test(p) || /^\d{{3}}$/.test(p))) {{
      const ok = ors.some(p => {{
        if (["fail","err","error","失败"].includes(p)) return failed;
        if (/^[1-5]xx$/.test(p)) return flow.status != null && Math.floor(flow.status / 100) === Number(p[0]);
        return flow.status === Number(p);
      }});
      if (!ok) return false;
      continue;
    }}
    if (!ors.some(p => host.includes(p) || path.includes(p))) return false;
  }}
  return true;
}}

function buildCurl(flow) {{
  let parts = ["curl", "-X", flow.method, "'" + String(flow.url || "").replace(/'/g, "'\\''") + "'"];
  for (const [k, v] of (flow.request_headers || [])) {{
    if (k.toLowerCase() === "host" || k.toLowerCase() === "content-length") continue;
    parts.push("-H '" + (k + ": " + v).replace(/'/g, "'\\''") + "'");
  }}
  if (flow.request_body) {{
    parts.push("--data-raw '" + String(flow.request_body).replace(/'/g, "'\\''") + "'");
  }}
  return parts.join(" \\\n  ");
}}

async function copyText(btn, text) {{
  try {{
    await navigator.clipboard.writeText(text);
    const prev = btn.textContent;
    btn.textContent = I18N.copied;
    setTimeout(() => {{ btn.textContent = prev; }}, 1000);
  }} catch (_) {{}}
}}

function renderToolbar(filteredCount, totalCount) {{
  const dot = document.getElementById("status-dot");
  const meta = document.getElementById("meta");
  const countPill = document.getElementById("count-pill");
  const prettyBtn = document.getElementById("pretty-btn");
  const themeBtn = document.getElementById("theme-btn");

  prettyBtn.textContent = state.pretty ? I18N.pretty : I18N.raw;
  prettyBtn.classList.toggle("active", state.pretty);
  themeBtn.textContent = state.theme === "dark" ? I18N.themeDark : I18N.themeLight;

  if (state.ended || !state.data || !state.data.flows) {{
    dot.className = "status-dot ended";
    meta.textContent = I18N.ended;
    countPill.textContent = "0";
    return;
  }}
  dot.className = LIVE ? "status-dot" : "status-dot snapshot";
  const parts = [LIVE ? I18N.live : I18N.snapshot];
  if (LIVE && state.data.expires) {{
    const when = new Date(state.data.expires * 1000);
    parts.push(I18N.expiresAt + when.toLocaleTimeString());
  }}
  if (state.data.filter) parts.push(I18N.frozenFilter + state.data.filter);
  if (state.data.omitted) parts.push(I18N.omitted + " (" + state.data.omitted + ")");
  meta.textContent = parts.join(" · ");

  countPill.textContent = state.filter.trim() ? (filteredCount + " / " + totalCount) : String(totalCount);
  filterClear.style.display = state.filter ? "inline-flex" : "none";

  const chipsRow = document.getElementById("chips-row");
  chipsRow.innerHTML = CHIPS.map(c => {{
    const on = hasToken(state.filter, c.token) ? " active" : "";
    return `<button class="chip${{on}}" data-token="${{esc(c.token)}}" type="button"><span class="chip-dot"></span>${{esc(c.label)}}</button>`;
  }}).join("");
  chipsRow.querySelectorAll(".chip").forEach(btn => {{
    btn.onclick = () => toggleToken(btn.dataset.token);
  }});
}}

function renderList(flows) {{
  const tbody = document.getElementById("table-body");
  if (!flows.length) {{
    tbody.innerHTML = `<div class="empty-state">${{esc(I18N.emptyList)}}</div>`;
    return;
  }}
  const nearBottom = tbody.scrollHeight - tbody.scrollTop - tbody.clientHeight < 36;
  tbody.innerHTML = flows.map(flow => {{
    const selected = flow.id === state.selectedId ? " selected" : "";
    const failed = !!flow.error && flow.status == null;
    const statusText = flow.status == null ? (failed ? "ERR" : "—") : String(flow.status);
    const sCls = statusClass(flow);
    const pathText = failed ? (flow.error || "ERR") : (flow.path || flow.url || "/");
    const pathCls = failed ? "cell path mono failed" : "cell path mono";
    const slowCls = flow.slow ? "cell num mono slow" : "cell num mono";
    const pinHtml = flow.pinned ? `<span class="pin-dot"></span>` : "";
    return `<div class="row${{selected}}" data-id="${{flow.id}}">`
      + `<div class="cell id mono">${{flow.id}}</div>`
      + `<div class="cell method"><span class="pin-slot">${{pinHtml}}</span><span class="m-badge mono m-${{esc(flow.method)}}">${{esc(flow.method)}}</span></div>`
      + `<div class="${{pathCls}}" title="${{esc(flow.url || pathText)}}">${{esc(pathText)}}</div>`
      + `<div class="cell status mono ${{sCls}}">${{esc(statusText)}}</div>`
      + `<div class="cell num mono" title="${{esc(flow.started || "")}}">${{esc(flow.gap || "0 ms")}}</div>`
      + `<div class="${{slowCls}}">${{esc(flow.elapsed || "—")}}</div>`
      + `<div class="cell num mono">${{esc(flow.size || "0 B")}}</div>`
      + `</div>`;
  }}).join("");

  tbody.querySelectorAll(".row").forEach(row => {{
    row.onclick = () => {{
      state.selectedId = Number(row.dataset.id);
      renderAll();
    }};
  }});
  if (LIVE && nearBottom) {{
    tbody.scrollTop = tbody.scrollHeight;
  }}
}}

function formatHeaders(rows, pretty) {{
  let list = (rows || []).slice();
  if (pretty) {{
    list.sort((a, b) => a[0].toLowerCase().localeCompare(b[0].toLowerCase()));
  }}
  const out = [];
  for (const [k, v] of list) {{
    if (pretty && k.toLowerCase() === "cookie") {{
      const pairs = String(v).split(";").map(s => s.trim()).filter(Boolean);
      if (pairs.length > 1) {{
        for (const p of pairs) {{
          const eq = p.indexOf("=");
          if (eq > 0) out.push(["Cookie." + p.slice(0, eq).trim(), p.slice(eq + 1).trim()]);
          else out.push(["Cookie", p]);
        }}
        continue;
      }}
    }}
    out.push([k, v]);
  }}
  return out;
}}

function kvSection(key, title, rows) {{
  if (!rows || !rows.length) return "";
  const folded = state.folded[key] ? " folded" : "";
  const items = rows.map(r => `<div class="kv-row mono"><div class="kv-key">${{esc(r[0])}}</div><div class="kv-val">${{esc(r[1])}}</div></div>`).join("");
  return `<div class="card${{folded}}" data-fold="${{key}}">`
    + `<div class="card-head"><div class="card-title-wrap"><span class="card-arrow">▼</span><span class="card-title">${{esc(title)}}</span><span class="count-badge mono">${{rows.length}}</span></div></div>`
    + `<div class="card-body">${{items}}</div>`
    + `</div>`;
}}

function bodySection(key, title, raw, prettyText) {{
  if (!raw) return "";
  const folded = state.folded[key] ? " folded" : "";
  const shown = (state.pretty && prettyText) ? prettyText : raw;
  return `<div class="card${{folded}}" data-fold="${{key}}">`
    + `<div class="card-head">`
    + `<div class="card-title-wrap"><span class="card-arrow">▼</span><span class="card-title">${{esc(title)}}</span></div>`
    + `<div class="card-actions">`
    + `<button class="mini-btn toggle-pretty-btn ${{state.pretty ? "active" : ""}}" type="button">${{esc(state.pretty ? I18N.pretty : I18N.raw)}}</button>`
    + `<button class="mini-btn copy-body-btn" type="button">${{esc(I18N.copy)}}</button>`
    + `</div></div>`
    + `<div class="card-body"><pre class="code-pre mono">${{esc(shown)}}</pre></div>`
    + `</div>`;
}}

function renderDetail(flow) {{
  const pane = document.getElementById("right-pane");
  if (!flow) {{
    pane.innerHTML = `<div class="empty-state">${{esc(I18N.emptyDetail)}}</div>`;
    return;
  }}
  const failed = !!flow.error && flow.status == null;
  const statusText = flow.status == null ? (failed ? "ERR" : I18N.inProgress) : String(flow.status);
  const sBadgeCls = statusBadgeClass(flow);
  const ruleHtml = flow.rule_hit ? `<span class="rule-badge mono">${{esc(flow.rule_hit)}}</span>` : "";
  const errHtml = flow.error ? `<div class="err-box mono">${{esc(flow.error)}}</div>` : "";

  const mParts = [flow.started || ""];
  if (flow.ttfb_ms != null) mParts.push(`${{I18N.ttfb}} ${{flow.ttfb_ms}} ms`);
  mParts.push(flow.total_ms != null ? `${{I18N.total}} ${{flow.total_ms}} ms` : I18N.inProgress);
  mParts.push(`${{I18N.req}} ${{formatBytes(flow.request_bytes)}}`);
  mParts.push(`${{I18N.res}} ${{formatBytes(flow.response_bytes)}}`);

  const summaryHtml = `<div class="card summary-card">`
    + `<div class="summary-top">`
    + `<div class="summary-badges">`
    + `<span class="m-badge mono m-${{esc(flow.method)}}">${{esc(flow.method)}}</span>`
    + `<span class="${{sBadgeCls}} mono">${{esc(statusText)}}</span>`
    + `<span class="proto-badge mono">${{esc(flow.protocol || "HTTP/1.1")}}</span>`
    + ruleHtml
    + `</div>`
    + `<div class="card-actions">`
    + `<button class="mini-btn" id="copy-url-btn" type="button">${{esc(I18N.copyUrl)}}</button>`
    + `<button class="mini-btn" id="copy-curl-btn" type="button">${{esc(I18N.copyCurl)}}</button>`
    + `</div></div>`
    + `<div class="url-box mono">${{esc(flow.url || flow.path)}}</div>`
    + errHtml
    + `<div class="metrics-line mono">${{esc(mParts.filter(Boolean).join(" · "))}}</div>`
    + `</div>`;

  const resBodyHtml = bodySection("res_body", I18N.resBody, flow.response_body, flow.response_body_pretty);
  const resHeadersHtml = kvSection("res_headers", I18N.resHeaders, formatHeaders(flow.response_headers, state.pretty));
  const reqBodyHtml = bodySection("req_body", I18N.reqBody, flow.request_body, flow.request_body_pretty);
  const queryHtml = kvSection("query", I18N.queryParams, flow.query);
  const reqHeadersHtml = kvSection("req_headers", I18N.reqHeaders, formatHeaders(flow.request_headers, state.pretty));
  const msgRows = (flow.messages || []).map((m, i) => ["#" + (i + 1), m]);
  const msgHtml = kvSection("messages", I18N.messages, msgRows);

  pane.innerHTML = summaryHtml + resBodyHtml + resHeadersHtml + reqBodyHtml + queryHtml + reqHeadersHtml + msgHtml;

  const copyUrlBtn = document.getElementById("copy-url-btn");
  if (copyUrlBtn) copyUrlBtn.onclick = () => copyText(copyUrlBtn, flow.url || flow.path || "");
  const copyCurlBtn = document.getElementById("copy-curl-btn");
  if (copyCurlBtn) copyCurlBtn.onclick = () => copyText(copyCurlBtn, buildCurl(flow));

  pane.querySelectorAll(".card[data-fold]").forEach(card => {{
    const key = card.dataset.fold;
    const head = card.querySelector(".card-head");
    head.onclick = (e) => {{
      if (e.target.closest("button")) return;
      state.folded[key] = !state.folded[key];
      card.classList.toggle("folded", !!state.folded[key]);
    }};
    const pBtn = card.querySelector(".toggle-pretty-btn");
    if (pBtn) pBtn.onclick = () => {{ state.pretty = !state.pretty; renderAll(); }};
    const cBtn = card.querySelector(".copy-body-btn");
    if (cBtn) {{
      const rawText = key === "res_body" ? flow.response_body : flow.request_body;
      cBtn.onclick = () => copyText(cBtn, rawText || "");
    }}
  }});
}}

function renderAll() {{
  const allFlows = (state.data && state.data.flows) ? state.data.flows : [];
  const filtered = allFlows.filter(f => matchesFlow(f, state.filter));
  if (state.selectedId == null && filtered.length) {{
    state.selectedId = filtered[filtered.length - 1].id;
  }} else if (state.selectedId != null && !filtered.some(f => f.id === state.selectedId)) {{
    state.selectedId = filtered.length ? filtered[filtered.length - 1].id : null;
  }}
  renderToolbar(filtered.length, allFlows.length);
  renderList(filtered);
  const picked = filtered.find(f => f.id === state.selectedId) || null;
  renderDetail(picked);
  document.getElementById("footer-right").textContent = allFlows.length ? ("#" + (picked ? picked.id : "—")) : "";
}}

filterInput.oninput = () => {{
  state.filter = filterInput.value;
  renderAll();
}};
filterClear.onclick = () => {{
  state.filter = "";
  filterInput.value = "";
  renderAll();
}};
document.getElementById("pretty-btn").onclick = () => {{
  state.pretty = !state.pretty;
  renderAll();
}};
document.getElementById("theme-btn").onclick = () => {{
  state.theme = state.theme === "dark" ? "light" : "dark";
  document.documentElement.setAttribute("data-theme", state.theme);
  renderAll();
}};

window.addEventListener("keydown", (e) => {{
  if (document.activeElement === filterInput) return;
  if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
  const allFlows = (state.data && state.data.flows) ? state.data.flows : [];
  const filtered = allFlows.filter(f => matchesFlow(f, state.filter));
  if (!filtered.length) return;
  e.preventDefault();
  const idx = filtered.findIndex(f => f.id === state.selectedId);
  const nextIdx = e.key === "ArrowUp" ? Math.max(0, (idx < 0 ? 0 : idx - 1)) : Math.min(filtered.length - 1, (idx < 0 ? 0 : idx + 1));
  state.selectedId = filtered[nextIdx].id;
  renderAll();
  const rowEl = document.querySelector(`.row[data-id="${{state.selectedId}}"]`);
  if (rowEl) rowEl.scrollIntoView({{ block: "nearest" }});
}});

(() => {{
  const splitter = document.getElementById("splitter");
  const leftPane = document.getElementById("left-pane");
  let dragging = false;
  splitter.addEventListener("mousedown", (e) => {{
    dragging = true;
    splitter.classList.add("dragging");
    e.preventDefault();
  }});
  window.addEventListener("mousemove", (e) => {{
    if (!dragging) return;
    const maxW = Math.min(1100, window.innerWidth - 340);
    const w = Math.max(280, Math.min(maxW, e.clientX));
    leftPane.style.width = w + "px";
  }});
  window.addEventListener("mouseup", () => {{
    if (dragging) {{
      dragging = false;
      splitter.classList.remove("dragging");
    }}
  }});
}})();

renderAll();
if (LIVE) {{
  const pull = async () => {{
    try {{
      const res = await fetch(LIVE, {{cache: "no-store"}});
      if (!res.ok) {{ state.ended = true; renderAll(); return; }}
      state.data = await res.json();
      state.ended = false;
      renderAll();
    }} catch (_) {{
      state.ended = true;
      renderAll();
    }}
  }};
  setInterval(pull, 2000);
}}
</script>
</body>
</html>
"##,
        lang = if zh { "zh-CN" } else { "en" },
        title = title,
        live = live,
        json = json,
        zh_bool = if zh { "true" } else { "false" },
    )
}

pub fn expired_page(zh: bool) -> String {
    let title = if zh { "分享已结束" } else { "Share ended" };
    let body = if zh {
        "这个分享地址已经停止，或超过了有效期。"
    } else {
        "This share link was stopped, or it is past its expiry."
    };
    format!(
        "<!DOCTYPE html><html lang=\"{lang}\"><meta charset=\"utf-8\"><title>{title}</title><body style=\"background:#0d1017;color:#e2e8f0;font:16px sans-serif;padding:24px\"><h1>{title}</h1><p>{body}</p></body></html>",
        lang = if zh { "zh-CN" } else { "en" },
    )
}

pub fn prefers_zh(accept_language: Option<&str>) -> bool {
    let primary = accept_language
        .unwrap_or("")
        .split(',')
        .next()
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    primary.is_empty() || primary.to_ascii_lowercase().starts_with("zh")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_paths_accept_only_the_id() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        assert!(valid_id(id));
        assert_eq!(
            parse_share_path(&format!("/s/{id}")),
            Some((id.into(), SharePath::Page))
        );
        assert_eq!(
            parse_share_path(&format!("/s/{id}/flows.json")),
            Some((id.into(), SharePath::Data))
        );
        assert!(parse_share_path("/s/not-a-uuid").is_none());
        assert!(parse_share_path("/zniff").is_none());
        assert!(parse_share_path(&format!("http://example.com/s/{id}")).is_none());
    }

    #[test]
    fn grant_ends_at_the_deadline() {
        let share = LiveShare {
            id: "550e8400-e29b-41d4-a716-446655440000".into(),
            expires: UNIX_EPOCH + std::time::Duration::from_secs(100),
            filter: "header:cookie".into(),
        };
        assert!(grant_ok(
            &share,
            &share.id,
            UNIX_EPOCH + std::time::Duration::from_secs(99)
        ));
        assert!(!grant_ok(
            &share,
            &share.id,
            UNIX_EPOCH + std::time::Duration::from_secs(100)
        ));
        assert!(!grant_ok(&share, "other", UNIX_EPOCH));
    }

    #[test]
    fn page_escapes_a_script_in_the_body() {
        let json = document_from_body("</script><script>alert(1)</script>");
        let html = page(&json, "", true);
        assert!(!html.contains("</script><script>"), "{html}");
        assert!(html.contains("\\u003c"));
    }

    fn document_from_body(body: &str) -> String {
        let doc = ShareDoc {
            expires: 0,
            filter: String::new(),
            omitted: 0,
            flows: vec![ShareFlow {
                id: 1,
                method: "GET".into(),
                status: Some(200),
                error: None,
                protocol: "HTTP/1.1".into(),
                host: "example.test".into(),
                path: "/".into(),
                url: "http://example.test/".into(),
                started: "00:00:00.000".into(),
                gap: "0 ms".into(),
                elapsed: "10 ms".into(),
                slow: false,
                size: "12 B".into(),
                request_bytes: 0,
                response_bytes: 12,
                ttfb_ms: Some(4),
                total_ms: Some(10),
                pinned: false,
                rule_hit: String::new(),
                query: Vec::new(),
                request_headers: vec![("Cookie".into(), "<script>".into())],
                response_headers: Vec::new(),
                request_body: String::new(),
                request_body_pretty: String::new(),
                response_body: body.into(),
                response_body_pretty: String::new(),
                messages: Vec::new(),
            }],
        };
        serde_json::to_string(&doc).unwrap()
    }
}
