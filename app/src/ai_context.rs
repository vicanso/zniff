//! Generates a redacted, token-efficient Markdown diagnostic summary of a
//! captured HTTP(S) flow for pasting into an LLM / AI assistant ("Copy for AI").

use std::fmt::Write as _;

use crate::present::{content_type, flow_timing_breakdown, pretty_body};
use crate::redact::{RedactOptions, RedactReport, redact_flow};
use crate::session::{Direction, Flow};

/// Maximum UTF-8 bytes included per body excerpt in the Markdown summary.
pub const MAX_BODY_EXCERPT_BYTES: usize = 4096;
/// Maximum number of recent WebSocket frames included in the Markdown summary.
const MAX_WS_FRAMES: usize = 10;

/// Formats a captured [`Flow`] as a redacted Markdown diagnostic summary.
///
/// Redaction is always enforced even if `redact_opts.enabled` is `false`, while
/// honoring `redact_opts.strict` and any user-configured `extra_fields`.
pub fn format_flow_for_ai(flow: &Flow, redact_opts: &RedactOptions) -> String {
    let mut effective_opts = redact_opts.clone();
    effective_opts.enabled = true;
    let (redacted, report) = redact_flow(flow, &effective_opts);

    let mut out = String::with_capacity(1024);
    let _ = writeln!(
        out,
        "# HTTP Flow #{}: {} {}\n",
        redacted.id, redacted.method, redacted.url
    );

    write_summary_section(&mut out, &redacted, &report);
    write_error_section(&mut out, &redacted);
    write_timing_section(&mut out, &redacted);
    write_rules_section(&mut out, &redacted);
    write_request_section(&mut out, &redacted);
    write_response_section(&mut out, &redacted);
    write_ws_section(&mut out, &redacted);

    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

fn write_summary_section(out: &mut String, flow: &Flow, report: &RedactReport) {
    let _ = writeln!(out, "## Summary");
    let status_text = match flow.status {
        Some(code) => code.to_string(),
        None if flow.error.is_some() => "ERR".to_string(),
        None => "Pending".to_string(),
    };
    let _ = writeln!(out, "- **Protocol**: {}", flow.protocol.label());
    let _ = writeln!(out, "- **Status**: {status_text}");
    if !flow.host.is_empty() {
        let _ = writeln!(out, "- **Host**: `{}`", flow.host);
    }
    if !flow.client_ip.is_empty() {
        let _ = writeln!(out, "- **Client**: `{}`", flow.client_ip);
    }
    let _ = writeln!(
        out,
        "- **Size**: request {} B, response {} B",
        flow.req_bytes(),
        flow.res_bytes()
    );
    if let Some(ms) = flow.elapsed_ms() {
        let _ = writeln!(out, "- **Duration**: {ms} ms");
    }
    if let Some(note) = report.note() {
        let _ = writeln!(
            out,
            "- **Redaction**: {note} (headers: {}, query: {}, body: {})",
            report.redacted_headers, report.redacted_query_params, report.redacted_body_fields
        );
    }
    let clean_comment = flow.comment.trim();
    if !clean_comment.is_empty() && !clean_comment.starts_with("[REDACTED") {
        let _ = writeln!(out, "- **Note**: {clean_comment}");
    }
    out.push('\n');
}

fn write_error_section(out: &mut String, flow: &Flow) {
    let Some(err) = flow.error.as_deref().filter(|e| !e.trim().is_empty()) else {
        return;
    };
    let _ = writeln!(out, "## Error");
    let _ = writeln!(out, "> {}\n", err.replace('\n', "\n> "));
}

fn write_timing_section(out: &mut String, flow: &Flow) {
    let tb = flow_timing_breakdown(flow);
    if !tb.has_content() {
        return;
    }
    let _ = writeln!(out, "## Timing & Connection");
    let mut conn_parts = Vec::new();
    if tb.reused {
        conn_parts.push("reused connection".to_string());
    } else if tb.dns_ms.is_some() || tb.tcp_ms.is_some() || tb.tls_ms.is_some() {
        conn_parts.push("new connection".to_string());
    }
    if !tb.server_addr.is_empty() {
        conn_parts.push(format!("server `{}`", tb.server_addr));
    }
    if !tb.tls_version.is_empty() {
        conn_parts.push(format!("TLS `{}`", tb.tls_version));
    }
    if !tb.client_cert.is_empty() {
        conn_parts.push(format!("mTLS `{}`", tb.client_cert));
    }
    if !conn_parts.is_empty() {
        let _ = writeln!(out, "- **Connection**: {}", conn_parts.join(", "));
    }

    let mut phases = Vec::new();
    if let Some(ms) = tb.dns_ms {
        phases.push(format!("DNS {ms} ms"));
    }
    if let Some(ms) = tb.tcp_ms {
        phases.push(format!("TCP {ms} ms"));
    }
    if let Some(ms) = tb.tls_ms {
        phases.push(format!("TLS {ms} ms"));
    }
    if let Some(ms) = tb.wait_ms {
        phases.push(format!("Wait (TTFB) {ms} ms"));
    }
    if let Some(ms) = tb.download_ms {
        phases.push(format!("Download {ms} ms"));
    }
    if !phases.is_empty() {
        let _ = writeln!(
            out,
            "- **Phases**: {} (total {} ms)",
            phases.join(" -> "),
            tb.total_ms
        );
    }
    out.push('\n');
}

fn write_rules_section(out: &mut String, flow: &Flow) {
    if flow.rule_hits.is_empty() && flow.rule_hit.trim().is_empty() {
        return;
    }
    let _ = writeln!(out, "## Matched Rules");
    if !flow.rule_hits.is_empty() {
        for hit in &flow.rule_hits {
            let _ = writeln!(out, "- `{}`", hit.format_badge());
        }
    } else {
        let _ = writeln!(out, "- `{}`", flow.rule_hit.trim());
    }
    out.push('\n');
}

fn write_request_section(out: &mut String, flow: &Flow) {
    let _ = writeln!(out, "## Request");
    let _ = writeln!(out, "```http");
    let _ = writeln!(
        out,
        "{} {} {}",
        flow.method,
        flow.url,
        flow.protocol.label()
    );
    for (name, value) in &flow.request_headers {
        let _ = writeln!(out, "{name}: {value}");
    }
    let _ = writeln!(out, "```\n");

    let req_ct = content_type(&flow.request_headers);
    write_body_excerpt(
        out,
        "Request Body",
        &flow.request_body,
        req_ct,
        flow.request_truncated,
        flow.request_expired,
        flow.request_encoding_error.as_deref(),
    );
}

fn write_response_section(out: &mut String, flow: &Flow) {
    if flow.status.is_none()
        && flow.response_headers.is_empty()
        && flow.response_body.is_empty()
        && flow.response_expired.is_none()
    {
        return;
    }
    let _ = writeln!(out, "## Response");
    let _ = writeln!(out, "```http");
    let status_line = match flow.status {
        Some(code) => format!("{} {code}", flow.protocol.label()),
        None => format!("{} (no status)", flow.protocol.label()),
    };
    let _ = writeln!(out, "{status_line}");
    for (name, value) in &flow.response_headers {
        let _ = writeln!(out, "{name}: {value}");
    }
    let _ = writeln!(out, "```\n");

    let res_ct = content_type(&flow.response_headers);
    write_body_excerpt(
        out,
        "Response Body",
        &flow.response_body,
        res_ct,
        flow.response_truncated,
        flow.response_expired,
        flow.response_encoding_error.as_deref(),
    );
}

fn write_ws_section(out: &mut String, flow: &Flow) {
    if flow.messages.is_empty() {
        return;
    }
    let total = flow.messages.len();
    let shown_start = total.saturating_sub(MAX_WS_FRAMES);
    let _ = writeln!(out, "## WebSocket Messages ({total} captured)");
    if shown_start > 0 {
        let _ = writeln!(out, "_Showing last {MAX_WS_FRAMES} of {total} frames._");
    }
    for msg in &flow.messages[shown_start..] {
        let dir = match msg.direction {
            Direction::Request => "client->server",
            Direction::Response => "server->client",
        };
        let (excerpt, truncated) = truncate_utf8(&msg.text, 256);
        let suffix = if truncated { "…" } else { "" };
        let single_line = excerpt.replace(['\r', '\n'], " ");
        let _ = writeln!(
            out,
            "- [{dir}] ({}, {} B): `{single_line}{suffix}`",
            msg.kind,
            msg.text.len()
        );
    }
    out.push('\n');
}

fn write_body_excerpt(
    out: &mut String,
    heading: &str,
    body: &[u8],
    ct: Option<&str>,
    captured_truncated: bool,
    expired_bytes: Option<usize>,
    encoding_error: Option<&str>,
) {
    if body.is_empty() {
        if let Some(bytes) = expired_bytes {
            let _ = writeln!(
                out,
                "### {heading}\n_Body expired from temporary spill store ({bytes} bytes)._\n"
            );
        }
        return;
    }

    let _ = writeln!(out, "### {heading}");
    if let Some(err) = encoding_error {
        let _ = writeln!(out, "_Decompression error: {err}_");
    }

    let Ok(raw_text) = std::str::from_utf8(body) else {
        let ct_label = ct.unwrap_or("application/octet-stream");
        let _ = writeln!(
            out,
            "_Binary payload ({} bytes, Content-Type: `{ct_label}`)._\n",
            body.len()
        );
        return;
    };

    let formatted = if ct.is_some_and(|c| c.to_ascii_lowercase().contains("json")) {
        pretty_body(ct, raw_text)
    } else {
        raw_text.to_string()
    };

    let (excerpt, excerpt_truncated) = truncate_utf8(&formatted, MAX_BODY_EXCERPT_BYTES);
    let lang = fence_lang(ct);
    let fence = safe_fence(excerpt);
    let _ = writeln!(out, "{fence}{lang}");
    let _ = writeln!(out, "{excerpt}");
    let _ = writeln!(out, "{fence}");

    if excerpt_truncated || captured_truncated {
        let mut notes = Vec::new();
        if excerpt_truncated {
            notes.push(format!(
                "excerpt truncated to {} of {} bytes",
                excerpt.len(),
                formatted.len()
            ));
        }
        if captured_truncated {
            notes.push("capture reached 1 MB body cap".to_string());
        }
        let _ = writeln!(out, "_{}._", notes.join("; "));
    }
    out.push('\n');
}

fn fence_lang(ct: Option<&str>) -> &'static str {
    let lower = ct.unwrap_or("").to_ascii_lowercase();
    if lower.contains("json") || lower.contains("graphql") {
        "json"
    } else if lower.contains("html") {
        "html"
    } else if lower.contains("xml") {
        "xml"
    } else if lower.contains("javascript") || lower.contains("ecmascript") {
        "js"
    } else if lower.contains("css") {
        "css"
    } else {
        "text"
    }
}

fn safe_fence(text: &str) -> String {
    let mut max_run = 0usize;
    let mut run = 0usize;
    for byte in text.bytes() {
        if byte == b'`' {
            run += 1;
            max_run = max_run.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat((max_run + 1).max(3))
}

fn truncate_utf8(text: &str, max_bytes: usize) -> (&str, bool) {
    if text.len() <= max_bytes {
        return (text, false);
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{ConnInfo, Direction, FlowDraft, Protocol, RuleHit, Session};
    use std::time::Duration;

    fn make_flow() -> Flow {
        let mut session = Session::default();
        let id = session.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "POST".into(),
            host: "api.example.com".into(),
            path: "/v1/orders?token=secret123&page=1".into(),
            url: "https://api.example.com/v1/orders?token=secret123&page=1".into(),
            request_headers: vec![
                ("Authorization".into(), "Bearer super-secret-jwt".into()),
                ("Cookie".into(), "session=abc123".into()),
                ("Content-Type".into(), "application/json".into()),
            ],
        });
        session.append_body(
            id,
            Direction::Request,
            br#"{"password":"p@ss","item":"book"}"#,
        );
        session.set_response(
            id,
            502,
            vec![("Content-Type".into(), "application/json".into())],
            None,
        );
        session.append_body(
            id,
            Direction::Response,
            br#"{"error":"upstream timeout","api_key":"sk-live-999"}"#,
        );
        let mut flow = session.get(id).unwrap().clone();
        flow.client_ip = "127.0.0.1 (curl)".into();
        flow.error = Some("upstream reset connection".into());
        flow.conn_info = ConnInfo {
            dns_ms: Some(12),
            tcp_ms: Some(25),
            tls_ms: Some(48),
            reused: false,
            server_addr: "93.184.216.34:443".into(),
            tls_version: "TLSv1.3".into(),
            client_cert: String::new(),
            dialed_before_start: false,
        };
        flow.response_at = Some(flow.started + Duration::from_millis(180));
        flow.finished_at = Some(flow.started + Duration::from_millis(220));
        flow.rule_hits = vec![RuleHit {
            group_name: Some("dev".into()),
            line_no: 4,
            verb: "delay".into(),
        }];
        flow
    }

    #[test]
    fn formats_redacted_markdown_with_timing_error_and_rules() {
        let flow = make_flow();
        // Even when opts.enabled is false, Copy for AI must enforce redaction.
        let md = format_flow_for_ai(&flow, &RedactOptions::default());

        assert!(md.contains("# HTTP Flow #1: POST https://api.example.com/v1/orders?"));
        assert!(!md.contains("super-secret-jwt"));
        assert!(!md.contains("secret123"));
        assert!(!md.contains("p@ss"));
        assert!(!md.contains("sk-live-999"));
        assert!(md.contains("\"item\": \"book\""));
        assert!(md.contains("\"error\": \"upstream timeout\""));
        assert!(md.contains("## Error\n> upstream reset connection"));
        assert!(md.contains("DNS 12 ms -> TCP 25 ms -> TLS 48 ms"));
        assert!(md.contains("TLS `TLSv1.3`"));
        assert!(md.contains("## Matched Rules\n- `[dev] delay #4`"));
    }

    #[test]
    fn truncates_large_utf8_body_cleanly_and_handles_backticks_and_binary() {
        let mut session = Session::default();
        let id = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "POST".into(),
            host: "example.com".into(),
            path: "/upload".into(),
            url: "https://example.com/upload".into(),
            request_headers: vec![("Content-Type".into(), "application/octet-stream".into())],
        });
        session.append_body(id, Direction::Request, &[0xFF, 0xFE, 0x00, 0x01]);
        session.set_response(
            id,
            200,
            vec![("Content-Type".into(), "text/plain".into())],
            None,
        );
        let mut big = "```triple```\n".to_string();
        big.push_str(&"中".repeat(2000));
        session.append_body(id, Direction::Response, big.as_bytes());
        let flow = session.get(id).unwrap().clone();

        let md = format_flow_for_ai(&flow, &RedactOptions::default());
        assert!(md.contains("Binary payload (4 bytes, Content-Type: `application/octet-stream`)"));
        assert!(md.contains("````text\n```triple```"));
        assert!(md.contains("excerpt truncated to"));
    }
}
