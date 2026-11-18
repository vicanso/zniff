//! Pure-Rust dynamic template variables and cryptographic signing helpers
//! (`{{uuid}}`, `{{ts_ms}}`, `{{hmac_sha256(...)}}`, `{{req.*}}`, etc.) for
//! runtime rule evaluation without embedding an external script VM.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::rules::{HeaderOp, JsonOp, QueryOp, Rule};

static SEQ_COUNTER: AtomicU64 = AtomicU64::new(1);

const K256: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

/// Request / response evaluation snapshot passed to rule variable expansion.
pub struct VarContext<'a> {
    pub method: &'a str,
    pub host: &'a str,
    pub target: &'a str,
    pub headers: &'a [(String, String)],
    pub body: &'a [u8],
    pub status: Option<u16>,
    pub now_ms: u64,
}

impl<'a> VarContext<'a> {
    /// Helper for building a consistent millisecond timestamp from the clock abstraction.
    pub fn current_ms() -> u64 {
        crate::clock::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// Pure-Rust SHA-256 (RFC 6234), safe across native and `wasm32`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity((data.len() + 9 + 63) & !63);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() + 8) % 64 != 0 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut w = [0_u32; 64];
    for chunk in padded.as_chunks::<64>().0 {
        for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K256[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (acc, val) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *acc = acc.wrapping_add(val);
        }
    }

    let mut out = [0_u8; 32];
    for (slot, val) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
        slot.copy_from_slice(&val.to_be_bytes());
    }
    out
}

/// Pure-Rust HMAC-SHA256 (RFC 2104), safe across native and `wasm32`.
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut key_block = [0_u8; 64];
    if key.len() > 64 {
        let hashed = sha256(key);
        key_block[..32].copy_from_slice(&hashed);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Vec::with_capacity(64 + msg.len());
    for byte in key_block {
        inner.push(byte ^ 0x36);
    }
    inner.extend_from_slice(msg);
    let inner_hash = sha256(&inner);

    let mut outer = Vec::with_capacity(64 + 32);
    for byte in key_block {
        outer.push(byte ^ 0x5c);
    }
    outer.extend_from_slice(&inner_hash);
    sha256(&outer)
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

fn percent_encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for &b in raw.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            use std::fmt::Write as _;
            let _ = write!(&mut out, "%{b:02X}");
        }
    }
    out
}

fn percent_decode(raw: &str) -> String {
    let mut bytes = Vec::with_capacity(raw.len());
    let src = raw.as_bytes();
    let mut i = 0;
    while i < src.len() {
        if src[i] == b'+' {
            bytes.push(b' ');
            i += 1;
        } else if src[i] == b'%'
            && i + 2 < src.len()
            && let (Some(hi), Some(lo)) = (hex_nibble(src[i + 1]), hex_nibble(src[i + 2]))
        {
            bytes.push((hi << 4) | lo);
            i += 3;
        } else {
            bytes.push(src[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn mix64(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn next_u128(now_ms: u64) -> u128 {
    let seq = SEQ_COUNTER.fetch_add(1, Ordering::Relaxed);
    let addr = (&seq as *const u64 as usize) as u64;
    let hi = mix64(now_ms ^ seq.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    let lo = mix64(addr ^ seq.wrapping_add(0x517c_c1b7_2722_0a95));
    ((hi as u128) << 64) | (lo as u128)
}

fn generate_uuid_v4(now_ms: u64) -> String {
    let mut bytes = next_u128(now_ms).to_be_bytes();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{}-{}-{}-{}-{}",
        hex_lower(&bytes[0..4]),
        hex_lower(&bytes[4..6]),
        hex_lower(&bytes[6..8]),
        hex_lower(&bytes[8..10]),
        hex_lower(&bytes[10..16]),
    )
}

fn format_iso_utc(ms: u64) -> String {
    let total_secs = (ms / 1000) as i64;
    let rem_ms = ms % 1000;
    let days = total_secs.div_euclid(86_400);
    let day_secs = total_secs.rem_euclid(86_400) as u32;
    let hour = day_secs / 3600;
    let minute = (day_secs % 3600) / 60;
    let second = day_secs % 60;
    // Howard Hinnant's civil-from-days algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}.{rem_ms:03}Z")
}

fn find_closing_mustache(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    let mut i = 0;
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if b == b'"' || b == b'\'' {
            quote = Some(b);
            i += 1;
            continue;
        }
        if b == b'}' && i + 1 < bytes.len() && bytes[i + 1] == b'}' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn lookup_query_param(target: &str, name: &str) -> Option<String> {
    let (_, query) = target.split_once('?')?;
    let query = query.split('#').next().unwrap_or(query);
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if percent_decode(k) == name {
            return Some(percent_decode(v));
        }
    }
    None
}

fn lookup_header(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

fn resolve_ident(ident: &str, ctx: &VarContext<'_>) -> Option<String> {
    let trimmed = ident.trim();
    match trimmed {
        "uuid" => Some(generate_uuid_v4(ctx.now_ms)),
        "ts" => Some((ctx.now_ms / 1000).to_string()),
        "ts_ms" => Some(ctx.now_ms.to_string()),
        "iso_now" => Some(format_iso_utc(ctx.now_ms)),
        "req.method" => Some(ctx.method.to_string()),
        "req.host" => Some(ctx.host.to_string()),
        "req.path" => Some(
            ctx.target
                .split('?')
                .next()
                .unwrap_or(ctx.target)
                .to_string(),
        ),
        "req.target" => Some(ctx.target.to_string()),
        "req.body" => Some(String::from_utf8_lossy(ctx.body).into_owned()),
        "res.status" => ctx.status.map(|code| code.to_string()),
        "res.body" => Some(String::from_utf8_lossy(ctx.body).into_owned()),
        _ if trimmed.starts_with("req.query.") => {
            let key = &trimmed["req.query.".len()..];
            Some(lookup_query_param(ctx.target, key).unwrap_or_default())
        }
        _ if trimmed.starts_with("req.header.") => {
            let key = &trimmed["req.header.".len()..];
            Some(lookup_header(ctx.headers, key).unwrap_or_default())
        }
        _ if trimmed.starts_with("res.header.") => {
            let key = &trimmed["res.header.".len()..];
            Some(lookup_header(ctx.headers, key).unwrap_or_default())
        }
        _ => None,
    }
}

fn unquote_literal(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let q = bytes[0];
    if (q != b'"' && q != b'\'') || bytes[bytes.len() - 1] != q {
        return None;
    }
    let inner = &raw[1..raw.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    Some(out)
}

fn split_top_level(input: &str, sep: u8) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = input.as_bytes();
    let mut start = 0;
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    for (idx, &b) in bytes.iter().enumerate() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == q {
                quote = None;
            }
            continue;
        }
        if b == b'"' || b == b'\'' {
            quote = Some(b);
            continue;
        }
        if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth = depth.saturating_sub(1);
        } else if b == sep && depth == 0 {
            out.push(&input[start..idx]);
            start = idx + 1;
        }
    }
    out.push(&input[start..]);
    out
}

fn eval_expr(expr: &str, ctx: &VarContext<'_>) -> Option<String> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return None;
    }
    let concat_parts = split_top_level(trimmed, b'+');
    if concat_parts.len() > 1 {
        let mut combined = String::new();
        for part in concat_parts {
            combined.push_str(&eval_atom(part, ctx)?);
        }
        return Some(combined);
    }
    eval_atom(trimmed, ctx)
}

fn eval_atom(raw: &str, ctx: &VarContext<'_>) -> Option<String> {
    let trimmed = raw.trim();
    if let Some(lit) = unquote_literal(trimmed) {
        return Some(lit);
    }
    if trimmed.bytes().all(|b| b.is_ascii_digit()) {
        return Some(trimmed.to_string());
    }
    if let Some(open) = trimmed.find('(')
        && trimmed.ends_with(')')
    {
        let fn_name = trimmed[..open].trim();
        let inner = &trimmed[open + 1..trimmed.len() - 1];
        let args = if inner.trim().is_empty() {
            Vec::new()
        } else {
            split_top_level(inner, b',')
        };
        return eval_function(fn_name, &args, ctx);
    }
    resolve_ident(trimmed, ctx)
}

fn eval_function(name: &str, args: &[&str], ctx: &VarContext<'_>) -> Option<String> {
    match name {
        "sha256" if args.len() == 1 => {
            let input = eval_expr(args[0], ctx)?;
            Some(hex_lower(&sha256(input.as_bytes())))
        }
        "sha256_b64" if args.len() == 1 => {
            let input = eval_expr(args[0], ctx)?;
            Some(crate::session::base64_encode(&sha256(input.as_bytes())))
        }
        "hmac_sha256" if args.len() == 2 => {
            let key = eval_expr(args[0], ctx)?;
            let msg = eval_expr(args[1], ctx)?;
            Some(hex_lower(&hmac_sha256(key.as_bytes(), msg.as_bytes())))
        }
        "hmac_sha256_b64" if args.len() == 2 => {
            let key = eval_expr(args[0], ctx)?;
            let msg = eval_expr(args[1], ctx)?;
            Some(crate::session::base64_encode(&hmac_sha256(
                key.as_bytes(),
                msg.as_bytes(),
            )))
        }
        "base64" if args.len() == 1 => {
            let input = eval_expr(args[0], ctx)?;
            Some(crate::session::base64_encode(input.as_bytes()))
        }
        "urlencode" if args.len() == 1 => {
            let input = eval_expr(args[0], ctx)?;
            Some(percent_encode(&input))
        }
        "random_int" if args.len() == 2 => {
            let min = eval_expr(args[0], ctx)?.trim().parse::<i64>().ok()?;
            let max = eval_expr(args[1], ctx)?.trim().parse::<i64>().ok()?;
            let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
            let span = (hi as i128 - lo as i128 + 1).max(1) as u128;
            let pick = (next_u128(ctx.now_ms) % span) as i128 + lo as i128;
            Some(pick.to_string())
        }
        _ => None,
    }
}

/// Expands known `{{...}}` dynamic expressions while preserving literal text
/// and unknown `{{mustache}}` tokens verbatim.
pub fn expand_str(input: &str, ctx: &VarContext<'_>) -> String {
    if !input.contains("{{") {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after_open = &rest[open + 2..];
        let Some(close) = find_closing_mustache(after_open) else {
            out.push_str(&rest[open..]);
            return out;
        };
        let expr = &after_open[..close];
        match eval_expr(expr, ctx) {
            Some(resolved) => out.push_str(&resolved),
            None => {
                out.push_str("{{");
                out.push_str(expr);
                out.push_str("}}");
            }
        }
        rest = &after_open[close + 2..];
    }
    out.push_str(rest);
    out
}

fn contains_req_body_var(text: &str) -> bool {
    text.contains("{{") && text.contains("req.body")
}

fn contains_req_context_var(text: &str) -> bool {
    text.contains("{{") && text.contains("req.")
}

fn json_value_uses_req_body(val: &serde_json::Value) -> bool {
    match val {
        serde_json::Value::String(s) => contains_req_body_var(s),
        serde_json::Value::Array(items) => items.iter().any(json_value_uses_req_body),
        serde_json::Value::Object(map) => map.values().any(json_value_uses_req_body),
        _ => false,
    }
}

/// Returns `true` when any host/path-matching request rule references `{{...req.body...}}`.
pub fn rules_need_request_body(rules: &[Rule], host: &str, path: &str) -> bool {
    rules.iter().any(|rule| {
        if !rule.matches_location(host, path) {
            return false;
        }
        match rule {
            Rule::Header {
                response: false,
                name,
                value,
                ..
            } => contains_req_body_var(name) || contains_req_body_var(value),
            Rule::Query { name, value, .. } => {
                contains_req_body_var(name) || contains_req_body_var(value)
            }
            Rule::Replace {
                response: false,
                from,
                to,
                ..
            } => contains_req_body_var(from) || contains_req_body_var(to),
            Rule::Json {
                response: false,
                json_path,
                value,
                ..
            } => contains_req_body_var(json_path) || json_value_uses_req_body(value),
            _ => false,
        }
    })
}

/// Returns `true` when any `HeaderOp::Set` references a `{{...req.*...}}` variable
/// that may change after query/path/body mutations.
pub fn header_ops_use_req_context(ops: &[HeaderOp]) -> bool {
    ops.iter().any(|op| match op {
        HeaderOp::Set { name, value } => {
            contains_req_context_var(name) || contains_req_context_var(value)
        }
        HeaderOp::Remove { .. } => false,
    })
}

/// Expands dynamic variables across header operations. If `only_req_context` is
/// `true`, non-`req.*` operations are omitted so previously minted timestamps/UUIDs
/// are not regenerated a second time during post-rewrite signing.
pub fn expand_header_ops(
    ops: &[HeaderOp],
    ctx: &VarContext<'_>,
    only_req_context: bool,
) -> Vec<HeaderOp> {
    ops.iter()
        .filter_map(|op| match op {
            HeaderOp::Set { name, value } => {
                if only_req_context
                    && !contains_req_context_var(name)
                    && !contains_req_context_var(value)
                {
                    return None;
                }
                Some(HeaderOp::Set {
                    name: expand_str(name, ctx),
                    value: expand_str(value, ctx),
                })
            }
            HeaderOp::Remove { name } => {
                if only_req_context && !contains_req_context_var(name) {
                    return None;
                }
                Some(HeaderOp::Remove {
                    name: expand_str(name, ctx),
                })
            }
        })
        .collect()
}

pub fn expand_query_ops(ops: &[QueryOp], ctx: &VarContext<'_>) -> Vec<QueryOp> {
    ops.iter()
        .map(|op| match op {
            QueryOp::Set { name, value } => QueryOp::Set {
                name: expand_str(name, ctx),
                value: expand_str(value, ctx),
            },
            QueryOp::Remove { name } => QueryOp::Remove {
                name: expand_str(name, ctx),
            },
        })
        .collect()
}

pub fn expand_replaces(pairs: &[(String, String)], ctx: &VarContext<'_>) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(from, to)| (expand_str(from, ctx), expand_str(to, ctx)))
        .collect()
}

fn expand_json_value(val: &serde_json::Value, ctx: &VarContext<'_>) -> serde_json::Value {
    match val {
        serde_json::Value::String(s) => serde_json::Value::String(expand_str(s, ctx)),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(|v| expand_json_value(v, ctx)).collect())
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| (expand_str(k, ctx), expand_json_value(v, ctx)))
                .collect(),
        ),
        other => other.clone(),
    }
}

pub fn expand_json_ops(ops: &[JsonOp], ctx: &VarContext<'_>) -> Vec<JsonOp> {
    ops.iter()
        .map(|op| match op {
            JsonOp::Set { path, value } => JsonOp::Set {
                path: expand_str(path, ctx),
                value: expand_json_value(value, ctx),
            },
            JsonOp::Remove { path } => JsonOp::Remove {
                path: expand_str(path, ctx),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_sha256_matches_rfc4231_test_vector_2() {
        // RFC 4231 Test Case 2: Key = "Jefe", Data = "what do ya want for nothing?"
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex_lower(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn expands_variables_concat_signing_and_preserves_unknown_mustaches() {
        let headers = vec![
            ("Content-Type".into(), "application/json".into()),
            ("X-Trace".into(), "trace-42".into()),
        ];
        let body = br#"{"amount":200,"host":"api.vicanso.dev"}"#;
        let ctx = VarContext {
            method: "POST",
            host: "api.vicanso.dev",
            target: "/v1/pay?currency=USD&note=hi%20there",
            headers: &headers,
            body,
            status: None,
            now_ms: 1_795_000_123_456,
        };

        assert_eq!(expand_str("{{ts}}", &ctx), "1795000123");
        assert_eq!(expand_str("{{ts_ms}}", &ctx), "1795000123456");
        assert_eq!(expand_str("{{req.method}}", &ctx), "POST");
        assert_eq!(expand_str("{{req.host}}", &ctx), "api.vicanso.dev");
        assert_eq!(expand_str("{{req.path}}", &ctx), "/v1/pay");
        assert_eq!(expand_str("{{req.query.note}}", &ctx), "hi there");
        assert_eq!(expand_str("{{req.header.x-trace}}", &ctx), "trace-42");

        let uuid_val = expand_str("{{uuid}}", &ctx);
        assert_eq!(uuid_val.len(), 36);
        assert_eq!(&uuid_val[14..15], "4");

        let canonical =
            "POST\n/v1/pay\n1795000123456\n{\"amount\":200,\"host\":\"api.vicanso.dev\"}";
        let expected_sig = hex_lower(&hmac_sha256(b"secret-key", canonical.as_bytes()));
        let actual_sig = expand_str(
            r#"sig={{hmac_sha256("secret-key", req.method + "\n" + req.path + "\n" + ts_ms + "\n" + req.body)}}"#,
            &ctx,
        );
        assert_eq!(actual_sig, format!("sig={expected_sig}"));

        // Unknown mustache token stays untouched.
        assert_eq!(
            expand_str("hello {{user.unknown_field}}!", &ctx),
            "hello {{user.unknown_field}}!"
        );
    }

    #[test]
    fn random_int_and_iso_now_format_cleanly() {
        let ctx = VarContext {
            method: "GET",
            host: "api.vicanso.dev",
            target: "/",
            headers: &[],
            body: &[],
            status: None,
            now_ms: 1_700_000_000_789,
        };
        assert_eq!(expand_str("{{iso_now}}", &ctx), "2023-11-14T22:13:20.789Z");
        for _ in 0..10 {
            let n: i64 = expand_str("{{random_int(10, 20)}}", &ctx).parse().unwrap();
            assert!((10..=20).contains(&n));
        }
    }
}
