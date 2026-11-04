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
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 8 | 13 | 18 | 23) || byte.is_ascii_hexdigit()
        })
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
    request_headers: Vec<(String, String)>,
    response_headers: Vec<(String, String)>,
    request_body: String,
    response_body: String,
    messages: Vec<String>,
}

pub fn document(flows: &[Flow], expires: SystemTime, filter: &str) -> String {
    let omitted = flows.len().saturating_sub(FLOW_CAP);
    let flows = flows.iter().skip(omitted).map(share_flow).collect();
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

fn share_flow(flow: &Flow) -> ShareFlow {
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
        request_headers: flow.request_headers.clone(),
        response_headers: flow.response_headers.clone(),
        request_body: share_body(&flow.request_body, flow.request_truncated),
        response_body: share_body(&flow.response_body, flow.response_truncated),
        messages: flow
            .messages
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
            .collect(),
    }
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
    let waiting = if zh { "正在读取…" } else { "Loading…" };
    let expired = if zh {
        "分享已结束或已过期"
    } else {
        "This share has ended or expired"
    };
    let omitted = if zh {
        "较早的记录没有放进这一页"
    } else {
        "Older flows were left out of this page"
    };
    format!(
        r#"<!DOCTYPE html>
<html lang="{lang}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
  body {{ margin: 0; background: #0d1017; color: #e2e8f0; font: 14px/1.45 -apple-system, BlinkMacSystemFont, sans-serif; }}
  header {{ position: sticky; top: 0; background: #151b26; padding: 12px 16px; border-bottom: 1px solid #222938; }}
  h1 {{ font-size: 16px; margin: 0; }}
  p.meta {{ margin: 4px 0 0; color: #94a3b8; }}
  main {{ padding: 12px 16px 32px; }}
  details {{ background: #10141d; border: 1px solid #222938; border-radius: 10px; margin: 0 0 8px; }}
  summary {{ cursor: pointer; padding: 8px 12px; display: flex; gap: 10px; align-items: baseline; }}
  .m {{ font-weight: 700; min-width: 4.2rem; }}
  .s {{ min-width: 2.4rem; }}
  .p {{ flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: ui-monospace, Menlo, monospace; }}
  .t {{ color: #94a3b8; }}
  .ok {{ color: #34d399; }} .re {{ color: #fbbf24; }} .bad {{ color: #f87171; }}
  pre {{ margin: 0; padding: 8px 12px 12px; overflow: auto; max-height: 280px; white-space: pre-wrap; word-break: break-word; font: 12px/1.45 ui-monospace, Menlo, monospace; color: #cbd5e1; }}
  h2 {{ font-size: 12px; margin: 8px 12px 0; color: #94a3b8; }}
</style>
</head>
<body>
<header><h1>{title}</h1><p class="meta" id="meta">{waiting}</p></header>
<main id="app"></main>
<script>
const LIVE = "{live}";
const INITIAL = {json};
const EXPIRED = "{expired}";
const OMITTED = "{omitted}";
let openId = null;
function cls(status) {{
  if (status == null) return "s";
  if (status >= 500 || status === 0) return "s bad";
  if (status >= 400) return "s bad";
  if (status >= 300) return "s re";
  return "s ok";
}}
function esc(text) {{
  return String(text ?? "").replace(/[&<>"]/g, ch => ({{"&":"&amp;","<":"&lt;",">":"&gt;","\"":"&quot;"}}[ch]));
}}
function block(title, text) {{
  if (!text) return "";
  return "<h2>" + esc(title) + "</h2><pre>" + esc(text) + "</pre>";
}}
function headers(rows) {{
  return (rows || []).map(row => esc(row[0]) + ": " + esc(row[1])).join("\n");
}}
function render(data) {{
  const meta = document.getElementById("meta");
  const app = document.getElementById("app");
  if (!data || !data.flows) {{ meta.textContent = EXPIRED; app.textContent = ""; return; }}
  const when = new Date((data.expires || 0) * 1000);
  meta.textContent = (data.flows.length) + " · " + when.toLocaleString() + (data.filter ? " · " + data.filter : "") + (data.omitted ? " · " + OMITTED : "");
  app.innerHTML = data.flows.map(flow => {{
    const status = flow.status == null ? (flow.error ? "ERR" : "—") : flow.status;
    const open = flow.id === openId ? " open" : "";
    return "<details data-id=\"" + flow.id + "\"" + open + "><summary>"
      + "<span class=\"m\">" + esc(flow.method) + "</span>"
      + "<span class=\"" + cls(flow.status) + "\">" + esc(status) + "</span>"
      + "<span class=\"p\">" + esc(flow.path || flow.url) + "</span>"
      + "<span class=\"t\">" + esc(flow.started) + "</span>"
      + "</summary>"
      + block("URL", flow.url)
      + (flow.error ? block("error", flow.error) : "")
      + block("request", headers(flow.request_headers))
      + block("request body", flow.request_body)
      + block("response", headers(flow.response_headers))
      + block("response body", flow.response_body)
      + block("messages", (flow.messages || []).join("\n"))
      + "</details>";
  }}).join("");
  app.querySelectorAll("details").forEach(node => {{
    node.addEventListener("toggle", () => {{
      if (node.open) openId = Number(node.dataset.id);
      else if (openId === Number(node.dataset.id)) openId = null;
    }});
  }});
}}
render(INITIAL);
if (LIVE) {{
  const pull = async () => {{
    try {{
      const res = await fetch(LIVE, {{cache: "no-store"}});
      if (!res.ok) {{ render(null); return; }}
      render(await res.json());
    }} catch (err) {{
      render(null);
    }}
  }};
  setInterval(pull, 2000);
}}
</script>
</body>
</html>
"#,
        lang = if zh { "zh-CN" } else { "en" },
        title = title,
        waiting = waiting,
        live = live,
        json = json,
        expired = expired,
        omitted = omitted,
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
        assert_eq!(parse_share_path(&format!("/s/{id}")), Some((id.into(), SharePath::Page)));
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
        assert!(grant_ok(&share, &share.id, UNIX_EPOCH + std::time::Duration::from_secs(99)));
        assert!(!grant_ok(&share, &share.id, UNIX_EPOCH + std::time::Duration::from_secs(100)));
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
                request_headers: vec![("Cookie".into(), "<script>".into())],
                response_headers: Vec::new(),
                request_body: String::new(),
                response_body: body.into(),
                messages: Vec::new(),
            }],
        };
        serde_json::to_string(&doc).unwrap()
    }
}
