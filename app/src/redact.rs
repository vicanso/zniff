//! Flow redaction helpers for safe HTML/HAR export, live share, and preview.

use crate::session::Flow;

/// Placeholder written in place of unstructured or binary bodies when `RedactOptions::strict` is on.
pub const STRICT_REDACTED_BODY: &[u8] =
    b"[REDACTED: unstructured or binary body omitted in strict mode]";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RedactOptions {
    pub enabled: bool,
    pub strict: bool,
    pub extra_fields: Vec<String>,
}

impl RedactOptions {
    pub fn new(enabled: bool, extra_raw: &str) -> Self {
        let mut strict = false;
        let extra_fields = extra_raw
            .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .filter_map(|s| {
                if s.eq_ignore_ascii_case("@strict") {
                    strict = true;
                    None
                } else {
                    Some(s.to_ascii_lowercase())
                }
            })
            .collect();
        Self {
            enabled,
            strict,
            extra_fields,
        }
    }

    pub const fn with_strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn to_wire_fields(&self) -> String {
        let mut items = self.extra_fields.clone();
        if self.strict {
            items.push("@strict".to_string());
        }
        items.join(", ")
    }

    pub fn is_sensitive_header(&self, name: &str) -> bool {
        let lower = name.trim().to_ascii_lowercase();
        let compact: String = lower.chars().filter(|&c| c != '-' && c != '_').collect();
        const COMPACT_DEFAULT_HEADERS: &[&str] = &[
            "authorization",
            "proxyauthorization",
            "cookie",
            "setcookie",
            "xapikey",
            "xauthtoken",
            "xaccesstoken",
            "xcsrftoken",
            "xxsrftoken",
            "apikey",
        ];
        COMPACT_DEFAULT_HEADERS.contains(&compact.as_str())
            || self.extra_fields.iter().any(|extra| {
                let extra_compact: String =
                    extra.chars().filter(|&c| c != '-' && c != '_').collect();
                extra == &lower || extra_compact == compact
            })
    }

    pub fn is_sensitive_field(&self, name: &str) -> bool {
        let lower = name.trim().to_ascii_lowercase();
        let compact: String = lower.chars().filter(|&c| c != '-' && c != '_').collect();
        const COMPACT_DEFAULT_FIELDS: &[&str] = &[
            "password",
            "passwd",
            "pass",
            "pwd",
            "secret",
            "clientsecret",
            "token",
            "accesstoken",
            "refreshtoken",
            "idtoken",
            "authtoken",
            "apikey",
            "privatekey",
            "sessionid",
            "sid",
            "session",
            "jwt",
            "authorization",
            "proxyauthorization",
            "credential",
            "credentials",
            "cvv",
            "cvc",
            "ssn",
            "pin",
            "cookie",
            "cookies",
            "setcookie",
            "xapikey",
            "xauthtoken",
            "xaccesstoken",
            "xcsrftoken",
            "xxsrftoken",
            "csrftoken",
            "xsrftoken",
            "csrf",
            "xsrf",
        ];
        COMPACT_DEFAULT_FIELDS.contains(&compact.as_str())
            || self.is_sensitive_header(name)
            || self.extra_fields.iter().any(|extra| {
                let extra_compact: String =
                    extra.chars().filter(|&c| c != '-' && c != '_').collect();
                extra == &lower || extra_compact == compact
            })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RedactReport {
    pub redacted_headers: usize,
    pub redacted_query_params: usize,
    pub redacted_body_fields: usize,
    pub unsanitizable_req_body: bool,
    pub unsanitizable_res_body: bool,
    pub strict_masked_req_body: bool,
    pub strict_masked_res_body: bool,
}

impl RedactReport {
    pub fn total_redacted(&self) -> usize {
        self.redacted_headers + self.redacted_query_params + self.redacted_body_fields
    }

    pub fn has_unsanitizable_body(&self) -> bool {
        self.unsanitizable_req_body || self.unsanitizable_res_body
    }

    pub fn has_strict_masked_body(&self) -> bool {
        self.strict_masked_req_body || self.strict_masked_res_body
    }

    pub fn note(&self) -> Option<String> {
        let total = self.total_redacted();
        match (
            self.unsanitizable_req_body,
            self.unsanitizable_res_body,
            self.strict_masked_req_body,
            self.strict_masked_res_body,
        ) {
            (true, true, _, _) => Some("[REDACTED; req/res body format not sanitizable]".into()),
            (true, false, _, _) => Some("[REDACTED; req body format not sanitizable]".into()),
            (false, true, _, _) => Some("[REDACTED; res body format not sanitizable]".into()),
            (false, false, true, true) => {
                Some("[REDACTED; req/res body omitted in strict mode]".into())
            }
            (false, false, true, false) => {
                Some("[REDACTED; req body omitted in strict mode]".into())
            }
            (false, false, false, true) => {
                Some("[REDACTED; res body omitted in strict mode]".into())
            }
            (false, false, false, false) if total > 0 => Some("[REDACTED]".into()),
            (false, false, false, false) => None,
        }
    }
}

pub fn redact_flow(flow: &Flow, opts: &RedactOptions) -> (Flow, RedactReport) {
    let mut out = flow.clone();
    if !opts.enabled {
        return (out, RedactReport::default());
    }
    let mut report = RedactReport::default();

    for (name, value) in &mut out.request_headers {
        if let Some(new_val) = redact_header_value(name, value, opts)
            && *value != new_val
        {
            *value = new_val;
            report.redacted_headers += 1;
        }
    }
    for (name, value) in &mut out.response_headers {
        if let Some(new_val) = redact_header_value(name, value, opts)
            && *value != new_val
        {
            *value = new_val;
            report.redacted_headers += 1;
        }
    }

    let (new_url, url_q_count) = redact_url_query(&out.url, opts);
    out.url = new_url;
    let (new_path, path_q_count) = redact_url_query(&out.path, opts);
    out.path = new_path;
    report.redacted_query_params += url_q_count.max(path_q_count);

    let req_ct = crate::present::content_type(&out.request_headers)
        .unwrap_or("")
        .to_string();
    if !out.request_body.is_empty() {
        let (new_body, count, unsanitizable, strict_masked) =
            redact_body_bytes(&out.request_body, &req_ct, opts);
        out.request_body = new_body;
        report.redacted_body_fields += count;
        report.unsanitizable_req_body = unsanitizable;
        report.strict_masked_req_body = strict_masked;
    }

    let res_ct = crate::present::content_type(&out.response_headers)
        .unwrap_or("")
        .to_string();
    if !out.response_body.is_empty() {
        let (new_body, count, unsanitizable, strict_masked) =
            redact_body_bytes(&out.response_body, &res_ct, opts);
        out.response_body = new_body;
        report.redacted_body_fields += count;
        report.unsanitizable_res_body = unsanitizable;
        report.strict_masked_res_body = strict_masked;
    }

    for msg in &mut out.messages {
        let is_binary_frame = msg.kind == "二进制" || msg.kind.eq_ignore_ascii_case("binary");
        if opts.strict && is_binary_frame {
            let strict_str = String::from_utf8_lossy(STRICT_REDACTED_BODY).into_owned();
            if msg.text != strict_str {
                msg.text = strict_str;
                report.redacted_body_fields += 1;
            }
        } else if let Ok(mut json_val) = serde_json::from_str::<serde_json::Value>(&msg.text) {
            let count = redact_json_value(&mut json_val, opts);
            if count > 0
                && let Ok(serialized) = serde_json::to_string(&json_val)
            {
                msg.text = serialized;
                report.redacted_body_fields += count;
            }
        } else {
            let count = redact_string_value(&mut msg.text, opts, 0);
            report.redacted_body_fields += count;
        }
    }

    if let Some(note) = report.note() {
        if out.comment.is_empty() {
            out.comment = note;
        } else if !out.comment.contains("[REDACTED") {
            out.comment = format!("{} {note}", out.comment);
        }
    }

    (out, report)
}

fn redact_header_value(name: &str, value: &str, opts: &RedactOptions) -> Option<String> {
    if !opts.is_sensitive_header(name) {
        return None;
    }
    let lower = name.trim().to_ascii_lowercase();
    if lower == "authorization" || lower == "proxy-authorization" {
        let trimmed = value.trim();
        if let Some((scheme, rest)) = trimmed.split_once(char::is_whitespace)
            && !rest.trim().is_empty()
        {
            return Some(format!("{scheme} [REDACTED]"));
        }
        return Some("[REDACTED]".into());
    }
    if lower == "cookie" {
        let parts: Vec<String> = value
            .split(';')
            .map(|seg| {
                let s = seg.trim();
                if let Some((k, _)) = s.split_once('=') {
                    format!("{}=[REDACTED]", k.trim())
                } else if s.is_empty() {
                    String::new()
                } else {
                    "[REDACTED]".into()
                }
            })
            .filter(|s| !s.is_empty())
            .collect();
        return Some(parts.join("; "));
    }
    if lower == "set-cookie" {
        let mut iter = value.split(';');
        let first = iter.next().unwrap_or("").trim();
        let redacted_first = if let Some((k, _)) = first.split_once('=') {
            format!("{}=[REDACTED]", k.trim())
        } else {
            "[REDACTED]".into()
        };
        let mut out = vec![redacted_first];
        for attr in iter {
            let attr_trimmed = attr.trim();
            if !attr_trimmed.is_empty() {
                out.push(attr_trimmed.to_string());
            }
        }
        return Some(out.join("; "));
    }
    Some("[REDACTED]".into())
}

fn redact_url_query(url_or_path: &str, opts: &RedactOptions) -> (String, usize) {
    let Some((before_q, after_q)) = url_or_path.split_once('?') else {
        return (url_or_path.to_string(), 0);
    };
    let (query, frag) = match after_q.split_once('#') {
        Some((q, f)) => (q, Some(f)),
        None => (after_q, None),
    };
    let (new_query, count) = redact_urlencoded_pairs(query, opts);
    if count == 0 {
        return (url_or_path.to_string(), 0);
    }
    let mut out = format!("{before_q}?{new_query}");
    if let Some(f) = frag {
        out.push('#');
        out.push_str(f);
    }
    (out, count)
}

fn percent_encode_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() * 3);
    for b in raw.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

fn redact_urlencoded_pairs(query: &str, opts: &RedactOptions) -> (String, usize) {
    let mut count = 0;
    let mut parts = Vec::new();
    for seg in query.split('&') {
        if let Some((k, v)) = seg.split_once('=') {
            let decoded_k = percent_decode_key(k);
            if opts.is_sensitive_field(&decoded_k) && v != "[REDACTED]" && v != "%5BREDACTED%5D" {
                parts.push(format!("{k}=[REDACTED]"));
                count += 1;
            } else if v.contains('?') {
                let (new_v, v_count) = redact_url_query(v, opts);
                if v_count > 0 {
                    parts.push(format!("{k}={new_v}"));
                    count += v_count;
                } else {
                    parts.push(seg.to_string());
                }
            } else if v.contains("%3F")
                || v.contains("%3f")
                || ((v.contains("%3D") || v.contains("%3d"))
                    && (v.contains("%26")
                        || percent_decode_key(v)
                            .split_once('=')
                            .is_some_and(|(sub_k, _)| {
                                opts.is_sensitive_field(&percent_decode_key(sub_k))
                            })))
            {
                let decoded_v = percent_decode_key(v);
                let (new_decoded_v, v_count) = if decoded_v.contains('?') {
                    redact_url_query(&decoded_v, opts)
                } else {
                    redact_urlencoded_pairs(&decoded_v, opts)
                };
                if v_count > 0 {
                    let re_encoded = percent_encode_component(&new_decoded_v);
                    parts.push(format!("{k}={re_encoded}"));
                    count += v_count;
                } else {
                    parts.push(seg.to_string());
                }
            } else {
                parts.push(seg.to_string());
            }
        } else {
            parts.push(seg.to_string());
        }
    }
    (parts.join("&"), count)
}

fn percent_decode_key(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3])
            && let Ok(val) = u8::from_str_radix(hex, 16)
        {
            out.push(val);
            i += 3;
            continue;
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn redact_sse_payload(payload: &str, opts: &RedactOptions) -> (String, usize) {
    let trimmed = payload.trim();
    if !trimmed.is_empty()
        && ((trimmed.starts_with('{') && trimmed.ends_with('}'))
            || (trimmed.starts_with('[') && trimmed.ends_with(']')))
        && let Ok(mut val) = serde_json::from_str::<serde_json::Value>(trimmed)
    {
        let count = redact_json_value(&mut val, opts);
        if count > 0
            && let Ok(serialized) = serde_json::to_string(&val)
        {
            return (serialized, count);
        }
        return (payload.to_string(), 0);
    }
    let mut s = payload.to_string();
    let count = redact_string_value(&mut s, opts, 0);
    (s, count)
}

fn redact_sse_event_block(lines: &[(&str, &str)], opts: &RedactOptions) -> (String, usize) {
    // Check if multiple `data:` lines in one SSE event combine into a single JSON document.
    let mut data_parts: Vec<&str> = Vec::new();
    for &(line, _) in lines {
        if let Some(rest) = line.strip_prefix("data:") {
            data_parts.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if data_parts.len() > 1 {
        let joined = data_parts.join("\n");
        let trimmed = joined.trim();
        if ((trimmed.starts_with('{') && trimmed.ends_with('}'))
            || (trimmed.starts_with('[') && trimmed.ends_with(']')))
            && let Ok(mut val) = serde_json::from_str::<serde_json::Value>(trimmed)
        {
            let count = redact_json_value(&mut val, opts);
            if count > 0
                && let Ok(serialized) = serde_json::to_string(&val)
            {
                let mut out = String::new();
                let mut emitted_data = false;
                for &(line, ending) in lines {
                    if line.starts_with("data:") {
                        if !emitted_data {
                            out.push_str("data: ");
                            out.push_str(&serialized);
                            out.push_str(ending);
                            emitted_data = true;
                        }
                    } else {
                        out.push_str(line);
                        out.push_str(ending);
                    }
                }
                return (out, count);
            }
            let mut unchanged = String::new();
            for &(line, ending) in lines {
                unchanged.push_str(line);
                unchanged.push_str(ending);
            }
            return (unchanged, 0);
        }
    }

    let mut out = String::new();
    let mut total = 0;
    for &(line, ending) in lines {
        if let Some(rest) = line.strip_prefix("data:") {
            let (prefix, payload) = match rest.strip_prefix(' ') {
                Some(p) => ("data: ", p),
                None => ("data:", rest),
            };
            let (new_payload, count) = redact_sse_payload(payload, opts);
            if count > 0 {
                out.push_str(prefix);
                out.push_str(&new_payload);
                out.push_str(ending);
                total += count;
            } else {
                out.push_str(line);
                out.push_str(ending);
            }
        } else {
            out.push_str(line);
            out.push_str(ending);
        }
    }
    (out, total)
}

fn redact_event_stream(text: &str, opts: &RedactOptions) -> (String, usize) {
    let mut out = String::with_capacity(text.len());
    let mut total_count = 0;
    let mut event_lines: Vec<(&str, &str)> = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let (line, ending, next) = match rest.find('\n') {
            Some(nl) => {
                if nl > 0 && rest.as_bytes()[nl - 1] == b'\r' {
                    (&rest[..nl - 1], "\r\n", &rest[nl + 1..])
                } else {
                    (&rest[..nl], "\n", &rest[nl + 1..])
                }
            }
            None => (rest, "", ""),
        };
        rest = next;
        if line.is_empty() {
            if !event_lines.is_empty() {
                let (block_out, c) = redact_sse_event_block(&event_lines, opts);
                out.push_str(&block_out);
                total_count += c;
                event_lines.clear();
            }
            out.push_str(ending);
        } else {
            event_lines.push((line, ending));
        }
    }
    if !event_lines.is_empty() {
        let (block_out, c) = redact_sse_event_block(&event_lines, opts);
        out.push_str(&block_out);
        total_count += c;
    }
    (out, total_count)
}

fn extract_multipart_boundary_param(content_type: &str) -> Option<String> {
    let mut parts = content_type.split(';');
    let mime = parts.next()?.trim();
    if !mime.eq_ignore_ascii_case("multipart/form-data") {
        return None;
    }
    for param in parts {
        let param = param.trim();
        if let Some((k, v)) = param.split_once('=')
            && k.trim().eq_ignore_ascii_case("boundary")
        {
            let val = v.trim().trim_matches('"').trim_matches('\'');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

fn extract_disposition_param(disposition: &str, target: &str) -> Option<String> {
    for seg in disposition.split(';').skip(1) {
        let seg = seg.trim();
        let Some((k, v)) = seg.split_once('=') else {
            continue;
        };
        let key = k.trim();
        if key.eq_ignore_ascii_case(target) {
            let val = v.trim().trim_matches('"').trim_matches('\'');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        } else if key.eq_ignore_ascii_case(&format!("{target}*")) {
            let raw = v.trim().trim_matches('"');
            let encoded = raw.split_once("''").map(|(_, rest)| rest).unwrap_or(raw);
            let decoded = percent_decode_key(encoded);
            if !decoded.is_empty() {
                return Some(decoded);
            }
        }
    }
    None
}

fn filename_matches_sensitive(filename: &str, opts: &RedactOptions) -> bool {
    let base = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    opts.is_sensitive_field(base)
        || base
            .rsplit_once('.')
            .is_some_and(|(stem, _)| !stem.is_empty() && opts.is_sensitive_field(stem))
}

fn find_boundary_positions(body: &[u8], delim: &[u8]) -> Vec<usize> {
    if delim.is_empty() || body.len() < delim.len() {
        return Vec::new();
    }
    let mut positions = Vec::new();
    let mut i = 0;
    while i + delim.len() <= body.len() {
        if &body[i..i + delim.len()] == delim && (i == 0 || body[i - 1] == b'\n') {
            positions.push(i);
            i += delim.len();
        } else {
            i += 1;
        }
    }
    positions
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn redact_multipart_body(
    body: &[u8],
    boundary: &str,
    opts: &RedactOptions,
) -> Option<(Vec<u8>, usize, bool, bool)> {
    let delim = format!("--{boundary}");
    let delim_bytes = delim.as_bytes();
    let positions = find_boundary_positions(body, delim_bytes);
    if positions.len() < 2 {
        return None;
    }

    let mut out = Vec::with_capacity(body.len());
    out.extend_from_slice(&body[..positions[0]]);

    let mut total_count = 0usize;
    let mut any_unsanitizable = false;
    let mut any_strict_masked = false;
    let mut changed = false;
    let mut saw_closing = false;

    for (idx, &pos) in positions.iter().enumerate() {
        let after_delim = pos + delim_bytes.len();
        if body[after_delim..].starts_with(b"--") {
            out.extend_from_slice(&body[pos..]);
            saw_closing = true;
            break;
        }
        let after_nl_len = if body[after_delim..].starts_with(b"\r\n") {
            2
        } else if body[after_delim..].starts_with(b"\n") {
            1
        } else {
            return None;
        };
        let part_start = after_delim + after_nl_len;
        let &next_pos = positions.get(idx + 1)?;
        let (part_end, pre_next_nl): (usize, &[u8]) =
            if next_pos >= 2 && &body[next_pos - 2..next_pos] == b"\r\n" {
                (next_pos - 2, b"\r\n")
            } else if next_pos >= 1 && body[next_pos - 1] == b'\n' {
                (next_pos - 1, b"\n")
            } else {
                return None;
            };
        if part_end < part_start {
            return None;
        }
        let part = &body[part_start..part_end];
        let (sep_pos, sep_len) = if let Some(p) = find_bytes(part, b"\r\n\r\n") {
            (p, 4)
        } else {
            let p = find_bytes(part, b"\n\n")?;
            (p, 2)
        };
        let raw_headers = &part[..sep_pos];
        let sep = &part[sep_pos..sep_pos + sep_len];
        let payload = &part[sep_pos + sep_len..];
        let headers_str = std::str::from_utf8(raw_headers).ok()?;

        let mut disp_name: Option<String> = None;
        let mut disp_filename: Option<String> = None;
        let mut part_ct: Option<String> = None;
        for line in headers_str.lines() {
            let line = line.trim_end_matches('\r');
            let Some((h_name, h_val)) = line.split_once(':') else {
                continue;
            };
            let h_name = h_name.trim();
            let h_val = h_val.trim();
            if h_name.eq_ignore_ascii_case("content-disposition") {
                disp_name = extract_disposition_param(h_val, "name");
                disp_filename = extract_disposition_param(h_val, "filename");
            } else if h_name.eq_ignore_ascii_case("content-type") {
                part_ct = Some(h_val.to_string());
            }
        }

        let field_sensitive = disp_name
            .as_deref()
            .is_some_and(|n| opts.is_sensitive_field(n))
            || disp_filename
                .as_deref()
                .is_some_and(|f| filename_matches_sensitive(f, opts));

        let new_payload: Vec<u8> = if field_sensitive {
            if payload == b"[REDACTED]" || payload == STRICT_REDACTED_BODY {
                payload.to_vec()
            } else {
                total_count += 1;
                changed = true;
                b"[REDACTED]".to_vec()
            }
        } else if payload.is_empty() || payload == STRICT_REDACTED_BODY {
            payload.to_vec()
        } else {
            let ct_str = part_ct.as_deref().unwrap_or("");
            let ct_lower = ct_str.to_ascii_lowercase();
            let is_plain_form_value = disp_filename.is_none()
                && (ct_lower.is_empty() || ct_lower.starts_with("text/plain"));
            if is_plain_form_value {
                match std::str::from_utf8(payload) {
                    Ok(text) => {
                        let trimmed = text.trim();
                        if ((trimmed.starts_with('{') && trimmed.ends_with('}'))
                            || (trimmed.starts_with('[') && trimmed.ends_with(']')))
                            && serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
                        {
                            let (sub_out, sub_cnt, _, _) =
                                redact_body_bytes(payload, "application/json", opts);
                            if sub_cnt > 0 {
                                total_count += sub_cnt;
                                changed = true;
                            }
                            sub_out
                        } else {
                            let mut val_str = text.to_string();
                            let sub_cnt = redact_string_value(&mut val_str, opts, 0);
                            if sub_cnt > 0 {
                                total_count += sub_cnt;
                                changed = true;
                                val_str.into_bytes()
                            } else {
                                payload.to_vec()
                            }
                        }
                    }
                    Err(_) => {
                        if opts.strict {
                            total_count += 1;
                            any_strict_masked = true;
                            changed = true;
                            STRICT_REDACTED_BODY.to_vec()
                        } else {
                            any_unsanitizable = true;
                            payload.to_vec()
                        }
                    }
                }
            } else {
                let (sub_out, sub_cnt, sub_unsan, sub_strict) =
                    redact_body_bytes(payload, ct_str, opts);
                if sub_cnt > 0 || sub_strict {
                    changed = true;
                }
                total_count += sub_cnt;
                any_unsanitizable |= sub_unsan;
                any_strict_masked |= sub_strict;
                sub_out
            }
        };

        out.extend_from_slice(&body[pos..part_start]);
        out.extend_from_slice(raw_headers);
        out.extend_from_slice(sep);
        out.extend_from_slice(&new_payload);
        out.extend_from_slice(pre_next_nl);
    }

    if !saw_closing {
        return None;
    }
    if !changed {
        return Some((body.to_vec(), 0, any_unsanitizable, any_strict_masked));
    }
    Some((out, total_count, any_unsanitizable, any_strict_masked))
}

fn finish_unsanitizable_body(body: &[u8], opts: &RedactOptions) -> (Vec<u8>, usize, bool, bool) {
    if opts.strict {
        (STRICT_REDACTED_BODY.to_vec(), 1, false, true)
    } else {
        (body.to_vec(), 0, true, false)
    }
}

fn redact_body_bytes(
    body: &[u8],
    content_type: &str,
    opts: &RedactOptions,
) -> (Vec<u8>, usize, bool, bool) {
    if body.is_empty() || body == STRICT_REDACTED_BODY {
        return (body.to_vec(), 0, false, false);
    }
    let ct_lower = content_type.to_ascii_lowercase();
    if ct_lower.contains("multipart/form-data") {
        if let Some(boundary) = extract_multipart_boundary_param(content_type)
            && let Some(res) = redact_multipart_body(body, &boundary, opts)
        {
            return res;
        }
        return finish_unsanitizable_body(body, opts);
    }
    let Ok(text) = std::str::from_utf8(body) else {
        return finish_unsanitizable_body(body, opts);
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return (body.to_vec(), 0, false, false);
    }
    if ct_lower.contains("event-stream") {
        let (redacted_sse, count) = redact_event_stream(text, opts);
        if count > 0 {
            return (redacted_sse.into_bytes(), count, false, false);
        }
        return (body.to_vec(), 0, false, false);
    }
    let looks_json = ct_lower.contains("json")
        || (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'));
    if looks_json {
        if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let count = redact_json_value(&mut val, opts);
            if count > 0
                && let Ok(out) = serde_json::to_vec(&val)
            {
                return (out, count, false, false);
            }
            return (body.to_vec(), 0, false, false);
        }
        if ct_lower.contains("json") {
            return finish_unsanitizable_body(body, opts);
        }
    }
    if ct_lower.contains("application/x-www-form-urlencoded") {
        let (redacted_form, count) = redact_urlencoded_pairs(text, opts);
        if count > 0 {
            return (redacted_form.into_bytes(), count, false, false);
        }
        return (body.to_vec(), 0, false, false);
    }
    finish_unsanitizable_body(body, opts)
}

pub(crate) fn redact_json_value(val: &mut serde_json::Value, opts: &RedactOptions) -> usize {
    redact_json_value_depth(val, opts, 0)
}

fn redact_json_value_depth(
    val: &mut serde_json::Value,
    opts: &RedactOptions,
    depth: usize,
) -> usize {
    if depth > 16 {
        return 0;
    }
    match val {
        serde_json::Value::Object(map) => {
            let mut count = 0;
            for (k, v) in map.iter_mut() {
                if opts.is_sensitive_field(k) && !v.is_null() {
                    if v.as_str() != Some("[REDACTED]") {
                        *v = serde_json::Value::String("[REDACTED]".into());
                        count += 1;
                    }
                } else {
                    count += redact_json_value_depth(v, opts, depth);
                }
            }
            count
        }
        serde_json::Value::Array(arr) => arr
            .iter_mut()
            .map(|item| redact_json_value_depth(item, opts, depth))
            .sum(),
        serde_json::Value::String(s) => redact_string_value(s, opts, depth),
        _ => 0,
    }
}

pub(crate) fn redact_string_value(s: &mut String, opts: &RedactOptions, depth: usize) -> usize {
    if s == "[REDACTED]" {
        return 0;
    }
    let trimmed = s.trim();
    // 1. Unpack stringified JSON if it starts with { or [
    if ((trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']')))
        && let Ok(mut nested) = serde_json::from_str::<serde_json::Value>(trimmed)
    {
        let count = redact_json_value_depth(&mut nested, opts, depth + 1);
        if count > 0
            && let Ok(serialized) = serde_json::to_string(&nested)
        {
            *s = serialized;
            return count;
        }
        return 0;
    }

    // 2. Redact URL query parameters if it contains '?'
    if s.contains('?') {
        let (new_s, count) = redact_url_query(s, opts);
        if count > 0 {
            *s = new_s;
        }
        return count;
    }

    // 2b. Redact percent-encoded URL query parameters if it contains %3F or %3f
    if s.contains("%3F") || s.contains("%3f") {
        let decoded = percent_decode_key(s);
        if decoded.contains('?') {
            let (new_decoded, count) = redact_url_query(&decoded, opts);
            if count > 0 {
                *s = percent_encode_component(&new_decoded);
            }
            return count;
        }
    }

    // 3. Redact standalone urlencoded query string (e.g. "token=xyz&secret=123" or single pair "password=123")
    if s.contains('=')
        && (s.contains('&')
            || s.split_once('=')
                .is_some_and(|(k, _)| opts.is_sensitive_field(&percent_decode_key(k))))
    {
        let (new_s, count) = redact_urlencoded_pairs(s, opts);
        if count > 0 {
            *s = new_s;
            return count;
        }
    }

    0
}

pub(crate) fn clip_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

pub fn format_redact_preview(
    flow: &Flow,
    opts: &RedactOptions,
    zh: bool,
) -> (String, String, RedactReport) {
    let mut enabled_opts = opts.clone();
    enabled_opts.enabled = true;
    let (sanitized, report) = redact_flow(flow, &enabled_opts);
    (
        format_flow_snapshot(flow, zh),
        format_flow_snapshot(&sanitized, zh),
        report,
    )
}

fn format_flow_snapshot(flow: &Flow, zh: bool) -> String {
    let mut lines = Vec::new();
    lines.push(format!("{} {}", flow.method, flow.url));
    if !flow.comment.is_empty() {
        let label = if zh { "备注" } else { "Note" };
        lines.push(format!("# {label}: {}", flow.comment));
    }
    lines.push(String::new());
    lines.push(if zh {
        "--- 请求头 ---".into()
    } else {
        "--- Request Headers ---".into()
    });
    for (k, v) in &flow.request_headers {
        lines.push(format!("{k}: {v}"));
    }
    if !flow.request_body.is_empty() {
        lines.push(String::new());
        lines.push(if zh {
            "--- 请求 Body ---".into()
        } else {
            "--- Request Body ---".into()
        });
        let req_ct = crate::present::content_type(&flow.request_headers);
        if let Ok(text) = std::str::from_utf8(&flow.request_body) {
            lines.push(clip_chars(
                &crate::present::pretty_body(req_ct, text),
                4_000,
            ));
        } else {
            lines.push(format!("[binary {} bytes]", flow.request_body.len()));
        }
    }
    if !flow.response_headers.is_empty() || flow.status.is_some() {
        lines.push(String::new());
        lines.push(if zh {
            format!("--- 响应头 ({}) ---", flow.status.unwrap_or(0))
        } else {
            format!("--- Response Headers ({}) ---", flow.status.unwrap_or(0))
        });
        for (k, v) in &flow.response_headers {
            lines.push(format!("{k}: {v}"));
        }
    }
    if !flow.response_body.is_empty() {
        lines.push(String::new());
        lines.push(if zh {
            "--- 响应 Body ---".into()
        } else {
            "--- Response Body ---".into()
        });
        let res_ct = crate::present::content_type(&flow.response_headers);
        if let Ok(text) = std::str::from_utf8(&flow.response_body) {
            lines.push(clip_chars(
                &crate::present::pretty_body(res_ct, text),
                4_000,
            ));
        } else {
            lines.push(format!("[binary {} bytes]", flow.response_body.len()));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Direction;

    #[test]
    fn redact_flow_sanitizes_headers_query_json_form_and_flags_binary() {
        let mut session = crate::session::Session::default();
        let id = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "POST".into(),
            host: "api.example.test".into(),
            path: "/v1/login?access_token=tok123&page=2&custom_sec=abc".into(),
            url: "https://api.example.test/v1/login?access_token=tok123&page=2&custom_sec=abc"
                .into(),
            request_headers: vec![
                ("Authorization".into(), "Bearer eyJhbGciOiJIUzI1NiJ9".into()),
                ("Cookie".into(), "sid=s1; theme=dark".into()),
                ("X-Custom-Header".into(), "my-secret-header".into()),
                ("Content-Type".into(), "application/json".into()),
            ],
        });
        session.append_body(
            id,
            Direction::Request,
            br#"{"username":"alice","password":"p@ssw0rd","nested":{"custom_sec":"xyz","ok":1}}"#,
        );
        session.set_response(
            id,
            200,
            vec![
                (
                    "Set-Cookie".into(),
                    "session_id=abc999; Path=/; HttpOnly; Secure".into(),
                ),
                ("Content-Type".into(), "application/octet-stream".into()),
            ],
            None,
        );
        session.append_body(id, Direction::Response, &[0xff, 0x00, 0xfe, 0x01]);
        session.mark_finished(id);

        let original = session.hydrated(id).unwrap();
        let opts = RedactOptions::new(true, "custom_sec, X-Custom-Header");
        let (sanitized, report) = redact_flow(&original, &opts);

        // Original capture in session is untouched
        let untouched = session.hydrated(id).unwrap();
        assert!(untouched.url.contains("tok123"));

        // Query parameters
        assert_eq!(
            sanitized.url,
            "https://api.example.test/v1/login?access_token=[REDACTED]&page=2&custom_sec=[REDACTED]"
        );
        assert_eq!(
            sanitized.path,
            "/v1/login?access_token=[REDACTED]&page=2&custom_sec=[REDACTED]"
        );
        assert_eq!(report.redacted_query_params, 2);

        // Headers
        assert_eq!(sanitized.request_headers[0].1, "Bearer [REDACTED]");
        assert_eq!(
            sanitized.request_headers[1].1,
            "sid=[REDACTED]; theme=[REDACTED]"
        );
        assert_eq!(sanitized.request_headers[2].1, "[REDACTED]");
        assert_eq!(
            sanitized.response_headers[0].1,
            "session_id=[REDACTED]; Path=/; HttpOnly; Secure"
        );
        assert_eq!(report.redacted_headers, 4);

        // Request JSON body
        let body_json: serde_json::Value = serde_json::from_slice(&sanitized.request_body).unwrap();
        assert_eq!(body_json["username"], "alice");
        assert_eq!(body_json["password"], "[REDACTED]");
        assert_eq!(body_json["nested"]["custom_sec"], "[REDACTED]");
        assert_eq!(body_json["nested"]["ok"], 1);
        assert_eq!(report.redacted_body_fields, 2);

        // Binary response body flagged as unsanitizable
        assert!(!report.unsanitizable_req_body);
        assert!(report.unsanitizable_res_body);
        assert!(
            sanitized
                .comment
                .contains("[REDACTED; res body format not sanitizable]"),
            "comment={}",
            sanitized.comment
        );
    }

    #[test]
    fn redact_json_cookie_casing_and_header_keys() {
        let opts = RedactOptions::new(true, "");
        let mut val: serde_json::Value = serde_json::from_str(
            r#"{
                "cookie": "sid=123",
                "Cookie": "sid=456",
                "COOKIE": "sid=789",
                "set-cookie": "session=abc; Secure",
                "Set-Cookie": "session=def; HttpOnly",
                "set_cookie": "session=ghi",
                "setCookie": "session=jkl",
                "cookies": ["a", "b"],
                "accessToken": "secret_tok",
                "access-token": "secret_tok_2",
                "clientSecret": "client_sec",
                "x-api-key": "my_api_key",
                "csrfToken": "csrf_val",
                "username": "john_doe"
            }"#,
        )
        .unwrap();

        let count = redact_json_value(&mut val, &opts);
        assert_eq!(count, 13);
        assert_eq!(val["cookie"], "[REDACTED]");
        assert_eq!(val["Cookie"], "[REDACTED]");
        assert_eq!(val["COOKIE"], "[REDACTED]");
        assert_eq!(val["set-cookie"], "[REDACTED]");
        assert_eq!(val["Set-Cookie"], "[REDACTED]");
        assert_eq!(val["set_cookie"], "[REDACTED]");
        assert_eq!(val["setCookie"], "[REDACTED]");
        assert_eq!(val["cookies"], "[REDACTED]");
        assert_eq!(val["accessToken"], "[REDACTED]");
        assert_eq!(val["access-token"], "[REDACTED]");
        assert_eq!(val["clientSecret"], "[REDACTED]");
        assert_eq!(val["x-api-key"], "[REDACTED]");
        assert_eq!(val["csrfToken"], "[REDACTED]");
        assert_eq!(val["username"], "john_doe");
    }

    #[test]
    fn redact_json_url_query_secrets_and_nested_urls() {
        let opts = RedactOptions::new(true, "");
        let mut val: serde_json::Value = serde_json::from_str(
            r#"{
                "url": "https://api.example.com/oauth?token=xyz123&secret=abc456&client_id=my_app",
                "path": "/v1/auth?access_token=secret_tok&user=alice",
                "links": [
                    "https://cdn.example.com/file?api_key=secretkey&download=1"
                ],
                "oauth_redirect": "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Ftoken%3Dsecret_nested"
            }"#,
        )
        .unwrap();

        let count = redact_json_value(&mut val, &opts);
        assert_eq!(count, 5);
        assert_eq!(
            val["url"],
            "https://api.example.com/oauth?token=[REDACTED]&secret=[REDACTED]&client_id=my_app"
        );
        assert_eq!(val["path"], "/v1/auth?access_token=[REDACTED]&user=alice");
        assert_eq!(
            val["links"][0],
            "https://cdn.example.com/file?api_key=[REDACTED]&download=1"
        );
        assert_eq!(
            val["oauth_redirect"],
            "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Ftoken%3D%5BREDACTED%5D"
        );
    }

    #[test]
    fn redact_percent_encoded_nested_redirect_urls() {
        let opts = RedactOptions::new(true, "");
        let mut val: serde_json::Value = serde_json::from_str(
            r#"{
                "standard": "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Fsecret%3Dnested_sec%26ok%3D1",
                "lowercase_hex": "https://auth.com/login?redirect=https%3a%2f%2fapp.com%2fcb%3fsecret%3dnested_sec%26ok%3d1",
                "multi_secrets": "https://auth.com/login?client_id=123&redirect=https%3A%2F%2Fapp.com%2Fcb%3Ftoken%3Dtok_nested%26password%3Dpass_nested%26user%3Dalice&state=xyz",
                "with_fragment": "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Fapi_key%3Dmykey%26step%3D2%23details#outer_frag",
                "standalone_params": "https://api.com/proxy?forward=secret%3Dtopsecret%26client%3Dweb",
                "relative_redirect": "https://app.com/login?next=%2Fdashboard%3Fapi_key%3Dmy_api_key%26tab%3D1"
            }"#,
        )
        .unwrap();

        let count = redact_json_value(&mut val, &opts);
        assert_eq!(count, 7);
        assert_eq!(
            val["standard"],
            "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Fsecret%3D%5BREDACTED%5D%26ok%3D1"
        );
        assert_eq!(
            val["lowercase_hex"],
            "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Fsecret%3D%5BREDACTED%5D%26ok%3D1"
        );
        assert_eq!(
            val["multi_secrets"],
            "https://auth.com/login?client_id=123&redirect=https%3A%2F%2Fapp.com%2Fcb%3Ftoken%3D%5BREDACTED%5D%26password%3D%5BREDACTED%5D%26user%3Dalice&state=xyz"
        );
        assert_eq!(
            val["with_fragment"],
            "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Fapi_key%3D%5BREDACTED%5D%26step%3D2%23details#outer_frag"
        );
        assert_eq!(
            val["standalone_params"],
            "https://api.com/proxy?forward=secret%3D%5BREDACTED%5D%26client%3Dweb"
        );
        assert_eq!(
            val["relative_redirect"],
            "https://app.com/login?next=%2Fdashboard%3Fapi_key%3D%5BREDACTED%5D%26tab%3D1"
        );

        // Pass 2 Idempotency verification
        let count2 = redact_json_value(&mut val, &opts);
        assert_eq!(count2, 0);
        assert_eq!(
            val["standard"],
            "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Fsecret%3D%5BREDACTED%5D%26ok%3D1"
        );
        assert_eq!(
            val["with_fragment"],
            "https://auth.com/login?redirect=https%3A%2F%2Fapp.com%2Fcb%3Fapi_key%3D%5BREDACTED%5D%26step%3D2%23details#outer_frag"
        );
    }

    #[test]
    fn redact_json_stringified_nested_json() {
        let opts = RedactOptions::new(true, "");
        let mut val: serde_json::Value = serde_json::from_str(
            r#"{
                "event": "login",
                "body": "{\"token\":\"abc12345\",\"password\":\"hunter2\",\"user\":\"alice\"}",
                "nested": "{\"redirect\":\"https://service.internal/oauth?secret=nested_sec&ok=true\"}"
            }"#,
        )
        .unwrap();

        let count = redact_json_value(&mut val, &opts);
        assert_eq!(count, 3);

        // Unpack inner body and verify
        let inner_body: serde_json::Value =
            serde_json::from_str(val["body"].as_str().unwrap()).unwrap();
        assert_eq!(inner_body["token"], "[REDACTED]");
        assert_eq!(inner_body["password"], "[REDACTED]");
        assert_eq!(inner_body["user"], "alice");

        // Unpack inner nested url and verify
        let inner_nested: serde_json::Value =
            serde_json::from_str(val["nested"].as_str().unwrap()).unwrap();
        assert_eq!(
            inner_nested["redirect"],
            "https://service.internal/oauth?secret=[REDACTED]&ok=true"
        );
    }

    #[test]
    fn redact_json_malformed_strings_and_robustness() {
        let opts = RedactOptions::new(true, "");
        let mut val: serde_json::Value = serde_json::from_str(
            r#"{
                "broken_json": "{\"token\": incomplete",
                "not_url": "How are you? I'm fine.",
                "broken_url": "https://example.com/??==&token=xyz",
                "empty": ""
            }"#,
        )
        .unwrap();

        // Must not panic
        let count = redact_json_value(&mut val, &opts);
        assert_eq!(val["broken_json"], "{\"token\": incomplete");
        assert_eq!(val["not_url"], "How are you? I'm fine.");
        assert_eq!(val["empty"], "");
        assert!(
            val["broken_url"]
                .as_str()
                .unwrap()
                .contains("token=[REDACTED]")
        );
        assert_eq!(count, 1);
    }

    #[test]
    fn redact_flow_idempotent() {
        let mut session = crate::session::Session::default();
        let id = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "POST".into(),
            host: "api.example.test".into(),
            path: "/api?token=secret".into(),
            url: "https://api.example.test/api?token=secret".into(),
            request_headers: vec![("Cookie".into(), "sid=secret123".into())],
        });
        session.append_body(id, Direction::Request, br#"{"password":"secret"}"#);
        session.set_response(id, 200, vec![], None);
        session.mark_finished(id);

        let flow = session.hydrated(id).unwrap();
        let opts = RedactOptions::new(true, "");
        let (sanitized1, report1) = redact_flow(&flow, &opts);
        assert_eq!(report1.total_redacted(), 3);

        // Second pass on already-redacted flow
        let (sanitized2, report2) = redact_flow(&sanitized1, &opts);
        assert_eq!(sanitized1.url, sanitized2.url);
        assert_eq!(sanitized1.request_body, sanitized2.request_body);
        assert_eq!(report2.total_redacted(), 0);
    }

    #[test]
    fn redact_flow_and_json_idempotency_with_fragments_multi_pass() {
        let mut session = crate::session::Session::default();
        let target_url = "https://api.example.com:8443/auth/v2?user=alice&token=tok123&env=prod&cOoKiE=sess456#tab1?extra=ignored";
        let target_path =
            "/auth/v2?user=alice&token=tok123&env=prod&cOoKiE=sess456#tab1?extra=ignored";

        let id = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "POST".into(),
            host: "api.example.com".into(),
            path: target_path.into(),
            url: target_url.into(),
            request_headers: vec![
                ("Cookie".into(), "sid=secret123; user=alice".into()),
                ("Authorization".into(), "Bearer my_jwt_token".into()),
            ],
        });
        session.append_body(
            id,
            Direction::Request,
            br#"{
                "callback": "https://api.example.com:8443/auth/v2?user=alice&token=tok123&env=prod&cOoKiE=sess456#tab1?extra=ignored",
                "nested_endpoint": "https://service.internal/path?api_key=secretkey&page=1#heading?section=sec2",
                "plain_query": "token=tok999&user=bob"
            }"#,
        );
        session.set_response(id, 200, vec![], None);
        session.mark_finished(id);

        let flow = session.hydrated(id).unwrap();
        let opts = RedactOptions::new(true, "");

        // --- Pass 1: First redaction pass ---
        let (pass1_flow, rep1) = redact_flow(&flow, &opts);
        // Query params in URL & path (token + cOoKiE = 2 each; report counts URL query params = 2)
        assert_eq!(rep1.redacted_query_params, 2);
        // Headers: Cookie sid + Authorization = 2
        assert_eq!(rep1.redacted_headers, 2);
        // Body fields: callback (token + cOoKiE = 2), nested_endpoint (api_key = 1), plain_query (token = 1) -> 4
        assert_eq!(rep1.redacted_body_fields, 4);
        assert_eq!(rep1.total_redacted(), 8);

        // Verify URL & Path: sensitive parameters redacted, fragments strictly preserved
        assert_eq!(
            pass1_flow.url,
            "https://api.example.com:8443/auth/v2?user=alice&token=[REDACTED]&env=prod&cOoKiE=[REDACTED]#tab1?extra=ignored"
        );
        assert_eq!(
            pass1_flow.path,
            "/auth/v2?user=alice&token=[REDACTED]&env=prod&cOoKiE=[REDACTED]#tab1?extra=ignored"
        );

        // Verify JSON body
        let parsed1: serde_json::Value = serde_json::from_slice(&pass1_flow.request_body).unwrap();
        assert_eq!(
            parsed1["callback"],
            "https://api.example.com:8443/auth/v2?user=alice&token=[REDACTED]&env=prod&cOoKiE=[REDACTED]#tab1?extra=ignored"
        );
        assert_eq!(
            parsed1["nested_endpoint"],
            "https://service.internal/path?api_key=[REDACTED]&page=1#heading?section=sec2"
        );
        assert_eq!(parsed1["plain_query"], "token=[REDACTED]&user=bob");

        // --- Pass 2: Redacting already-redacted flow ---
        let (pass2_flow, rep2) = redact_flow(&pass1_flow, &opts);
        assert_eq!(rep2.total_redacted(), 0, "Pass 2 must produce 0 redactions");
        assert_eq!(rep2.redacted_query_params, 0);
        assert_eq!(rep2.redacted_headers, 0);
        assert_eq!(rep2.redacted_body_fields, 0);

        // Exact byte-for-byte and string identity between Pass 1 and Pass 2
        assert_eq!(pass2_flow.url, pass1_flow.url);
        assert_eq!(pass2_flow.path, pass1_flow.path);
        assert_eq!(pass2_flow.request_headers, pass1_flow.request_headers);
        assert_eq!(pass2_flow.request_body, pass1_flow.request_body);

        let parsed2: serde_json::Value = serde_json::from_slice(&pass2_flow.request_body).unwrap();
        assert_eq!(parsed2["callback"], parsed1["callback"]);
        assert!(
            parsed2["callback"]
                .as_str()
                .unwrap()
                .ends_with("#tab1?extra=ignored"),
            "Fragment was destroyed on Pass 2!"
        );

        // --- Pass 3: Verification of stability across 3 passes ---
        let (pass3_flow, rep3) = redact_flow(&pass2_flow, &opts);
        assert_eq!(rep3.total_redacted(), 0, "Pass 3 must produce 0 redactions");
        assert_eq!(pass3_flow.url, pass2_flow.url);
        assert_eq!(pass3_flow.path, pass2_flow.path);
        assert_eq!(pass3_flow.request_headers, pass2_flow.request_headers);
        assert_eq!(pass3_flow.request_body, pass2_flow.request_body);
    }

    #[test]
    fn redact_plain_unencoded_nested_urls_and_mixed_params() {
        let opts = RedactOptions::new(true, "");
        let mut val: serde_json::Value = serde_json::from_str(
            r#"{
                "single_nested": "https://auth.com/login?redirect=https://app.com/cb?token=secret",
                "mixed_outer_inner": "https://auth.com/login?app_id=42&redirect=https://app.com/cb?token=secret&env=staging",
                "nested_with_frag": "https://auth.com/login?redirect=https://app.com/cb?token=secret#section1?extra=ignored"
            }"#,
        )
        .unwrap();

        let count = redact_json_value(&mut val, &opts);
        assert_eq!(count, 3);

        // Verification of single nested URL
        let single = val["single_nested"].as_str().unwrap().to_string();
        assert!(!single.contains("secret"));
        assert_eq!(
            single,
            "https://auth.com/login?redirect=https://app.com/cb?token=[REDACTED]"
        );

        // Verification of mixed outer and inner query parameters
        let mixed = val["mixed_outer_inner"].as_str().unwrap().to_string();
        assert!(!mixed.contains("secret"));
        assert!(mixed.contains("app_id=42"));
        assert!(mixed.contains("env=staging"));
        assert_eq!(
            mixed,
            "https://auth.com/login?app_id=42&redirect=https://app.com/cb?token=[REDACTED]&env=staging"
        );

        // Verification of nested URL with fragment
        let with_frag = val["nested_with_frag"].as_str().unwrap().to_string();
        assert!(!with_frag.contains("secret"));
        assert_eq!(
            with_frag,
            "https://auth.com/login?redirect=https://app.com/cb?token=[REDACTED]#section1?extra=ignored"
        );

        // Idempotency on Pass 2 and Pass 3
        let count2 = redact_json_value(&mut val, &opts);
        assert_eq!(count2, 0);
        assert_eq!(val["single_nested"], single);
        assert_eq!(val["mixed_outer_inner"], mixed);
        assert_eq!(val["nested_with_frag"], with_frag);

        let count3 = redact_json_value(&mut val, &opts);
        assert_eq!(count3, 0);
    }

    #[test]
    fn redact_query_param_permutations_and_edge_cases() {
        let opts = RedactOptions::new(true, "");

        // 1. Sensitive at start, middle, end
        let (out1, c1) = redact_url_query("https://example.com/api?token=sec1&keep=1", &opts);
        assert_eq!(c1, 1);
        assert_eq!(out1, "https://example.com/api?token=[REDACTED]&keep=1");

        let (out2, c2) = redact_url_query("https://example.com/api?keep=1&token=sec2", &opts);
        assert_eq!(c2, 1);
        assert_eq!(out2, "https://example.com/api?keep=1&token=[REDACTED]");

        let (out3, c3) = redact_url_query("https://example.com/api?a=1&password=sec3&b=2", &opts);
        assert_eq!(c3, 1);
        assert_eq!(out3, "https://example.com/api?a=1&password=[REDACTED]&b=2");

        // 2. Contiguous sensitive parameters
        let (out4, c4) = redact_url_query(
            "https://example.com/api?access_token=t1&refresh_token=t2&secret=s3",
            &opts,
        );
        assert_eq!(c4, 3);
        assert_eq!(
            out4,
            "https://example.com/api?access_token=[REDACTED]&refresh_token=[REDACTED]&secret=[REDACTED]"
        );

        // 3. Casing and delimiter variants in query string
        let (out5, c5) = redact_url_query(
            "https://example.com/api?X-Api-Key=k1&cOoKiE=c1&clientSecret=s1&session-id=sid1",
            &opts,
        );
        assert_eq!(c5, 4);
        assert_eq!(
            out5,
            "https://example.com/api?X-Api-Key=[REDACTED]&cOoKiE=[REDACTED]&clientSecret=[REDACTED]&session-id=[REDACTED]"
        );

        // 4. Empty value in sensitive parameter
        let (out6, c6) = redact_url_query("https://example.com/api?token=&user=alice", &opts);
        assert_eq!(c6, 1);
        assert_eq!(out6, "https://example.com/api?token=[REDACTED]&user=alice");

        // 5. Flag parameters without '='
        let (out7, c7) =
            redact_url_query("https://example.com/api?flag1&token=secret&flag2", &opts);
        assert_eq!(c7, 1);
        assert_eq!(out7, "https://example.com/api?flag1&token=[REDACTED]&flag2");

        // 6. Duplicate keys
        let (out8, c8) =
            redact_url_query("https://example.com/api?token=s1&user=bob&token=s2", &opts);
        assert_eq!(c8, 2);
        assert_eq!(
            out8,
            "https://example.com/api?token=[REDACTED]&user=bob&token=[REDACTED]"
        );

        // 7. Percent-encoded sensitive key (%74%6f%6b%65%6e == "token")
        let (out9, c9) = redact_url_query(
            "https://example.com/api?%74%6f%6b%65%6e=secret&other=val",
            &opts,
        );
        assert_eq!(c9, 1);
        assert_eq!(
            out9,
            "https://example.com/api?%74%6f%6b%65%6e=[REDACTED]&other=val"
        );

        // Idempotency: redacting output again produces 0 count and exact same string
        for out in [
            &out1, &out2, &out3, &out4, &out5, &out6, &out7, &out8, &out9,
        ] {
            let (pass2, count2) = redact_url_query(out, &opts);
            assert_eq!(count2, 0);
            assert_eq!(&pass2, out);
        }
    }

    #[test]
    fn redact_form_urlencoded_request_body() {
        let mut session = crate::session::Session::default();
        let id = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "POST".into(),
            host: "auth.example.com".into(),
            path: "/oauth/token".into(),
            url: "https://auth.example.com/oauth/token".into(),
            request_headers: vec![(
                "Content-Type".into(),
                "application/x-www-form-urlencoded; charset=utf-8".into(),
            )],
        });
        session.append_body(
            id,
            Direction::Request,
            b"grant_type=password&username=alice&password=my_secret_pw&client_secret=sec_val&refresh_token=rt_999&scope=read",
        );
        session.set_response(id, 200, vec![], None);
        session.mark_finished(id);

        let flow = session.hydrated(id).unwrap();
        let opts = RedactOptions::new(true, "");

        let (sanitized1, rep1) = redact_flow(&flow, &opts);
        assert_eq!(rep1.redacted_body_fields, 3);
        assert!(!rep1.unsanitizable_req_body);

        let body_str = std::str::from_utf8(&sanitized1.request_body).unwrap();
        assert!(!body_str.contains("my_secret_pw"));
        assert!(!body_str.contains("sec_val"));
        assert!(!body_str.contains("rt_999"));
        assert_eq!(
            body_str,
            "grant_type=password&username=alice&password=[REDACTED]&client_secret=[REDACTED]&refresh_token=[REDACTED]&scope=read"
        );

        // Pass 2 idempotency
        let (sanitized2, rep2) = redact_flow(&sanitized1, &opts);
        assert_eq!(rep2.total_redacted(), 0);
        assert_eq!(sanitized2.request_body, sanitized1.request_body);
    }

    #[test]
    fn redact_options_parses_at_strict_token_and_with_strict() {
        let opts = RedactOptions::new(true, "phone, @strict, X-Custom-Sec");
        assert!(opts.enabled);
        assert!(opts.strict);
        assert_eq!(opts.extra_fields, vec!["phone", "x-custom-sec"]);
        assert!(!opts.is_sensitive_field("@strict"));
        assert!(opts.is_sensitive_field("phone"));
        assert!(opts.is_sensitive_header("x-custom-sec"));

        let roundtrip = RedactOptions::new(true, &opts.to_wire_fields());
        assert_eq!(roundtrip, opts);

        let via_builder = RedactOptions::new(true, "phone").with_strict(true);
        assert!(via_builder.strict);
        assert_eq!(via_builder.extra_fields, vec!["phone"]);
    }

    #[test]
    fn redact_multipart_form_data_fields_filenames_and_nested_parts() {
        let boundary = "----ZniffBoundary7MA4YWxkTrZu0gW";
        let multipart_body = format!(
            "--{boundary}\r\n\
Content-Disposition: form-data; name=\"username\"\r\n\r\n\
alice\r\n\
--{boundary}\r\n\
Content-Disposition: form-data; name=\"password\"\r\n\r\n\
hunter2\r\n\
--{boundary}\r\n\
Content-Disposition: form-data; name=\"upload\"; filename=\"client_secret.pem\"\r\n\
Content-Type: application/x-pem-file\r\n\r\n\
PRIVATE_KEY_MATERIAL\r\n\
--{boundary}\r\n\
Content-Disposition: form-data; name=\"metadata\"\r\n\
Content-Type: application/json\r\n\r\n\
{{\"access_token\":\"tok_999\",\"role\":\"admin\"}}\r\n\
--{boundary}\r\n\
Content-Disposition: form-data; name=\"params\"\r\n\
Content-Type: application/x-www-form-urlencoded\r\n\r\n\
api_key=sec_key_1&lang=zh\r\n\
--{boundary}--\r\n"
        );

        let mut session = crate::session::Session::default();
        let id = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "POST".into(),
            host: "upload.example.com".into(),
            path: "/v1/submit".into(),
            url: "https://upload.example.com/v1/submit".into(),
            request_headers: vec![(
                "Content-Type".into(),
                format!("multipart/form-data; boundary=\"{boundary}\""),
            )],
        });
        session.append_body(id, Direction::Request, multipart_body.as_bytes());
        session.set_response(id, 200, vec![], None);
        session.mark_finished(id);

        let flow = session.hydrated(id).unwrap();
        let opts = RedactOptions::new(true, "");
        let (sanitized, rep) = redact_flow(&flow, &opts);

        // password + client_secret.pem + metadata.access_token + params.api_key = 4
        assert_eq!(rep.redacted_body_fields, 4);
        assert!(!rep.unsanitizable_req_body);
        assert!(!rep.strict_masked_req_body);

        let text = std::str::from_utf8(&sanitized.request_body).unwrap();
        assert!(text.contains("alice"), "{text}");
        assert!(text.contains("\"role\":\"admin\""), "{text}");
        assert!(text.contains("lang=zh"), "{text}");
        assert!(!text.contains("hunter2"), "{text}");
        assert!(!text.contains("PRIVATE_KEY_MATERIAL"), "{text}");
        assert!(!text.contains("tok_999"), "{text}");
        assert!(!text.contains("sec_key_1"), "{text}");
        assert!(text.contains(&format!("--{boundary}--")), "{text}");

        // Valid multipart structure still parses through present::multipart_pairs
        let ct = crate::present::content_type(&sanitized.request_headers);
        let pairs = crate::present::multipart_pairs(ct, &sanitized.request_body).unwrap();
        assert!(pairs.iter().any(|(k, v)| k == "username" && v == "alice"));
        assert!(
            pairs
                .iter()
                .any(|(k, v)| k == "password" && v == "[REDACTED]")
        );

        // Pass 2 idempotency
        let (sanitized2, rep2) = redact_flow(&sanitized, &opts);
        assert_eq!(rep2.total_redacted(), 0);
        assert_eq!(sanitized2.request_body, sanitized.request_body);
    }

    #[test]
    fn redact_strict_masks_binary_and_unstructured_bodies_idempotently() {
        let mut session = crate::session::Session::default();
        let id = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http11,
            method: "POST".into(),
            host: "files.example.com".into(),
            path: "/raw".into(),
            url: "https://files.example.com/raw".into(),
            request_headers: vec![("Content-Type".into(), "text/plain".into())],
        });
        session.append_body(id, Direction::Request, b"unstructured secret prose");
        session.set_response(
            id,
            200,
            vec![("Content-Type".into(), "application/octet-stream".into())],
            None,
        );
        session.append_body(id, Direction::Response, &[0xff, 0x00, 0xfe, 0x01]);
        session.mark_finished(id);

        let flow = session.hydrated(id).unwrap();
        let strict_opts = RedactOptions::new(true, "@strict");
        let (sanitized, rep) = redact_flow(&flow, &strict_opts);

        assert_eq!(sanitized.request_body, STRICT_REDACTED_BODY);
        assert_eq!(sanitized.response_body, STRICT_REDACTED_BODY);
        assert_eq!(rep.redacted_body_fields, 2);
        assert!(!rep.unsanitizable_req_body);
        assert!(!rep.unsanitizable_res_body);
        assert!(rep.strict_masked_req_body);
        assert!(rep.strict_masked_res_body);
        assert!(rep.has_strict_masked_body());
        assert!(
            sanitized
                .comment
                .contains("[REDACTED; req/res body omitted in strict mode]"),
            "comment={}",
            sanitized.comment
        );

        // Pass 2 idempotency
        let (sanitized2, rep2) = redact_flow(&sanitized, &strict_opts);
        assert_eq!(rep2.total_redacted(), 0);
        assert_eq!(sanitized2.request_body, sanitized.request_body);
        assert_eq!(sanitized2.response_body, sanitized.response_body);
        assert_eq!(sanitized2.comment, sanitized.comment);
    }
}
