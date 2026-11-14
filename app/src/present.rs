//! Display helpers that do not touch the window: cURL, pretty bodies, query
//! strings, and the remembered listen port.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::i18n::{Lang, t};
use crate::session::Flow;

pub const DEFAULT_PORT: u16 = 8877;

pub fn parse_port(text: &str) -> Option<u16> {
    let port: u16 = text.trim().parse().ok()?;
    if port == 0 { None } else { Some(port) }
}

pub fn curl_command(flow: &Flow) -> String {
    let mut command = format!(
        "curl -X {} {}",
        shell_quote(&flow.method),
        shell_quote(&flow.url)
    );
    for (name, value) in &flow.request_headers {
        if name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("host")
            || name.eq_ignore_ascii_case("proxy-connection")
            || name.eq_ignore_ascii_case("proxy-authorization")
        {
            continue;
        }
        command.push_str(" -H ");
        command.push_str(&shell_quote(&format!("{name}: {value}")));
    }
    if !flow.request_body.is_empty()
        && let Ok(text) = std::str::from_utf8(&flow.request_body)
    {
        command.push_str(" --data-binary ");
        command.push_str(&shell_quote(text));
    }
    command
}

pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

pub fn query_pairs(url: &str) -> Vec<(String, String)> {
    let Some(query) = url.split_once('?').map(|(_, query)| query) else {
        return Vec::new();
    };
    let query = query.split('#').next().unwrap_or(query);
    if query.is_empty() {
        return Vec::new();
    }
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(key), percent_decode(value))
        })
        .collect()
}

pub fn pretty_body(content_type: Option<&str>, body: &str) -> String {
    let kind = content_type.unwrap_or("").to_ascii_lowercase();
    if (kind.contains("json") || kind.contains("graphql") || looks_like_json(body))
        && let Some(gql) = pretty_graphql_json(body)
    {
        return gql;
    }
    if (kind.contains("json") || looks_like_json(body))
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(body)
        && let Ok(pretty) = serde_json::to_string_pretty(&value)
    {
        return pretty;
    }
    if let Some(pairs) = multipart_pairs(content_type, body.as_bytes()) {
        return pairs
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("\n");
    }
    if kind.contains("x-www-form-urlencoded")
        || (!kind.contains('/') && body.contains('=') && body.contains('&'))
    {
        return pretty_form(body);
    }
    if kind.contains("xml") || kind.contains("html") {
        return pretty_markup(body);
    }
    body.to_string()
}

/// Decodes binary Protobuf (including gRPC length-prefixed frames) or MessagePack
/// payloads into a human-readable string when `pretty` mode is active.
pub fn pretty_binary_body(content_type: Option<&str>, body: &[u8]) -> Option<String> {
    if is_msgpack(content_type) {
        return decode_msgpack_to_json(body);
    }
    if is_protobuf(content_type) {
        return decode_protobuf_wire(content_type, body);
    }
    None
}

/// Case-insensitive keyword search inside a body text. Returns `(match_count, filtered_text)`
/// where each matching line is prefixed with `[L<n>] ` so its original line number stays visible.
pub fn filter_body_lines(text: &str, query: &str) -> (usize, String) {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() || text.is_empty() {
        return (0, text.to_string());
    }
    let mut hits = 0usize;
    let mut matched_lines = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let lower = line.to_lowercase();
        let line_hits = lower.matches(&needle).count();
        if line_hits > 0 {
            hits += line_hits;
            let line_no = idx + 1;
            matched_lines.push(format!("[L{line_no}] {line}"));
        }
    }
    (hits, matched_lines.join("\n"))
}

pub fn content_type(headers: &[(String, String)]) -> Option<&str> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.as_str())
}

/// Pretty mode draws the image and a byte count. Raw keeps the hex text.
pub struct ImageBody {
    pub text: String,
    pub note: String,
    pub caption: String,
}

pub fn image_body(
    pretty: bool,
    has_preview: bool,
    text: String,
    note: String,
    bytes: usize,
    lang: Lang,
) -> ImageBody {
    if !pretty || !has_preview {
        return ImageBody {
            text,
            note,
            caption: String::new(),
        };
    }
    let caption = match lang {
        Lang::Zh => format!("共 {bytes} 字节"),
        Lang::En => format!("{bytes} bytes"),
    };
    ImageBody {
        text: String::new(),
        note: without_hex_cap_note(&note),
        caption,
    }
}

/// The hex-window note belongs on the raw dump. A decoded preview drops it
/// and keeps every other note, such as a failed decompression.
pub fn without_hex_cap_note(note: &str) -> String {
    note.lines()
        .filter(|line| !is_hex_cap_note(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_hex_cap_note(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("十六进制只显示前 ") || line.starts_with("Hex shows the first ")
}

/// Pixel size of a PNG, JPEG, GIF, WebP, or ICO body. Unknown bytes are skipped.
pub fn image_pixel_size(bytes: &[u8]) -> Option<(u32, u32)> {
    png_size(bytes)
        .or_else(|| gif_size(bytes))
        .or_else(|| webp_size(bytes))
        .or_else(|| jpeg_size(bytes))
        .or_else(|| ico_size(bytes))
        .filter(|&(w, h)| w > 0 && h > 0 && w <= 32_768 && h <= 32_768)
}

pub fn sniff_raster_extension(bytes: &[u8]) -> Option<&'static str> {
    if png_size(bytes).is_some() {
        Some("png")
    } else if gif_size(bytes).is_some() {
        Some("gif")
    } else if webp_size(bytes).is_some() {
        Some("webp")
    } else if jpeg_size(bytes).is_some() {
        Some("jpg")
    } else {
        None
    }
}

/// Display size. A small icon stays at its own pixels. A larger picture
/// shrinks so neither side passes 200.
pub fn preview_box(width: u32, height: u32) -> (u32, u32) {
    const MAX: f32 = 200.0;
    if width == 0 || height == 0 {
        return (160, 160);
    }
    let scale = (MAX / width as f32).min(MAX / height as f32).min(1.0);
    let w = ((width as f32) * scale).round().max(1.0) as u32;
    let h = ((height as f32) * scale).round().max(1.0) as u32;
    (w, h)
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    Some((
        u32::from_be_bytes(bytes[16..20].try_into().ok()?),
        u32::from_be_bytes(bytes[20..24].try_into().ok()?),
    ))
}

fn gif_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 10 || !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return None;
    }
    Some((
        u16::from_le_bytes(bytes[6..8].try_into().ok()?) as u32,
        u16::from_le_bytes(bytes[8..10].try_into().ok()?) as u32,
    ))
}

fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 30 || !bytes.starts_with(b"RIFF") || &bytes[8..12] != b"WEBP" {
        return None;
    }
    match &bytes[12..16] {
        b"VP8X" => Some((
            1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]),
            1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]),
        )),
        b"VP8 " if bytes.len() >= 30 && bytes[23..26] == [0x9d, 0x01, 0x2a] => Some((
            u16::from_le_bytes(bytes[26..28].try_into().ok()?) as u32 & 0x3fff,
            u16::from_le_bytes(bytes[28..30].try_into().ok()?) as u32 & 0x3fff,
        )),
        b"VP8L" if bytes.len() >= 25 && bytes[20] == 0x2f => {
            let bits = u32::from_le_bytes(bytes[21..25].try_into().ok()?);
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        _ => None,
    }
}

fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xff || bytes[1] != 0xd8 {
        return None;
    }
    let mut i = 2;
    while i + 8 < bytes.len() {
        if bytes[i] != 0xff {
            i += 1;
            continue;
        }
        while i < bytes.len() && bytes[i] == 0xff {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let marker = bytes[i];
        i += 1;
        if marker == 0xd8 || marker == 0xd9 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        if i + 1 >= bytes.len() {
            break;
        }
        let len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
        if len < 2 || i + len > bytes.len() {
            break;
        }
        let sof = (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc);
        if sof && len >= 7 {
            let height = u16::from_be_bytes([bytes[i + 3], bytes[i + 4]]) as u32;
            let width = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            return Some((width, height));
        }
        i += len;
    }
    None
}

pub fn image_extension(content_type: &str) -> Option<&'static str> {
    let base = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase();
    if base.contains("png") {
        Some("png")
    } else if base.contains("jpeg") || base.contains("jpg") {
        Some("jpg")
    } else if base.contains("webp") {
        Some("webp")
    } else if base.contains("gif") {
        Some("gif")
    } else if base.contains("svg") {
        Some("svg")
    } else if base.contains("x-icon")
        || base.contains("vnd.microsoft.icon")
        || matches!(base.as_str(), "image/ico" | "image/icon")
    {
        Some("ico")
    } else {
        None
    }
}

fn ico_size(bytes: &[u8]) -> Option<(u32, u32)> {
    best_ico_entry(bytes).map(|entry| (entry.width, entry.height))
}

/// Converts a Windows `.ico` body (or raw PNG bytes served as `image/x-icon`)
/// into a standard PNG buffer so every platform's `image` view can render it.
pub fn ico_to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    if png_size(bytes).is_some() {
        return Some(bytes.to_vec());
    }
    let entry = best_ico_entry(bytes)?;
    if png_size(entry.data).is_some() {
        return Some(entry.data.to_vec());
    }
    let (width, height, rgba) = decode_ico_dib(&entry)?;
    png_rgba(width, height, &rgba)
}

struct IcoEntry<'a> {
    width: u32,
    height: u32,
    bpp: u16,
    dir_height: u32,
    data: &'a [u8],
}

fn best_ico_entry(bytes: &[u8]) -> Option<IcoEntry<'_>> {
    if bytes.len() < 6 || bytes[0..2] != [0, 0] {
        return None;
    }
    if u16::from_le_bytes([bytes[2], bytes[3]]) != 1 {
        return None;
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    if count == 0 || count > 256 || 6usize.checked_add(count * 16)? > bytes.len() {
        return None;
    }
    let mut best: Option<IcoEntry<'_>> = None;
    for idx in 0..count {
        let base = 6 + idx * 16;
        let dir_w = if bytes[base] == 0 {
            256
        } else {
            u32::from(bytes[base])
        };
        let dir_h = if bytes[base + 1] == 0 {
            256
        } else {
            u32::from(bytes[base + 1])
        };
        let size = u32::from_le_bytes(bytes[base + 8..base + 12].try_into().ok()?) as usize;
        let offset = u32::from_le_bytes(bytes[base + 12..base + 16].try_into().ok()?) as usize;
        if size == 0 || offset.checked_add(size).is_none_or(|end| end > bytes.len()) {
            continue;
        }
        let data = &bytes[offset..offset + size];
        let Some((width, height, bpp)) = ico_entry_meta(dir_w, dir_h, data) else {
            continue;
        };
        let replace = match &best {
            None => true,
            Some(cur) => {
                (u64::from(width) * u64::from(height), bpp)
                    > (u64::from(cur.width) * u64::from(cur.height), cur.bpp)
            }
        };
        if replace {
            best = Some(IcoEntry {
                width,
                height,
                bpp,
                dir_height: dir_h,
                data,
            });
        }
    }
    best
}

fn ico_entry_meta(_dir_w: u32, dir_h: u32, data: &[u8]) -> Option<(u32, u32, u16)> {
    if let Some((w, h)) = png_size(data) {
        if w > 0 && h > 0 && w <= 1024 && h <= 1024 {
            return Some((w, h, 32));
        }
        return None;
    }
    let dib = parse_ico_dib_header(dir_h, data)?;
    Some((dib.width, dib.height, dib.bpp))
}

struct DibHeader {
    header_size: usize,
    width: u32,
    height: u32,
    top_down: bool,
    bpp: u16,
    palette_colors: usize,
    xor_offset: usize,
    xor_stride: usize,
    xor_len: usize,
}

fn parse_ico_dib_header(dir_h: u32, data: &[u8]) -> Option<DibHeader> {
    if data.len() < 40 {
        return None;
    }
    let header_size = u32::from_le_bytes(data[0..4].try_into().ok()?) as usize;
    if !(40..=data.len()).contains(&header_size) {
        return None;
    }
    let raw_w = i32::from_le_bytes(data[4..8].try_into().ok()?);
    let raw_h = i32::from_le_bytes(data[8..12].try_into().ok()?);
    if raw_w <= 0 || raw_w > 1024 || raw_h == 0 || raw_h == i32::MIN {
        return None;
    }
    let width = raw_w as u32;
    let abs_h = raw_h.unsigned_abs();
    let height = if abs_h == dir_h { dir_h } else { abs_h / 2 };
    if height == 0 || height > 1024 {
        return None;
    }
    let bpp = u16::from_le_bytes(data[14..16].try_into().ok()?);
    if !matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32) {
        return None;
    }
    let compression = u32::from_le_bytes(data[16..20].try_into().ok()?);
    if compression != 0 {
        return None;
    }
    let clr_used = u32::from_le_bytes(data[32..36].try_into().ok()?) as usize;
    let palette_colors = if bpp <= 8 {
        let max_colors = 1usize << bpp;
        if clr_used == 0 {
            max_colors
        } else {
            clr_used.min(max_colors)
        }
    } else {
        clr_used.min(256)
    };
    let xor_offset = header_size.checked_add(palette_colors * 4)?;
    let xor_stride = (width as usize * bpp as usize).div_ceil(32) * 4;
    let xor_len = xor_stride.checked_mul(height as usize)?;
    if xor_offset.checked_add(xor_len)? > data.len() {
        return None;
    }
    Some(DibHeader {
        header_size,
        width,
        height,
        top_down: raw_h < 0,
        bpp,
        palette_colors,
        xor_offset,
        xor_stride,
        xor_len,
    })
}

fn decode_ico_dib(entry: &IcoEntry<'_>) -> Option<(u32, u32, Vec<u8>)> {
    let data = entry.data;
    let dib = parse_ico_dib_header(entry.dir_height, data)?;
    let w = dib.width as usize;
    let h = dib.height as usize;
    let palette = &data[dib.header_size..dib.xor_offset];
    let xor_data = &data[dib.xor_offset..dib.xor_offset + dib.xor_len];
    let and_offset = dib.xor_offset + dib.xor_len;
    let and_stride = w.div_ceil(32) * 4;
    let and_data = and_stride
        .checked_mul(h)
        .and_then(|len| and_offset.checked_add(len))
        .and_then(|end| data.get(and_offset..end));

    let mut rgba = Vec::with_capacity(w * h * 4);
    let mut any_alpha = false;
    for y in 0..h {
        let src_y = if dib.top_down { y } else { h - 1 - y };
        let row = &xor_data[src_y * dib.xor_stride..(src_y + 1) * dib.xor_stride];
        let and_row = and_data.map(|m| &m[src_y * and_stride..(src_y + 1) * and_stride]);
        for x in 0..w {
            let masked = and_row.is_some_and(|mr| (mr[x / 8] >> (7 - (x % 8))) & 1 != 0);
            let mask_alpha = if masked { 0 } else { 255 };
            let (r, g, b, a) = match dib.bpp {
                32 => {
                    let p = x * 4;
                    let a = row[p + 3];
                    if a != 0 {
                        any_alpha = true;
                    }
                    (row[p + 2], row[p + 1], row[p], a)
                }
                24 => {
                    let p = x * 3;
                    (row[p + 2], row[p + 1], row[p], mask_alpha)
                }
                16 => {
                    let v = u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]);
                    let r = (((v >> 10) & 0x1f) * 255 / 31) as u8;
                    let g = (((v >> 5) & 0x1f) * 255 / 31) as u8;
                    let b = ((v & 0x1f) * 255 / 31) as u8;
                    (r, g, b, mask_alpha)
                }
                8 => {
                    let (r, g, b) = palette_rgb(palette, dib.palette_colors, row[x] as usize);
                    (r, g, b, mask_alpha)
                }
                4 => {
                    let byte = row[x / 2];
                    let idx = if x % 2 == 0 {
                        (byte >> 4) & 0x0f
                    } else {
                        byte & 0x0f
                    } as usize;
                    let (r, g, b) = palette_rgb(palette, dib.palette_colors, idx);
                    (r, g, b, mask_alpha)
                }
                1 => {
                    let idx = ((row[x / 8] >> (7 - (x % 8))) & 1) as usize;
                    let (r, g, b) = palette_rgb(palette, dib.palette_colors, idx);
                    (r, g, b, mask_alpha)
                }
                _ => return None,
            };
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }
    if dib.bpp == 32 && !any_alpha {
        if let Some(mask) = and_data {
            for y in 0..h {
                let src_y = if dib.top_down { y } else { h - 1 - y };
                let and_row = &mask[src_y * and_stride..(src_y + 1) * and_stride];
                for x in 0..w {
                    let masked = (and_row[x / 8] >> (7 - (x % 8))) & 1 != 0;
                    rgba[(y * w + x) * 4 + 3] = if masked { 0 } else { 255 };
                }
            }
        } else {
            for px in rgba.as_chunks_mut::<4>().0 {
                px[3] = 255;
            }
        }
    }
    Some((dib.width, dib.height, rgba))
}

fn palette_rgb(palette: &[u8], colors: usize, idx: usize) -> (u8, u8, u8) {
    if idx < colors {
        let p = idx * 4;
        (palette[p + 2], palette[p + 1], palette[p])
    } else {
        (0, 0, 0)
    }
}

fn png_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    use std::io::Write;
    let row_bytes = width as usize * 4;
    if rgba.len() != row_bytes * height as usize {
        return None;
    }
    let mut raw = Vec::with_capacity((row_bytes + 1) * height as usize);
    for row in rgba.chunks_exact(row_bytes) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&raw).ok()?;
    let zlib_data = encoder.finish().ok()?;

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = Vec::with_capacity(64 + zlib_data.len());
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    out.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
    out.extend_from_slice(&png_chunk(b"IDAT", &zlib_data));
    out.extend_from_slice(&png_chunk(b"IEND", &[]));
    Some(out)
}

pub(crate) fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(4 + data.len());
    body.extend_from_slice(kind);
    body.extend_from_slice(data);
    let crc = crc32(&body);
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc.to_be_bytes());
    out
}

pub(crate) fn crc32(data: &[u8]) -> u32 {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// Splits a `text/event-stream` body into events. A blank line ends one.
pub fn sse_events(body: &str) -> Vec<SseEvent> {
    let mut events = Vec::new();
    let mut event = "message".to_string();
    let mut data: Vec<String> = Vec::new();
    for line in body.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            if !data.is_empty() {
                events.push(SseEvent {
                    event: event.clone(),
                    data: data.join("\n"),
                });
                data.clear();
                event = "message".to_string();
            }
            continue;
        }
        if line.starts_with(':') {
            continue;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => event = value.to_string(),
            "data" => data.push(value.to_string()),
            _ => {}
        }
    }
    if !data.is_empty() {
        events.push(SseEvent {
            event,
            data: data.join("\n"),
        });
    }
    events
}

pub fn format_timing(
    started: SystemTime,
    ttfb: Option<Duration>,
    total: Option<Duration>,
    request_bytes: u64,
    response_bytes: u64,
    lang: Lang,
) -> String {
    let mut parts = vec![format_clock(started)];
    if let Some(ttfb) = ttfb {
        parts.push(format!("TTFB {}", format_duration(ttfb)));
    }
    if let Some(total) = total {
        parts.push(format!(
            "{} {}",
            t(lang, "耗时", "elapsed"),
            format_duration(total)
        ));
    } else {
        parts.push(t(lang, "进行中", "in progress").into());
    }
    parts.push(format!(
        "{} {}",
        t(lang, "请求", "request"),
        format_count(request_bytes)
    ));
    parts.push(format!(
        "{} {}",
        t(lang, "响应", "response"),
        format_count(response_bytes)
    ));
    parts.join(" · ")
}

pub fn is_slow(started: SystemTime, finished: Option<SystemTime>, now: SystemTime) -> bool {
    crate::session::is_slow(started, finished, now)
}

pub const REPEAT_COUNT_MIN: u32 = 1;
pub const REPEAT_COUNT_MAX: u32 = 100;
pub const REPEAT_COUNT_DEFAULT: u32 = 5;
pub const REPEAT_CONCURRENCY_MIN: u32 = 1;
pub const REPEAT_CONCURRENCY_MAX: u32 = 20;
pub const REPEAT_CONCURRENCY_DEFAULT: u32 = 1;
pub const REPEAT_INTERVAL_MAX_MS: u64 = 60_000;
pub const REPEAT_INTERVAL_DEFAULT_MS: u64 = 200;

pub fn parse_repeat_count(text: &str) -> u32 {
    text.trim()
        .parse::<u32>()
        .unwrap_or(REPEAT_COUNT_DEFAULT)
        .clamp(REPEAT_COUNT_MIN, REPEAT_COUNT_MAX)
}

pub fn parse_repeat_concurrency(text: &str) -> u32 {
    text.trim()
        .parse::<u32>()
        .unwrap_or(REPEAT_CONCURRENCY_DEFAULT)
        .clamp(REPEAT_CONCURRENCY_MIN, REPEAT_CONCURRENCY_MAX)
}

pub fn parse_repeat_interval_ms(text: &str) -> u64 {
    text.trim()
        .parse::<u64>()
        .unwrap_or(REPEAT_INTERVAL_DEFAULT_MS)
        .min(REPEAT_INTERVAL_MAX_MS)
}

/// Returns true if the message line label represents a WebSocket message.
pub fn is_ws_message(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with("客户端 ")
        || lower.starts_with("client ")
        || lower.starts_with("源站 ")
        || lower.starts_with("origin ")
        || lower.starts_with('↑')
        || lower.starts_with('↓')
}

pub fn is_ws_client_message(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with("客户端 ") || lower.starts_with("client ") || lower.starts_with('↑')
}

pub fn is_ws_origin_message(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with("源站 ") || lower.starts_with("origin ") || lower.starts_with('↓')
}

/// Checks whether a WebSocket or SSE message row (`(name, value)`) matches the
/// details message filter (`query`, `dir`: `0 = All`, `1 = Client ↑`, `2 = Origin ↓`,
/// `kind`: `0 = All`, `1 = Text`, `2 = Binary`, `3 = Control (ping/pong/close)`).
pub fn message_matches_filter(name: &str, value: &str, query: &str, dir: u8, kind: u8) -> bool {
    if name == "…" {
        return true;
    }
    let is_client = is_ws_client_message(name);
    let is_origin = is_ws_origin_message(name);
    let is_ws = is_client || is_origin;
    match dir {
        1 if is_ws && !is_client => return false,
        2 if is_ws && !is_origin => return false,
        _ => {}
    }
    if is_ws && kind != 0 {
        let name_lower = name.to_lowercase();
        let is_text = name_lower.ends_with(" 文本") || name_lower.ends_with(" text");
        let is_binary = name_lower.ends_with(" 二进制") || name_lower.ends_with(" binary");
        let is_ctrl = name_lower.ends_with(" ping")
            || name_lower.ends_with(" pong")
            || name_lower.ends_with(" 关闭")
            || name_lower.ends_with(" close");
        match kind {
            1 if !is_text => return false,
            2 if !is_binary => return false,
            3 if !is_ctrl => return false,
            _ => {}
        }
    }
    let needle = query.trim().to_lowercase();
    if !needle.is_empty() {
        let name_lower = name.to_lowercase();
        let value_lower = value.to_lowercase();
        if !name_lower.contains(&needle) && !value_lower.contains(&needle) {
            return false;
        }
    }
    true
}

pub fn bytes(bytes: usize) -> String {
    format_count(bytes as u64)
}

fn encode_userinfo_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for &byte in raw.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Shell exports so curl, Node, Python, Git, Cargo, Deno, and JVM tools trust this proxy.
#[cfg(test)]
pub fn proxy_env(port: u16, cert_path: &str) -> String {
    proxy_env_with_auth(port, cert_path, "")
}

pub fn proxy_env_with_auth(port: u16, cert_path: &str, proxy_auth: &str) -> String {
    let trimmed_auth = proxy_auth.trim();
    let proxy = if trimmed_auth.is_empty() {
        format!("http://127.0.0.1:{port}")
    } else {
        let (user, pass) = trimmed_auth.split_once(':').unwrap_or((trimmed_auth, ""));
        format!(
            "http://{}:{}@127.0.0.1:{port}",
            encode_userinfo_component(user),
            encode_userinfo_component(pass)
        )
    };
    let cert = shell_quote(cert_path);
    let java_opts = shell_quote(&format!(
        "-Dhttp.proxyHost=127.0.0.1 -Dhttp.proxyPort={port} -Dhttps.proxyHost=127.0.0.1 -Dhttps.proxyPort={port}"
    ));
    [
        format!("export http_proxy={proxy}"),
        format!("export https_proxy={proxy}"),
        format!("export all_proxy={proxy}"),
        format!("export HTTP_PROXY={proxy}"),
        format!("export HTTPS_PROXY={proxy}"),
        format!("export ALL_PROXY={proxy}"),
        format!("export SSL_CERT_FILE={cert}"),
        format!("export NODE_EXTRA_CA_CERTS={cert}"),
        format!("export REQUESTS_CA_BUNDLE={cert}"),
        format!("export CURL_CA_BUNDLE={cert}"),
        format!("export GIT_SSL_CAINFO={cert}"),
        format!("export CARGO_HTTP_CAINFO={cert}"),
        format!("export DENO_CERT={cert}"),
        format!("export JAVA_TOOL_OPTIONS={java_opts}"),
    ]
    .join("\n")
}

#[cfg(test)]
pub fn proxy_env_export(port: u16, cert_path: &str) -> String {
    proxy_env(port, cert_path)
}

pub fn proxy_env_export_with_auth(port: u16, cert_path: &str, proxy_auth: &str) -> String {
    proxy_env_with_auth(port, cert_path, proxy_auth)
}

pub fn fetch_snippet(flow: &Flow) -> String {
    let headers = copy_headers(&flow.request_headers);
    let mut lines = vec![
        format!("fetch({}, {{", js_string(&flow.url)),
        format!("  method: {},", js_string(&flow.method)),
    ];
    if headers.is_empty() {
        lines.push("  headers: {},".into());
    } else {
        lines.push("  headers: {".into());
        for (name, value) in &headers {
            lines.push(format!("    {}: {},", js_string(name), js_string(value)));
        }
        lines.push("  },".into());
    }
    if let Ok(text) = std::str::from_utf8(&flow.request_body)
        && !text.is_empty()
    {
        lines.push(format!("  body: {},", js_string(text)));
    }
    lines.push("});".into());
    lines.join("\n")
}

pub fn python_requests(flow: &Flow) -> String {
    let headers = copy_headers(&flow.request_headers);
    let mut lines = vec![
        "import requests".into(),
        "requests.request(".into(),
        format!("    {},", py_string(&flow.method)),
        format!("    {},", py_string(&flow.url)),
    ];
    if !headers.is_empty() {
        lines.push("    headers={".into());
        for (name, value) in &headers {
            lines.push(format!(
                "        {}: {},",
                py_string(name),
                py_string(value)
            ));
        }
        lines.push("    },".into());
    }
    if let Ok(text) = std::str::from_utf8(&flow.request_body)
        && !text.is_empty()
    {
        lines.push(format!("    data={},", py_string(text)));
    }
    lines.push(")".into());
    lines.join("\n")
}

/// A shallow TypeScript interface for a JSON object or array. Nested objects
/// become sibling interfaces. Anything else returns `None`.
pub fn typescript_interface(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json.trim()).ok()?;
    if !value.is_object() && !value.is_array() {
        return None;
    }
    let mut types = TsGen::default();
    if value.is_array() {
        let root = types.emit("Item", &value);
        types.blocks.push(format!("export type Response = {root};"));
    } else {
        types.emit("Response", &value);
    }
    Some(types.blocks.join("\n"))
}

pub fn rust_reqwest(flow: &Flow) -> String {
    let headers = copy_headers(&flow.request_headers);
    let method = flow.method.to_ascii_uppercase();
    let method_expr = match method.as_str() {
        "GET" => "reqwest::Method::GET".to_string(),
        "POST" => "reqwest::Method::POST".to_string(),
        "PUT" => "reqwest::Method::PUT".to_string(),
        "DELETE" => "reqwest::Method::DELETE".to_string(),
        "PATCH" => "reqwest::Method::PATCH".to_string(),
        "HEAD" => "reqwest::Method::HEAD".to_string(),
        "OPTIONS" => "reqwest::Method::OPTIONS".to_string(),
        _ => format!(
            "reqwest::Method::from_bytes({}.as_bytes())?",
            rust_string(&flow.method)
        ),
    };
    let mut lines = vec![
        "let client = reqwest::Client::new();".into(),
        format!(
            "let response = client\n    .request({method_expr}, {})",
            rust_string(&flow.url)
        ),
    ];
    for (name, value) in &headers {
        lines.push(format!(
            "    .header({}, {})",
            rust_string(name),
            rust_string(value)
        ));
    }
    if let Ok(text) = std::str::from_utf8(&flow.request_body)
        && !text.is_empty()
    {
        lines.push(format!("    .body({})", rust_string(text)));
    }
    lines.push("    .send()\n    .await?;".into());
    lines.join("\n")
}

pub fn go_http(flow: &Flow) -> String {
    let headers = copy_headers(&flow.request_headers);
    let body_text = std::str::from_utf8(&flow.request_body)
        .ok()
        .filter(|t| !t.is_empty());
    let mut lines = vec![
        "package main".into(),
        "".into(),
        "import (".into(),
        "\t\"net/http\"".into(),
    ];
    if body_text.is_some() {
        lines.push("\t\"strings\"".into());
    }
    lines.extend([
        ")".into(),
        "".into(),
        "func doRequest() (*http.Response, error) {".into(),
    ]);
    let body_arg = match body_text {
        Some(text) => format!("strings.NewReader({})", js_string(text)),
        None => "nil".to_string(),
    };
    lines.push(format!(
        "\treq, err := http.NewRequest({}, {}, {body_arg})",
        js_string(&flow.method),
        js_string(&flow.url)
    ));
    lines.push("\tif err != nil {\n\t\treturn nil, err\n\t}".into());
    for (name, value) in &headers {
        lines.push(format!(
            "\treq.Header.Set({}, {})",
            js_string(name),
            js_string(value)
        ));
    }
    lines.push("\treturn http.DefaultClient.Do(req)".into());
    lines.push("}".into());
    lines.join("\n")
}

/// Generates Rust `#[derive(Debug, Clone, Serialize, Deserialize)]` structs
/// from a JSON object or array. Nested objects become sibling structs.
pub fn rust_serde_struct(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json.trim()).ok()?;
    if !value.is_object() && !value.is_array() {
        return None;
    }
    let mut gen_state = RustStructGen::default();
    if value.is_array() {
        let root = gen_state.emit("Item", &value);
        gen_state
            .blocks
            .push(format!("pub type Response = {root};"));
    } else {
        gen_state.emit("Response", &value);
    }
    Some(format!(
        "use serde::{{Deserialize, Serialize}};\n\n{}",
        gen_state.blocks.join("\n\n")
    ))
}

/// Formats binary bytes as 16-bytes-per-line hex without line numbers.
pub fn hex_dump_raw(bytes: &[u8]) -> String {
    bytes
        .chunks(16)
        .map(|chunk| {
            chunk
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurlRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// Parses a single curl command. Supports `-X`/`--request`, `-H`/`--header`,
/// `--url`, `-I`/`--head`, `-G`/`--get`, `--json`, `--data-urlencode`, and
/// `-d`/`--data`/`--data-raw`/`--data-binary`/`--data-ascii` (joining multiple
/// data flags with `&`), with shell quotes and continuations. Unknown flags are ignored.
pub fn parse_curl(text: &str) -> Option<CurlRequest> {
    let tokens = shell_tokens(text);
    if tokens.is_empty() {
        return None;
    }
    let mut index = 0;
    if tokens[0] == "curl" {
        index = 1;
    }
    let mut method = String::new();
    let mut url = String::new();
    let mut headers = Vec::new();
    let mut data_parts: Vec<String> = Vec::new();
    let mut saw_json = false;
    let mut saw_head = false;
    let mut saw_get = false;
    while index < tokens.len() {
        let token = tokens[index].as_str();
        let (flag, inline) = split_flag(token);
        let arg = |index: &mut usize| -> Option<String> {
            if let Some(inline) = inline {
                return Some(inline.to_string());
            }
            *index += 1;
            tokens.get(*index).cloned()
        };
        match flag {
            "-X" | "--request" => method = arg(&mut index)?,
            "-H" | "--header" => {
                let header = arg(&mut index)?;
                let (name, value) = header.split_once(':')?;
                headers.push((name.trim().to_string(), value.trim().to_string()));
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-ascii" => {
                data_parts.push(arg(&mut index)?);
            }
            "--data-urlencode" => {
                let raw = arg(&mut index)?;
                data_parts.push(encode_curl_data_urlencode(&raw));
            }
            "--json" => {
                data_parts.push(arg(&mut index)?);
                saw_json = true;
            }
            "-I" | "--head" => saw_head = true,
            "-G" | "--get" => saw_get = true,
            "--url" => url = arg(&mut index)?,
            "-A" | "--user-agent" => {
                headers.push(("User-Agent".into(), arg(&mut index)?));
            }
            "-e" | "--referer" => {
                headers.push(("Referer".into(), arg(&mut index)?));
            }
            "-b" | "--cookie" => {
                headers.push(("Cookie".into(), arg(&mut index)?));
            }
            "-u" | "--user" | "-x" | "--proxy" | "-o" | "--output" | "-m" | "--max-time"
            | "--connect-timeout" | "-w" | "--write-out" => {
                let _ = arg(&mut index)?;
            }
            other if other.starts_with('-') => {
                if !other.starts_with("--")
                    && other[1..].chars().all(|c| {
                        matches!(
                            c,
                            's' | 'S' | 'L' | 'k' | 'v' | 'i' | 'f' | 'N' | '4' | '6' | 'I' | 'G'
                        )
                    })
                {
                    if other.contains('I') {
                        saw_head = true;
                    }
                    if other.contains('G') {
                        saw_get = true;
                    }
                }
            }
            other if url.is_empty() => url = other.to_string(),
            _ => {}
        }
        index += 1;
    }
    if url.is_empty() {
        return None;
    }
    if saw_json {
        if !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        {
            headers.push(("Content-Type".into(), "application/json".into()));
        }
        if !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("accept"))
        {
            headers.push(("Accept".into(), "application/json".into()));
        }
    }
    let saw_body = !data_parts.is_empty();
    let joined_data = data_parts.join("&");
    let body = if saw_get {
        if !joined_data.is_empty() {
            url = append_query_to_url(&url, &joined_data);
        }
        String::new()
    } else {
        joined_data
    };
    if method.is_empty() {
        method = if saw_head {
            "HEAD".into()
        } else if saw_get {
            "GET".into()
        } else if saw_body {
            "POST".into()
        } else {
            "GET".into()
        };
    }
    Some(CurlRequest {
        method,
        url,
        headers,
        body,
    })
}

fn encode_curl_data_urlencode(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix('=') {
        percent_encode_curl_component(rest)
    } else if let Some((name, content)) = raw.split_once('=') {
        format!("{name}={}", percent_encode_curl_component(content))
    } else {
        percent_encode_curl_component(raw)
    }
}

fn percent_encode_curl_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn append_query_to_url(url: &str, query: &str) -> String {
    let (base, frag) = match url.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (url, None),
    };
    let mut out = String::with_capacity(url.len() + query.len() + 1);
    out.push_str(base);
    if base.contains('?') {
        if !base.ends_with('?') && !base.ends_with('&') {
            out.push('&');
        }
    } else {
        out.push('?');
    }
    out.push_str(query);
    if let Some(f) = frag {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// Extra readable text for a header value: JWT payload, a unix time, or
/// printable base64. The original value stays intact.
/// Splits a `Cookie` or `Set-Cookie` value on `;`. A piece without `=` is a flag.
pub fn cookie_pairs(value: &str) -> Vec<(String, String)> {
    value
        .split(';')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            let (key, val) = part.split_once('=').unwrap_or((part, ""));
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), val.trim().to_string()))
        })
        .collect()
}

/// Case-insensitive `a–z`. Equal names keep their original order and case.
pub fn sort_headers_pretty(headers: &mut [(String, String)]) {
    headers.sort_by(|left, right| {
        left.0
            .to_ascii_lowercase()
            .cmp(&right.0.to_ascii_lowercase())
    });
}

/// Pretty rows for one `Cookie` header. Other headers (including `Set-Cookie`) return `None`.
pub fn pretty_cookie_rows(header: &str, value: &str) -> Option<Vec<(String, String)>> {
    if !header.eq_ignore_ascii_case("cookie") {
        return None;
    }
    let pairs = cookie_pairs(value);
    if pairs.is_empty() {
        return None;
    }
    Some(
        pairs
            .into_iter()
            .map(|(key, val)| {
                let shown = if val.is_empty() {
                    String::new()
                } else if let Some(hint) = annotate_value(&key, &val) {
                    format!("{val}\n{hint}")
                } else {
                    val
                };
                (format!("{header} · {key}"), shown)
            })
            .collect(),
    )
}

pub fn annotate_value(name: &str, value: &str) -> Option<String> {
    if let Some(text) = jwt_hint(name, value) {
        return Some(text);
    }
    if let Some(text) = timestamp_hint(value.trim()) {
        return Some(text);
    }
    base64_hint(value.trim())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(test)]
pub enum DiffKind {
    Same,
    Add,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(test)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub text: String,
}

/// Line diff capped at 200 lines on each side.
#[cfg(test)]
pub fn line_diff(before: &str, after: &str) -> Vec<DiffLine> {
    let left: Vec<&str> = before.lines().take(200).collect();
    let right: Vec<&str> = after.lines().take(200).collect();
    let rows = left.len();
    let cols = right.len();
    let mut score = vec![vec![0u16; cols + 1]; rows + 1];
    for i in (0..rows).rev() {
        for j in (0..cols).rev() {
            score[i][j] = if left[i] == right[j] {
                score[i + 1][j + 1].saturating_add(1)
            } else {
                score[i + 1][j].max(score[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < rows && j < cols {
        if left[i] == right[j] {
            out.push(DiffLine {
                kind: DiffKind::Same,
                text: left[i].to_string(),
            });
            i += 1;
            j += 1;
        } else if score[i + 1][j] >= score[i][j + 1] {
            out.push(DiffLine {
                kind: DiffKind::Remove,
                text: left[i].to_string(),
            });
            i += 1;
        } else {
            out.push(DiffLine {
                kind: DiffKind::Add,
                text: right[j].to_string(),
            });
            j += 1;
        }
    }
    while i < rows {
        out.push(DiffLine {
            kind: DiffKind::Remove,
            text: left[i].to_string(),
        });
        i += 1;
    }
    while j < cols {
        out.push(DiffLine {
            kind: DiffKind::Add,
            text: right[j].to_string(),
        });
        j += 1;
    }
    out
}

/// Language for the read-only body editor. Short or binary text stays a
/// plain block. JSON, HTML, and XML use their highlighter; other long
/// multiline text uses plain text with line numbers.
pub fn editor_language(content_type: Option<&str>, text: &str) -> Option<&'static str> {
    if text.is_empty() {
        return None;
    }
    let kind = content_type.unwrap_or("").to_ascii_lowercase();
    if is_msgpack(content_type) && looks_like_json(text) {
        return Some("json");
    }
    if is_protobuf(content_type) && text.contains('\n') {
        return Some("plaintext");
    }
    if kind.contains("json") && !looks_like_json(text) && text.contains('\n') {
        return Some("plaintext");
    }
    if kind.contains("json")
        || ((kind.is_empty() || kind.contains("text/plain")) && looks_like_json(text))
    {
        return Some("json");
    }
    if kind.contains("html") {
        return Some("html");
    }
    if kind.contains("markdown") {
        return Some("markdown");
    }
    if kind.contains("xml") {
        return Some("xml");
    }
    if kind.contains("javascript") {
        return Some("javascript");
    }
    let textual = kind.is_empty()
        || kind.contains("text/")
        || kind.contains("yaml")
        || kind.contains("csv")
        || kind.contains("graphql");
    if textual && text.contains('\n') && text.len() >= 80 {
        return Some(if kind.contains("yaml") {
            "yaml"
        } else {
            "plaintext"
        });
    }
    None
}

/// Parses HTML or Markdown bodies into an [`nsh_base::text_view::Document`] when
/// `content_type` indicates `html` or `markdown` and the parsed document has at
/// least one visible block (empty SPA shells return `None`).
pub fn parse_text_view_doc(
    content_type: Option<&str>,
    bytes: &[u8],
) -> Option<nsh_base::text_view::Document> {
    let text = std::str::from_utf8(bytes).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    let kind = content_type.unwrap_or("").to_ascii_lowercase();
    let doc = if kind.contains("markdown") {
        nsh_base::text_view::Parser::new().markdown(text)
    } else if kind.contains("html") {
        nsh_base::text_view::Parser::new().html(text)
    } else {
        return None;
    };
    (!doc.blocks.is_empty()).then_some(doc)
}

/// One block the diff view can compare: URL, both header lists, both bodies.
pub fn flow_snapshot(flow: &Flow, lang: Lang) -> String {
    let mut out = String::new();
    push_section(&mut out, t(lang, "URL", "URL"), &flow.url);
    push_section(
        &mut out,
        t(lang, "请求头", "Request headers"),
        &header_block(&flow.request_headers),
    );
    push_section(
        &mut out,
        t(lang, "请求 body", "Request body"),
        &body_text(&flow.request_body),
    );
    push_section(
        &mut out,
        t(lang, "响应头", "Response headers"),
        &header_block(&flow.response_headers),
    );
    push_section(
        &mut out,
        t(lang, "响应 body", "Response body"),
        &body_text(&flow.response_body),
    );
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepKind {
    Redirect,
    Cookie,
    Token,
    QueryParam,
    BodyOrPath,
    Sequential,
}

impl DepKind {
    pub fn is_hard(self) -> bool {
        !matches!(self, Self::Sequential)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlowDependency {
    pub from_flow_id: u64,
    pub from_index: usize,
    pub kind: DepKind,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DependencyStage {
    pub stage_no: usize,
    pub start_ms: f64,
    pub end_ms: f64,
    pub offset_label: String,
    pub span_label: String,
    pub bar_indices: Vec<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SequenceStep {
    pub step_no: usize,
    pub flow_id: u64,
    pub method: String,
    pub host_idx: usize,
    pub is_request: bool,
    pub is_error: bool,
    pub label: String,
    pub note: String,
    pub active_hosts: Vec<bool>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WaterfallBar {
    pub flow_id: u64,
    pub method: String,
    pub status: Option<u16>,
    pub failed: bool,
    pub slow: bool,
    pub host: String,
    pub target: String,
    pub comment: String,
    pub start_ms: f64,
    pub end_ms: f64,
    pub offset_label: String,
    pub duration_label: String,
    pub size_label: String,
    pub stage: usize,
    pub deps: Vec<FlowDependency>,
    pub on_critical_path: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SequenceDiagramData {
    pub hosts: Vec<String>,
    pub steps: Vec<SequenceStep>,
    pub bars: Vec<WaterfallBar>,
    pub stages: Vec<DependencyStage>,
    pub hard_dep_count: usize,
    pub mermaid: String,
    pub flowchart_mermaid: String,
}

/// Builds both the structured visual sequence/waterfall diagram data and the
/// Mermaid `sequenceDiagram` text from the given flows along the real timeline.
pub fn build_sequence_diagram_data(
    flows: &[&Flow],
    include_response: bool,
    include_query: bool,
) -> SequenceDiagramData {
    build_sequence_inner(flows, include_response, include_query, true)
}

/// Only the Mermaid `sequenceDiagram` text, skipping the dependency analysis
/// (which reads bodies and compares every pair). Used for large selections
/// whose preview is capped but whose copy must still cover every flow.
pub fn build_sequence_mermaid(
    flows: &[&Flow],
    include_response: bool,
    include_query: bool,
) -> String {
    build_sequence_inner(flows, include_response, include_query, false).mermaid
}

fn build_sequence_inner(
    flows: &[&Flow],
    include_response: bool,
    include_query: bool,
    analyze: bool,
) -> SequenceDiagramData {
    let mut lines = vec![
        "sequenceDiagram".to_string(),
        "    autonumber".to_string(),
        "    participant C as Client".to_string(),
    ];
    if flows.is_empty() {
        return SequenceDiagramData {
            hosts: Vec::new(),
            steps: Vec::new(),
            bars: Vec::new(),
            stages: Vec::new(),
            hard_dep_count: 0,
            mermaid: lines.join("\n"),
            flowchart_mermaid: "flowchart TD".to_string(),
        };
    }
    let mut sorted: Vec<&Flow> = flows.to_vec();
    sorted.sort_by_key(|flow| (flow.started, flow.id));
    let base_start = sorted[0].started;
    let show_offset = sorted.len() > 1;
    let now = crate::clock::now();

    let mut participants: Vec<(String, String)> = Vec::new();
    let mut flow_host_indices: Vec<usize> = Vec::with_capacity(sorted.len());
    let mut flow_aliases: Vec<String> = Vec::with_capacity(sorted.len());
    let mut flow_raw_targets: Vec<String> = Vec::with_capacity(sorted.len());
    let mut flow_targets: Vec<String> = Vec::with_capacity(sorted.len());
    let mut end_times: Vec<SystemTime> = Vec::with_capacity(sorted.len());
    let mut bars: Vec<WaterfallBar> = Vec::with_capacity(sorted.len());

    for flow in &sorted {
        let host = resolve_flow_host(flow);
        let (host_idx, alias) = if let Some((idx, (_, existing))) = participants
            .iter()
            .enumerate()
            .find(|(_, (h, _))| h == &host)
        {
            (idx, existing.clone())
        } else {
            let idx = participants.len();
            let alias = format!("H{}", idx + 1);
            let label = format_mermaid_participant(&host);
            lines.push(format!("    participant {alias} as {label}"));
            participants.push((host.clone(), alias.clone()));
            (idx, alias)
        };
        flow_host_indices.push(host_idx);
        flow_aliases.push(alias);
        let target_raw = resolve_flow_target(flow, include_query);
        flow_raw_targets.push(target_raw.clone());
        flow_targets.push(sanitize_mermaid_text(&target_raw));
        let end_time = flow
            .finished_at
            .map(|fin| fin.max(flow.started))
            .unwrap_or(flow.started);
        end_times.push(end_time);

        let delta = flow.started.duration_since(base_start).unwrap_or_default();
        let start_ms = delta.as_secs_f64() * 1000.0;
        let dur_ms = end_time
            .duration_since(flow.started)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0;
        let end_ms = start_ms + dur_ms.max(1.0);
        let method_raw = if flow.method.trim().is_empty() {
            "GET".to_string()
        } else {
            flow.method.trim().to_string()
        };
        let res_bytes = flow.res_bytes();
        let size_label = if res_bytes > 0 {
            format_count(res_bytes as u64)
        } else {
            String::new()
        };
        bars.push(WaterfallBar {
            flow_id: flow.id,
            method: method_raw,
            status: flow.status,
            failed: flow.status.is_none() && flow.error.is_some(),
            slow: flow.finished_at.is_some() && is_slow(flow.started, flow.finished_at, now),
            host,
            target: target_raw,
            comment: flow.comment.trim().to_string(),
            start_ms,
            end_ms,
            offset_label: format!("+{}", format_delta(delta)),
            duration_label: format_elapsed(flow.started, flow.finished_at),
            size_label,
            stage: 1,
            deps: Vec::new(),
            on_critical_path: false,
        });
    }

    let (stages, hard_dep_count, flowchart_mermaid) = if analyze {
        analyze_flow_dependencies_and_stages(&sorted, &mut bars)
    } else {
        (Vec::new(), 0, "flowchart TD".to_string())
    };

    // Detect if a flow overlaps in time with another flow targeting the same host
    // so response arrows on shared lifelines can include the path to disambiguate.
    let mut same_host_overlap = vec![false; sorted.len()];
    if include_response {
        for i in 0..sorted.len() {
            for j in (i + 1)..sorted.len() {
                if flow_host_indices[i] == flow_host_indices[j] && sorted[j].started < end_times[i]
                {
                    same_host_overlap[i] = true;
                    same_host_overlap[j] = true;
                }
            }
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum EventKind {
        Request,
        Response,
    }

    let mut events: Vec<(SystemTime, EventKind, usize)> =
        Vec::with_capacity(sorted.len() * if include_response { 2 } else { 1 });
    for (idx, flow) in sorted.iter().enumerate() {
        events.push((flow.started, EventKind::Request, idx));
        if include_response {
            events.push((end_times[idx], EventKind::Response, idx));
        }
    }
    events.sort_by_key(|(time, kind, idx)| (*time, *kind, *idx));

    let host_count = participants.len();
    let mut active_depths = vec![0usize; host_count];
    let mut steps: Vec<SequenceStep> = Vec::with_capacity(events.len());
    let act = if include_response { "+" } else { "" };

    for (_, kind, idx) in events {
        let flow = sorted[idx];
        let host_idx = flow_host_indices[idx];
        let alias = &flow_aliases[idx];
        let target = &flow_targets[idx];
        let raw_target = &flow_raw_targets[idx];
        let step_no = steps.len() + 1;
        let method_raw = if flow.method.trim().is_empty() {
            "GET"
        } else {
            flow.method.trim()
        };

        match kind {
            EventKind::Request => {
                if include_response {
                    active_depths[host_idx] += 1;
                }
                let active_hosts: Vec<bool> = active_depths.iter().map(|&d| d > 0).collect();
                let comment = sanitize_mermaid_text(&flow.comment);
                if !comment.is_empty() {
                    lines.push(format!("    Note over C,{alias}: {comment}"));
                }
                let method = sanitize_mermaid_text(method_raw);
                let offset_suffix = if show_offset {
                    let delta = flow.started.duration_since(base_start).unwrap_or_default();
                    format!(" [+{}]", format_delta(delta))
                } else {
                    String::new()
                };
                let failed_without_res = flow.status.is_none() && flow.error.is_some();
                if failed_without_res {
                    lines.push(format!(
                        "    C-x{act}{alias}: {method} {target}{offset_suffix}"
                    ));
                } else {
                    lines.push(format!(
                        "    C->>{act}{alias}: {method} {target}{offset_suffix}"
                    ));
                }
                steps.push(SequenceStep {
                    step_no,
                    flow_id: flow.id,
                    method: method_raw.to_string(),
                    host_idx,
                    is_request: true,
                    is_error: failed_without_res,
                    label: format!("{method_raw} {raw_target}{offset_suffix}"),
                    note: flow.comment.trim().to_string(),
                    active_hosts,
                });
            }
            EventKind::Response => {
                let active_hosts: Vec<bool> = active_depths.iter().map(|&d| d > 0).collect();
                active_depths[host_idx] = active_depths[host_idx].saturating_sub(1);
                let path_hint = if same_host_overlap[idx] {
                    format!(" {target}")
                } else {
                    String::new()
                };
                let raw_path_hint = if same_host_overlap[idx] {
                    format!(" {raw_target}")
                } else {
                    String::new()
                };
                let (label, is_error) = match flow.status {
                    Some(code) => {
                        let status_text = match http::StatusCode::from_u16(code)
                            .ok()
                            .and_then(|s| s.canonical_reason())
                        {
                            Some(reason) => format!("{code} {reason}"),
                            None => code.to_string(),
                        };
                        let elapsed = flow
                            .finished_at
                            .and_then(|fin| fin.duration_since(flow.started).ok())
                            .map(format_duration);
                        let res_bytes = flow.res_bytes();
                        let size = (res_bytes > 0).then(|| format_count(res_bytes as u64));
                        let meta = match (elapsed, size) {
                            (Some(e), Some(s)) => format!(" ({e}, {s})"),
                            (Some(e), None) => format!(" ({e})"),
                            (None, Some(s)) => format!(" ({s})"),
                            (None, None) => String::new(),
                        };
                        lines.push(format!("    {alias}-->>-C: {status_text}{path_hint}{meta}"));
                        (format!("{status_text}{raw_path_hint}{meta}"), code >= 400)
                    }
                    None if flow.error.is_some() => {
                        let reason = sanitize_mermaid_text(flow.error.as_deref().unwrap_or(""));
                        if reason.is_empty() {
                            lines.push(format!("    {alias}--x-C: ERR{path_hint}"));
                            (format!("ERR{raw_path_hint}"), true)
                        } else {
                            lines.push(format!("    {alias}--x-C: ERR{path_hint} ({reason})"));
                            (format!("ERR{raw_path_hint} ({reason})"), true)
                        }
                    }
                    None => {
                        lines.push(format!("    {alias}-->>-C: ...{path_hint}"));
                        (format!("...{raw_path_hint}"), false)
                    }
                };
                steps.push(SequenceStep {
                    step_no,
                    flow_id: flow.id,
                    method: method_raw.to_string(),
                    host_idx,
                    is_request: false,
                    is_error,
                    label,
                    note: String::new(),
                    active_hosts,
                });
            }
        }
    }

    SequenceDiagramData {
        hosts: participants.into_iter().map(|(h, _)| h).collect(),
        steps,
        bars,
        stages,
        hard_dep_count,
        mermaid: lines.join("\n"),
        flowchart_mermaid,
    }
}

#[derive(Clone)]
struct ProducedToken {
    path: String,
    key: String,
    value: String,
}

#[derive(Clone, Default)]
struct ProducedArtifacts {
    redirect_location: Option<String>,
    cookies: Vec<(String, String)>,
    tokens: Vec<ProducedToken>,
}

fn is_id_or_token_key(key: &str) -> bool {
    let k = key.trim().to_ascii_lowercase();
    if k.is_empty() {
        return false;
    }
    matches!(
        k.as_str(),
        "id" | "uid"
            | "uuid"
            | "guid"
            | "token"
            | "access_token"
            | "refresh_token"
            | "id_token"
            | "auth_token"
            | "ticket"
            | "code"
            | "key"
            | "secret"
            | "nonce"
            | "cursor"
            | "next_cursor"
            | "page_token"
            | "session"
            | "session_id"
            | "sid"
            | "sn"
            | "seq"
            | "order_no"
            | "trace_id"
            | "request_id"
    ) || k.ends_with("_id")
        || k.ends_with("id") && k.len() >= 3
        || k.ends_with("_token")
        || k.ends_with("token")
        || k.ends_with("_code")
        || k.ends_with("_key")
        || k.ends_with("_no")
        || k.ends_with("_sn")
}

fn is_trivial_scalar(val: &str) -> bool {
    let lower = val.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "" | "true"
            | "false"
            | "null"
            | "none"
            | "nil"
            | "ok"
            | "success"
            | "error"
            | "fail"
            | "failed"
            | "pending"
            | "get"
            | "post"
            | "put"
            | "delete"
            | "patch"
            | "head"
            | "options"
            | "utf-8"
            | "gzip"
            | "br"
            | "deflate"
            | "keep-alive"
            | "close"
            | "zh"
            | "en"
            | "zh-cn"
            | "en-us"
            | "0"
            | "1"
            | "-1"
            | "200"
            | "201"
            | "204"
            | "301"
            | "302"
            | "304"
            | "400"
            | "401"
            | "403"
            | "404"
            | "500"
    )
}

fn keys_compatible(src_key: &str, dst_key: &str) -> bool {
    let a = src_key.trim().to_ascii_lowercase();
    let b = dst_key.trim().to_ascii_lowercase();
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b || a.ends_with(&b) || b.ends_with(&a) {
        return true;
    }
    is_id_or_token_key(&a) && is_id_or_token_key(&b)
}

fn collect_json_scalars(
    val: &serde_json::Value,
    prefix: &str,
    last_key: &str,
    depth: usize,
    out: &mut Vec<ProducedToken>,
) {
    if depth > 4 || out.len() >= 64 {
        return;
    }
    match val {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let next_prefix = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                collect_json_scalars(v, &next_prefix, k, depth + 1, out);
                if out.len() >= 64 {
                    break;
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for (idx, v) in arr.iter().take(6).enumerate() {
                let next_prefix = format!("{prefix}[{idx}]");
                collect_json_scalars(v, &next_prefix, last_key, depth + 1, out);
                if out.len() >= 64 {
                    break;
                }
            }
        }
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.len() >= 3
                && trimmed.len() <= 180
                && !trimmed.contains(char::is_whitespace)
                && !trimmed.starts_with("http://")
                && !trimmed.starts_with("https://")
                && !trimmed.starts_with('/')
                && !is_trivial_scalar(trimmed)
                && (trimmed.len() >= 6 || is_id_or_token_key(last_key))
                && !out.iter().any(|t| t.value == trimmed)
            {
                out.push(ProducedToken {
                    path: if prefix.is_empty() {
                        "body".to_string()
                    } else {
                        prefix.to_string()
                    },
                    key: last_key.to_string(),
                    value: trimmed.to_string(),
                });
            }
        }
        serde_json::Value::Number(n) => {
            let s = n.to_string();
            if s.len() >= 2
                && s.len() <= 32
                && !s.contains('.')
                && !is_trivial_scalar(&s)
                && is_id_or_token_key(last_key)
                && !out.iter().any(|t| t.value == s)
            {
                out.push(ProducedToken {
                    path: if prefix.is_empty() {
                        "body".to_string()
                    } else {
                        prefix.to_string()
                    },
                    key: last_key.to_string(),
                    value: s,
                });
            }
        }
        _ => {}
    }
}

fn extract_produced_artifacts(flow: &Flow) -> ProducedArtifacts {
    let mut out = ProducedArtifacts::default();
    if matches!(flow.status, Some(301 | 302 | 303 | 307 | 308)) {
        for (name, val) in &flow.response_headers {
            if name.eq_ignore_ascii_case("location") && !val.trim().is_empty() {
                out.redirect_location = Some(val.trim().to_string());
                break;
            }
        }
    }
    for (name, val) in &flow.response_headers {
        if name.eq_ignore_ascii_case("set-cookie") {
            let pair = val.split(';').next().unwrap_or("").trim();
            if let Some((ck_name, ck_val)) = pair.split_once('=') {
                let ck_name = ck_name.trim();
                let ck_val = ck_val.trim();
                if !ck_name.is_empty() && ck_val.len() >= 3 && !is_trivial_scalar(ck_val) {
                    out.cookies.push((ck_name.to_string(), ck_val.to_string()));
                }
            }
        } else if matches!(
            name.to_ascii_lowercase().as_str(),
            "x-auth-token"
                | "x-csrf-token"
                | "x-xsrf-token"
                | "etag"
                | "x-request-id"
                | "x-trace-id"
                | "authorization"
        ) {
            let trimmed = val.trim().trim_matches('"');
            if trimmed.len() >= 4 && !is_trivial_scalar(trimmed) {
                out.tokens.push(ProducedToken {
                    path: format!("Header:{name}"),
                    key: name.clone(),
                    value: trimmed.to_string(),
                });
            }
        }
    }
    let res_bytes = &flow.response_body;
    if !res_bytes.is_empty()
        && res_bytes.len() <= 256 * 1024
        && let Ok(text) = std::str::from_utf8(res_bytes)
        && looks_like_json(text)
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(text)
    {
        collect_json_scalars(&json, "", "", 0, &mut out.tokens);
    }
    out
}

fn is_ignored_req_header(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "host"
            | "user-agent"
            | "accept"
            | "accept-encoding"
            | "accept-language"
            | "connection"
            | "proxy-connection"
            | "content-length"
            | "content-type"
            | "origin"
            | "referer"
            | "cookie"
            | "cache-control"
            | "pragma"
            | "upgrade-insecure-requests"
            | "dnt"
            | "priority"
            | "te"
    ) || lower.starts_with("sec-")
}

fn analyze_flow_dependencies_and_stages(
    sorted: &[&Flow],
    bars: &mut [WaterfallBar],
) -> (Vec<DependencyStage>, usize, String) {
    let n = sorted.len();
    let produced: Vec<ProducedArtifacts> = sorted
        .iter()
        .map(|f| extract_produced_artifacts(f))
        .collect();

    let mut hard_dep_count = 0usize;

    for j in 1..n {
        let flow_j = sorted[j];
        let mut deps: Vec<FlowDependency> = Vec::new();

        // 1. Check Redirect Location from earlier 3xx responses
        for i in (0..j).rev() {
            if let Some(loc) = &produced[i].redirect_location
                && (flow_j.url == *loc
                    || flow_j.path == *loc
                    || (!flow_j.path.is_empty()
                        && flow_j.path != "/"
                        && loc.ends_with(&flow_j.path)))
            {
                deps.push(FlowDependency {
                    from_flow_id: sorted[i].id,
                    from_index: i,
                    kind: DepKind::Redirect,
                    label: "Location".to_string(),
                });
                break;
            }
        }

        // 2. Check Cookie dependencies (`Set-Cookie` -> `Cookie`)
        for (h_name, h_val) in &flow_j.request_headers {
            if !h_name.eq_ignore_ascii_case("cookie") {
                continue;
            }
            for part in h_val.split(';') {
                let Some((ck_name, ck_val)) = part.trim().split_once('=') else {
                    continue;
                };
                let ck_name = ck_name.trim();
                let ck_val = ck_val.trim();
                if ck_name.is_empty() || ck_val.is_empty() {
                    continue;
                }
                for i in (0..j).rev() {
                    if produced[i]
                        .cookies
                        .iter()
                        .any(|(k, v)| k.eq_ignore_ascii_case(ck_name) && v == ck_val)
                    {
                        let label = format!("Set-Cookie: {ck_name}");
                        if !deps.iter().any(|d| d.from_index == i && d.label == label) {
                            deps.push(FlowDependency {
                                from_flow_id: sorted[i].id,
                                from_index: i,
                                kind: DepKind::Cookie,
                                label,
                            });
                        }
                        break;
                    }
                }
            }
        }

        // 3. Check Request Headers (`Authorization`, `X-*`, `If-None-Match`, etc.)
        for (h_name, h_val) in &flow_j.request_headers {
            if is_ignored_req_header(h_name) {
                continue;
            }
            let h_trimmed = h_val.trim();
            if h_trimmed.len() < 4 {
                continue;
            }
            let bearer_stripped = h_trimmed
                .strip_prefix("Bearer ")
                .or_else(|| h_trimmed.strip_prefix("bearer "))
                .map(str::trim)
                .unwrap_or(h_trimmed);
            for i in (0..j).rev() {
                if let Some(tok) = produced[i].tokens.iter().find(|t| {
                    t.value.len() >= 6 && (t.value == h_trimmed || t.value == bearer_stripped)
                }) {
                    let label = format!("{} → Header:{h_name}", tok.path);
                    if !deps.iter().any(|d| d.label == label) {
                        deps.push(FlowDependency {
                            from_flow_id: sorted[i].id,
                            from_index: i,
                            kind: DepKind::Token,
                            label,
                        });
                    }
                    break;
                }
            }
        }

        // 4. Check Query Parameters (`?key=val`)
        let raw_target = resolve_flow_target(flow_j, true);
        if let Some((_, q_str)) = raw_target.split_once('?') {
            let q_clean = q_str.split('#').next().unwrap_or(q_str);
            for (q_key, q_val) in form_pairs(q_clean) {
                let q_val = q_val.trim();
                if q_val.len() < 2 || is_trivial_scalar(q_val) {
                    continue;
                }
                for i in (0..j).rev() {
                    if let Some(tok) = produced[i].tokens.iter().find(|t| {
                        t.value == q_val && (q_val.len() >= 6 || keys_compatible(&t.key, &q_key))
                    }) {
                        let label = format!("{} → ?{q_key}", tok.path);
                        if !deps.iter().any(|d| d.label == label) {
                            deps.push(FlowDependency {
                                from_flow_id: sorted[i].id,
                                from_index: i,
                                kind: DepKind::QueryParam,
                                label,
                            });
                        }
                        break;
                    }
                }
            }
        }

        // 5. Check URL path segments & Request Body (JSON / Form)
        let path_only = raw_target.split(['?', '#']).next().unwrap_or(&raw_target);
        for seg in path_only.split('/').filter(|s| s.len() >= 4) {
            if is_trivial_scalar(seg) {
                continue;
            }
            for i in (0..j).rev() {
                if let Some(tok) = produced[i]
                    .tokens
                    .iter()
                    .find(|t| t.value == seg && (seg.len() >= 8 || is_id_or_token_key(&t.key)))
                {
                    let label = format!("{} → path:/{seg}", tok.path);
                    if !deps.iter().any(|d| d.label == label) {
                        deps.push(FlowDependency {
                            from_flow_id: sorted[i].id,
                            from_index: i,
                            kind: DepKind::BodyOrPath,
                            label,
                        });
                    }
                    break;
                }
            }
        }

        let req_bytes = &flow_j.request_body;
        if !req_bytes.is_empty()
            && req_bytes.len() <= 128 * 1024
            && let Ok(req_text) = std::str::from_utf8(req_bytes)
        {
            if looks_like_json(req_text)
                && let Ok(req_json) = serde_json::from_str::<serde_json::Value>(req_text)
            {
                let mut req_scalars = Vec::new();
                collect_json_scalars(&req_json, "", "", 0, &mut req_scalars);
                for req_tok in req_scalars {
                    for i in (0..j).rev() {
                        if let Some(tok) = produced[i].tokens.iter().find(|t| {
                            t.value == req_tok.value
                                && (req_tok.value.len() >= 6
                                    || keys_compatible(&t.key, &req_tok.key))
                        }) {
                            let label = format!("{} → body.{}", tok.path, req_tok.path);
                            if !deps.iter().any(|d| d.label == label) {
                                deps.push(FlowDependency {
                                    from_flow_id: sorted[i].id,
                                    from_index: i,
                                    kind: DepKind::BodyOrPath,
                                    label,
                                });
                            }
                            break;
                        }
                    }
                }
            } else if is_form(None, req_text) {
                for (f_key, f_val) in form_pairs(req_text) {
                    let f_val = f_val.trim();
                    if f_val.len() < 2 || is_trivial_scalar(f_val) {
                        continue;
                    }
                    for i in (0..j).rev() {
                        if let Some(tok) = produced[i].tokens.iter().find(|t| {
                            t.value == f_val
                                && (f_val.len() >= 6 || keys_compatible(&t.key, &f_key))
                        }) {
                            let label = format!("{} → form.{f_key}", tok.path);
                            if !deps.iter().any(|d| d.label == label) {
                                deps.push(FlowDependency {
                                    from_flow_id: sorted[i].id,
                                    from_index: i,
                                    kind: DepKind::BodyOrPath,
                                    label,
                                });
                            }
                            break;
                        }
                    }
                }
            }
        }

        deps.truncate(4);
        hard_dep_count += deps.len();
        bars[j].deps = deps;
    }

    // Group flows into timeline Stages (concurrent bursts vs sequential steps)
    let mut stage_of = vec![0usize; n];
    let mut cur_stage = 0usize;
    let mut stage_start_ms = bars[0].start_ms;

    for j in 1..n {
        let min_from_hard = bars[j]
            .deps
            .iter()
            .map(|d| stage_of[d.from_index] + 1)
            .max()
            .unwrap_or(0);

        let waited_for_current_stage = (0..j).any(|i| {
            stage_of[i] == cur_stage
                && sorted[i].finished_at.is_some()
                && bars[j].start_ms >= bars[i].end_ms - 1.0
                && bars[j].start_ms - stage_start_ms > 20.0
        });

        let next_stage = if min_from_hard > cur_stage {
            min_from_hard
        } else if waited_for_current_stage {
            cur_stage + 1
        } else {
            cur_stage
        };

        if next_stage != cur_stage {
            cur_stage = next_stage;
            stage_start_ms = bars[j].start_ms;
        }
        stage_of[j] = cur_stage;
    }

    // Normalize stage numbers to contiguous 1..=K and attach sequential timing predecessors
    // for flows in stage > 1 that have no hard data dependencies.
    let mut unique_stages: Vec<usize> = stage_of.clone();
    unique_stages.sort_unstable();
    unique_stages.dedup();

    let mut stages: Vec<DependencyStage> = Vec::with_capacity(unique_stages.len());
    for (norm_idx, &raw_stage) in unique_stages.iter().enumerate() {
        let stage_no = norm_idx + 1;
        let mut bar_indices = Vec::new();
        let mut s_min = f64::INFINITY;
        let mut s_max = 0.0_f64;
        for (j, &st) in stage_of.iter().enumerate() {
            if st == raw_stage {
                bars[j].stage = stage_no;
                bar_indices.push(j);
                s_min = s_min.min(bars[j].start_ms);
                s_max = s_max.max(bars[j].end_ms);
            }
        }
        let s_min = if s_min.is_finite() { s_min } else { 0.0 };
        let span_ms = (s_max - s_min).max(0.0);
        stages.push(DependencyStage {
            stage_no,
            start_ms: s_min,
            end_ms: s_max,
            offset_label: format!("+{}", format_delta(Duration::from_secs_f64(s_min / 1000.0))),
            span_label: format_duration(Duration::from_secs_f64(span_ms / 1000.0)),
            bar_indices,
        });
    }

    for j in 1..n {
        if bars[j].deps.is_empty() && bars[j].stage > 1 {
            let prev_stage = bars[j].stage - 1;
            let pred = (0..j)
                .filter(|&i| {
                    bars[i].stage == prev_stage && bars[i].end_ms <= bars[j].start_ms + 2.0
                })
                .max_by(|&a, &b| bars[a].end_ms.total_cmp(&bars[b].end_ms))
                .or_else(|| {
                    (0..j)
                        .filter(|&i| bars[i].stage == prev_stage)
                        .max_by(|&a, &b| bars[a].end_ms.total_cmp(&bars[b].end_ms))
                });
            if let Some(i) = pred {
                let wait_ms = (bars[j].start_ms - bars[i].end_ms).max(0.0);
                bars[j].deps.push(FlowDependency {
                    from_flow_id: bars[i].flow_id,
                    from_index: i,
                    kind: DepKind::Sequential,
                    label: format!("+{wait_ms:.0} ms"),
                });
            }
        }
    }

    // Compute Critical Path across the DAG
    let mut best_score = vec![0.0_f64; n];
    let mut best_parent = vec![None::<usize>; n];
    for j in 0..n {
        let own_dur = (bars[j].end_ms - bars[j].start_ms).max(1.0);
        let mut parent = None;
        let mut max_prev = 0.0_f64;
        for dep in &bars[j].deps {
            let candidate = best_score[dep.from_index];
            if parent.is_none() || candidate > max_prev {
                max_prev = candidate;
                parent = Some(dep.from_index);
            }
        }
        best_score[j] = max_prev + own_dur;
        best_parent[j] = parent;
    }
    if let Some((last_idx, _)) = bars.iter().enumerate().max_by(|(ia, a), (ib, b)| {
        a.end_ms
            .total_cmp(&b.end_ms)
            .then_with(|| best_score[*ia].total_cmp(&best_score[*ib]))
    }) {
        let mut cur = Some(last_idx);
        while let Some(idx) = cur {
            bars[idx].on_critical_path = true;
            cur = best_parent[idx];
        }
    }

    // Build Mermaid `flowchart TD`
    let mut fc = vec!["flowchart TD".to_string()];
    for stage in &stages {
        let title = if stage.bar_indices.len() > 1 {
            format!(
                "Stage {} ({}, 并发 x{})",
                stage.stage_no,
                stage.offset_label,
                stage.bar_indices.len()
            )
        } else {
            format!("Stage {} ({})", stage.stage_no, stage.offset_label)
        };
        fc.push(format!("    subgraph S{}[\"{}\"]", stage.stage_no, title));
        for &idx in &stage.bar_indices {
            let b = &bars[idx];
            let st = match b.status {
                Some(code) => code.to_string(),
                None if b.failed => "ERR".to_string(),
                None => "...".to_string(),
            };
            let clean_target = sanitize_mermaid_text(&b.target).replace('"', "'");
            fc.push(format!(
                "        F{}[\"#{} {} {} ({} · {})\"]",
                b.flow_id, b.flow_id, b.method, clean_target, st, b.duration_label
            ));
        }
        fc.push("    end".to_string());
    }
    for b in bars.iter() {
        for dep in &b.deps {
            let clean_label = sanitize_mermaid_text(&dep.label).replace('"', "'");
            if dep.kind.is_hard() {
                fc.push(format!(
                    "    F{} -->|\"{}\"| F{}",
                    dep.from_flow_id, clean_label, b.flow_id
                ));
            } else {
                fc.push(format!(
                    "    F{} -.->|\"{}\"| F{}",
                    dep.from_flow_id, clean_label, b.flow_id
                ));
            }
        }
    }

    (stages, hard_dep_count, fc.join("\n"))
}

/// Builds a Mermaid `sequenceDiagram` from the given flows, interleaving
/// request start and response finish timestamps along the real timeline so
/// concurrent/overlapping requests and lifeline activations (`+`/`-`) are visible.
#[cfg(test)]
pub fn mermaid_sequence_diagram(
    flows: &[&Flow],
    include_response: bool,
    include_query: bool,
) -> String {
    build_sequence_diagram_data(flows, include_response, include_query).mermaid
}

/// Wraps a Mermaid diagram string in a fenced Markdown code block.
pub fn mermaid_markdown_block(diagram: &str) -> String {
    format!("```mermaid\n{}\n```", diagram.trim_end())
}

pub(crate) fn resolve_flow_host(flow: &Flow) -> String {
    let trimmed = flow.host.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    if let Some((_, rest)) = flow.url.split_once("://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("").trim();
        let host = authority
            .rsplit_once('@')
            .map(|(_, h)| h)
            .unwrap_or(authority)
            .trim();
        if !host.is_empty() {
            return host.to_string();
        }
    } else {
        let raw = flow.url.trim();
        if !raw.is_empty() && !raw.starts_with('/') {
            let host = raw.split(['/', '?', '#']).next().unwrap_or("").trim();
            if !host.is_empty() {
                return host.to_string();
            }
        }
    }
    "unknown".to_string()
}

pub(crate) fn resolve_flow_target(flow: &Flow, include_query: bool) -> String {
    let raw: String = if !flow.path.trim().is_empty() {
        flow.path.trim().to_string()
    } else if let Some((_, rest)) = flow.url.split_once("://") {
        if let Some(idx) = rest.find('/') {
            rest[idx..].to_string()
        } else if let Some(idx) = rest.find(['?', '#']) {
            format!("/{}", &rest[idx..])
        } else {
            "/".to_string()
        }
    } else {
        "/".to_string()
    };
    let sliced = if include_query {
        raw.as_str()
    } else {
        raw.split(['?', '#']).next().unwrap_or(&raw)
    };
    if sliced.is_empty() {
        "/".to_string()
    } else if sliced.starts_with('/') || sliced.starts_with(':') {
        sliced.to_string()
    } else {
        format!("/{sliced}")
    }
}

fn format_mermaid_participant(host: &str) -> String {
    let clean = sanitize_mermaid_text(&host.replace('"', "'"));
    let clean = if clean.is_empty() {
        "unknown".to_string()
    } else {
        clean
    };
    let simple = clean
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'));
    if simple {
        clean
    } else {
        format!("\"{clean}\"")
    }
}

fn sanitize_mermaid_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_space = false;
    for ch in text.chars() {
        match ch {
            ';' => {
                out.push_str("#59;");
                prev_space = false;
            }
            '#' => {
                out.push_str("#35;");
                prev_space = false;
            }
            '\r' | '\n' | '\t' | ' ' => {
                if !prev_space && !out.is_empty() {
                    out.push(' ');
                    prev_space = true;
                }
            }
            c if c.is_control() => {}
            c => {
                out.push(c);
                prev_space = false;
            }
        }
    }
    out.trim().to_string()
}

fn push_section(out: &mut String, title: &str, body: &str) {
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(title);
    out.push('\n');
    out.push_str(body);
}

fn header_block(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn body_text(body: &[u8]) -> String {
    match std::str::from_utf8(body) {
        Ok(text) => text.to_string(),
        Err(_) => format!("<{len} binary bytes>", len = body.len()),
    }
}

fn copy_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(name, _)| {
            !name.eq_ignore_ascii_case("host")
                && !name.eq_ignore_ascii_case("content-length")
                && !name.eq_ignore_ascii_case("connection")
                && !name.eq_ignore_ascii_case("proxy-connection")
                && !name.eq_ignore_ascii_case("proxy-authorization")
        })
        .cloned()
        .collect()
}

fn js_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}

pub fn rust_string(text: &str) -> String {
    format!("{text:?}")
}

fn py_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\x7f' => {
                use std::fmt::Write;
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[derive(Default)]
struct TsGen {
    blocks: Vec<String>,
    used: Vec<String>,
}

impl TsGen {
    fn emit(&mut self, name: &str, value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::Object(map) => {
                let iface = self.fresh(name);
                let mut fields = String::new();
                for (key, child) in map {
                    let child_name = pascal(key);
                    let ty = self.emit(&child_name, child);
                    fields.push_str(&format!("  {}: {ty};\n", ts_key(key)));
                }
                self.blocks
                    .push(format!("export interface {iface} {{\n{fields}}}"));
                iface
            }
            serde_json::Value::Array(items) => {
                let inner = items
                    .first()
                    .map(|item| self.emit(name, item))
                    .unwrap_or_else(|| "unknown".into());
                format!("{inner}[]")
            }
            serde_json::Value::String(_) => "string".into(),
            serde_json::Value::Number(_) => "number".into(),
            serde_json::Value::Bool(_) => "boolean".into(),
            serde_json::Value::Null => "null".into(),
        }
    }

    fn fresh(&mut self, name: &str) -> String {
        let base = if name.is_empty() { "Value" } else { name };
        let mut candidate = base.to_string();
        let mut n = 2;
        while self.used.iter().any(|used| used == &candidate) {
            candidate = format!("{base}{n}");
            n += 1;
        }
        self.used.push(candidate.clone());
        candidate
    }
}

fn pascal(key: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for ch in key.chars() {
        if ch.is_ascii_alphanumeric() {
            if upper {
                out.extend(ch.to_uppercase());
                upper = false;
            } else {
                out.push(ch);
            }
        } else {
            upper = true;
        }
    }
    if out.is_empty() {
        "Value".into()
    } else if out.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        format!("N{out}")
    } else {
        out
    }
}

fn ts_key(key: &str) -> String {
    if !key.is_empty()
        && key
            .chars()
            .enumerate()
            .all(|(index, ch)| ch.is_ascii_alphanumeric() || ch == '_' || (ch == '$' && index == 0))
        && !key.chars().next().is_some_and(|ch| ch.is_ascii_digit())
    {
        key.to_string()
    } else {
        js_string(key)
    }
}

#[derive(Default)]
struct RustStructGen {
    blocks: Vec<String>,
    used: Vec<String>,
}

impl RustStructGen {
    fn emit(&mut self, name: &str, value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::Object(map) => {
                let struct_name = self.fresh(name);
                let mut fields = String::new();
                let mut seen_fields: Vec<String> = Vec::new();
                for (key, child) in map {
                    let child_name = pascal(key);
                    let ty = self.emit(&child_name, child);
                    let (mut ident, mut needs_rename) = rust_field_ident(key);
                    let base_ident = ident.trim_start_matches("r#").to_string();
                    let mut suffix = 2;
                    while seen_fields.contains(&ident) {
                        ident = format!("{base_ident}_{suffix}");
                        needs_rename = true;
                        suffix += 1;
                    }
                    seen_fields.push(ident.clone());
                    if needs_rename {
                        fields.push_str(&format!("    #[serde(rename = {})]\n", rust_string(key)));
                    }
                    fields.push_str(&format!("    pub {ident}: {ty},\n"));
                }
                self.blocks.push(format!(
                    "#[derive(Debug, Clone, Serialize, Deserialize)]\npub struct {struct_name} {{\n{fields}}}"
                ));
                struct_name
            }
            serde_json::Value::Array(items) => {
                let inner = items
                    .first()
                    .map(|item| self.emit(name, item))
                    .unwrap_or_else(|| "serde_json::Value".into());
                format!("Vec<{inner}>")
            }
            serde_json::Value::String(_) => "String".into(),
            serde_json::Value::Number(num) => {
                if num.is_i64() {
                    "i64".into()
                } else if num.is_u64() {
                    "u64".into()
                } else {
                    "f64".into()
                }
            }
            serde_json::Value::Bool(_) => "bool".into(),
            serde_json::Value::Null => "Option<serde_json::Value>".into(),
        }
    }

    fn fresh(&mut self, name: &str) -> String {
        const RESERVED_TYPES: &[&str] = &[
            "String",
            "Vec",
            "Option",
            "Result",
            "Box",
            "Serialize",
            "Deserialize",
            "Self",
            "Copy",
            "Clone",
            "Debug",
            "Default",
            "Drop",
            "Send",
            "Sync",
            "Sized",
            "ToOwned",
            "ToString",
            "Fn",
            "FnMut",
            "FnOnce",
        ];
        let raw_base = if name.is_empty() { "Value" } else { name };
        let base_owned;
        let base = if RESERVED_TYPES.contains(&raw_base) {
            base_owned = format!("{raw_base}Value");
            base_owned.as_str()
        } else {
            raw_base
        };
        let mut candidate = base.to_string();
        let mut n = 2;
        while self.used.iter().any(|used| used == &candidate) {
            candidate = format!("{base}{n}");
            n += 1;
        }
        self.used.push(candidate.clone());
        candidate
    }
}

fn rust_field_ident(key: &str) -> (String, bool) {
    let mut snake = String::new();
    let mut prev_lower_or_digit = false;
    for ch in key.chars() {
        if ch.is_ascii_alphanumeric() {
            if ch.is_ascii_uppercase() {
                if prev_lower_or_digit && !snake.ends_with('_') {
                    snake.push('_');
                }
                snake.push(ch.to_ascii_lowercase());
                prev_lower_or_digit = false;
            } else {
                snake.push(ch);
                prev_lower_or_digit = true;
            }
        } else {
            if !snake.is_empty() && !snake.ends_with('_') {
                snake.push('_');
            }
            prev_lower_or_digit = false;
        }
    }
    let clean = snake.trim_matches('_').to_string();
    let mut ident = if clean.is_empty() {
        "field".to_string()
    } else if clean.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        format!("f_{clean}")
    } else {
        clean
    };
    const RUST_KEYWORDS: &[&str] = &[
        "as", "async", "await", "box", "break", "const", "continue", "crate", "dyn", "else",
        "enum", "extern", "false", "fn", "for", "gen", "if", "impl", "in", "let", "loop", "macro",
        "match", "mod", "move", "mut", "priv", "pub", "ref", "return", "self", "Self", "static",
        "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "yield",
    ];
    let raw_needed = RUST_KEYWORDS.contains(&ident.as_str());
    let rename_needed = ident != key;
    if raw_needed {
        if matches!(ident.as_str(), "self" | "Self" | "super" | "crate") {
            ident.push('_');
            return (ident, true);
        }
        ident = format!("r#{ident}");
    }
    (ident, rename_needed)
}

fn decode_ansi_c_segment(rest: &str) -> (String, usize) {
    let bytes = rest.as_bytes();
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let ch = rest[i..].chars().next().unwrap_or('\0');
        let ch_len = ch.len_utf8();
        if ch == '\'' {
            return (String::from_utf8_lossy(&out).into_owned(), i + 1);
        }
        if ch != '\\' {
            out.extend_from_slice(&bytes[i..i + ch_len]);
            i += ch_len;
            continue;
        }
        i += 1;
        if i >= bytes.len() {
            out.push(b'\\');
            break;
        }
        let esc = rest[i..].chars().next().unwrap_or('\0');
        let esc_len = esc.len_utf8();
        i += esc_len;
        match esc {
            'a' => out.push(0x07),
            'b' => out.push(0x08),
            'e' | 'E' => out.push(0x1b),
            'f' => out.push(0x0c),
            'n' => out.push(b'\n'),
            'r' => out.push(b'\r'),
            't' => out.push(b'\t'),
            'v' => out.push(0x0b),
            '\\' => out.push(b'\\'),
            '\'' => out.push(b'\''),
            '"' => out.push(b'"'),
            '?' => out.push(b'?'),
            'x' => {
                let mut hex_len = 0;
                while hex_len < 2
                    && i + hex_len < bytes.len()
                    && bytes[i + hex_len].is_ascii_hexdigit()
                {
                    hex_len += 1;
                }
                if hex_len > 0
                    && let Ok(val) = u8::from_str_radix(&rest[i..i + hex_len], 16)
                {
                    out.push(val);
                    i += hex_len;
                } else {
                    out.extend_from_slice(b"\\x");
                }
            }
            'u' | 'U' => {
                let max_digits = if esc == 'u' { 4 } else { 8 };
                let mut hex_len = 0;
                while hex_len < max_digits
                    && i + hex_len < bytes.len()
                    && bytes[i + hex_len].is_ascii_hexdigit()
                {
                    hex_len += 1;
                }
                if hex_len > 0
                    && let Ok(code) = u32::from_str_radix(&rest[i..i + hex_len], 16)
                    && let Some(decoded) = char::from_u32(code)
                {
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(decoded.encode_utf8(&mut buf).as_bytes());
                    i += hex_len;
                } else {
                    out.push(b'\\');
                    out.push(esc as u8);
                }
            }
            '0'..='7' => {
                let mut oct = (esc as u8 - b'0') as u32;
                let mut count = 1;
                while count < 3 && i < bytes.len() && matches!(bytes[i], b'0'..=b'7') {
                    oct = oct * 8 + (bytes[i] - b'0') as u32;
                    i += 1;
                    count += 1;
                }
                out.push((oct & 0xff) as u8);
            }
            other => {
                out.push(b'\\');
                let mut buf = [0u8; 4];
                out.extend_from_slice(other.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    (String::from_utf8_lossy(&out).into_owned(), bytes.len())
}

fn shell_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < input.len() {
        let rest = &input[i..];
        let ch = rest.chars().next().unwrap_or('\0');
        let ch_len = ch.len_utf8();
        if let Some(mark) = quote {
            i += ch_len;
            if ch == mark {
                quote = None;
            } else if ch == '\\' && mark == '"' {
                let after = &input[i..];
                if after.starts_with("\r\n") {
                    i += 2;
                } else if after.starts_with('\n') {
                    i += 1;
                } else if let Some(next) = after.chars().next() {
                    current.push(next);
                    i += next.len_utf8();
                }
            } else {
                current.push(ch);
            }
            continue;
        }
        if rest.starts_with("$'") {
            i += 2;
            in_token = true;
            let (decoded, consumed) = decode_ansi_c_segment(&input[i..]);
            current.push_str(&decoded);
            i += consumed;
            continue;
        }
        i += ch_len;
        match ch {
            '\'' | '"' => {
                in_token = true;
                quote = Some(ch);
            }
            '\\' => {
                let after = &input[i..];
                if after.starts_with("\r\n") {
                    i += 2;
                } else if after.starts_with('\n') {
                    i += 1;
                } else if let Some(next) = after.chars().next() {
                    in_token = true;
                    current.push(next);
                    i += next.len_utf8();
                }
            }
            c if c.is_whitespace() => {
                if in_token {
                    tokens.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            _ => {
                in_token = true;
                current.push(ch);
            }
        }
    }
    if in_token {
        tokens.push(current);
    }
    tokens
}

fn split_flag(token: &str) -> (&str, Option<&str>) {
    const FLAGS: &[&str] = &[
        "--connect-timeout",
        "--data-urlencode",
        "--data-binary",
        "--data-raw",
        "--data-ascii",
        "--user-agent",
        "--write-out",
        "--max-time",
        "--request",
        "--header",
        "--referer",
        "--cookie",
        "--output",
        "--proxy",
        "--data",
        "--json",
        "--url",
        "--user",
        "-X",
        "-H",
        "-d",
        "-A",
        "-e",
        "-b",
        "-u",
        "-x",
        "-o",
        "-m",
        "-w",
    ];
    for flag in FLAGS {
        if token == *flag {
            return (flag, None);
        }
        if let Some(rest) = token.strip_prefix(flag) {
            if flag.starts_with("--") {
                if let Some(rest) = rest.strip_prefix('=') {
                    return (flag, Some(rest));
                }
            } else if !rest.is_empty() && !rest.starts_with('-') {
                return (flag, Some(rest));
            }
        }
    }
    (token, None)
}

fn jwt_hint(name: &str, value: &str) -> Option<String> {
    let token = value.trim();
    let token = if name.eq_ignore_ascii_case("authorization") {
        token
            .strip_prefix("Bearer ")
            .or_else(|| token.strip_prefix("bearer "))?
    } else {
        token
    };
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    let _sig = parts.next()?;
    if parts.next().is_some() || payload.is_empty() {
        return None;
    }
    let bytes = base64url_decode(payload)?;
    let text = String::from_utf8(bytes).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let pretty = serde_json::to_string_pretty(&value).ok()?;
    let mut out = format!("JWT\n{pretty}");
    if let Some(exp) = value.get("exp").and_then(|item| item.as_i64()) {
        out.push_str(&format!("\nexp {}", format_unix_utc(exp)));
    }
    Some(out)
}

fn timestamp_hint(value: &str) -> Option<String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let (secs, label) = if value.len() == 13 {
        (value.parse::<i64>().ok()? / 1000, "unix ms")
    } else if value.len() == 10 {
        (value.parse::<i64>().ok()?, "unix")
    } else {
        return None;
    };
    let year = format_unix_utc(secs);
    if !year.starts_with("20") {
        return None;
    }
    Some(format!("{label} {year}"))
}

fn base64_hint(value: &str) -> Option<String> {
    if value.len() < 16 || !value.len().is_multiple_of(4) || value.contains('.') {
        return None;
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/' || byte == b'=')
    {
        return None;
    }
    let bytes = base64_decode(value)?;
    let text = String::from_utf8(bytes).ok()?;
    if text.len() < 8
        || text
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
    {
        return None;
    }
    Some(format!("base64 {text}"))
}

fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    let mut standard = text.replace('-', "+").replace('_', "/");
    while !standard.len().is_multiple_of(4) {
        standard.push('=');
    }
    base64_decode(&standard)
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    fn val(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let chunk: Vec<u8> = bytes[index..].iter().take(4).copied().collect();
        if chunk.len() < 4 {
            return None;
        }
        index += 4;
        let pads = chunk.iter().filter(|byte| **byte == b'=').count();
        if pads > 2 {
            return None;
        }
        let a = val(chunk[0])?;
        let b = val(chunk[1])?;
        let c = if chunk[2] == b'=' { 0 } else { val(chunk[2])? };
        let d = if chunk[3] == b'=' { 0 } else { val(chunk[3])? };
        out.push((a << 2) | (b >> 4));
        if pads < 2 {
            out.push((b << 4) | (c >> 2));
        }
        if pads < 1 {
            out.push((c << 6) | d);
        }
    }
    Some(out)
}

fn format_unix_utc(secs: i64) -> String {
    let tod = secs.rem_euclid(86_400);
    let hour = tod / 3600;
    let minute = (tod % 3600) / 60;
    let second = tod % 60;
    let z = secs.div_euclid(86_400) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {hour:02}:{minute:02}:{second:02} UTC")
}

pub fn format_clock(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs() as i64;
    let millis = duration.subsec_millis();
    let local = secs + local_offset_secs();
    let day = local.rem_euclid(86_400);
    let hour = day / 3600;
    let minute = (day % 3600) / 60;
    let second = day % 60;
    format!("{hour:02}:{minute:02}:{second:02}.{millis:03}")
}

/// Gap from the previous arrival. The first flow, and a clock that steps
/// backwards, both read as `0 ms`.
pub fn format_gap(previous: Option<SystemTime>, started: SystemTime) -> String {
    let Some(previous) = previous else {
        return "0 ms".into();
    };
    match started.duration_since(previous) {
        Ok(duration) => format_delta(duration),
        Err(_) => "0 ms".into(),
    }
}

fn format_delta(duration: Duration) -> String {
    let ms = duration.as_millis();
    if ms < 1_000 {
        return format!("{ms} ms");
    }
    if ms < 60_000 {
        return format_seconds(ms);
    }
    if ms < 3_600_000 {
        // Tenths of a minute. 60_000 ms is `1.0 m`. From 10 minutes the fraction is dropped.
        let tenths = ms.saturating_add(3_000) / 6_000;
        if tenths < 100 {
            return format!("{}.{} m", tenths / 10, tenths % 10);
        }
        return format!("{} m", tenths / 10);
    }
    let tenths = ms / 360_000;
    if tenths < 100 {
        format!("{}.{} h", tenths / 10, tenths % 10)
    } else {
        format!("{} h", tenths / 10)
    }
}

fn local_offset_secs() -> i64 {
    use std::sync::OnceLock;
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        #[cfg(target_arch = "wasm32")]
        {
            0
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Ok(output) = std::process::Command::new("date").arg("+%z").output() else {
                return 0;
            };
            let text = String::from_utf8_lossy(&output.stdout);
            let text = text.trim();
            let (sign, rest) = match text.as_bytes().first() {
                Some(b'+') => (1, &text[1..]),
                Some(b'-') => (-1, &text[1..]),
                _ => return 0,
            };
            if rest.len() < 4 {
                return 0;
            }
            let hours: i64 = rest[..2].parse().unwrap_or(0);
            let minutes: i64 = rest[2..4].parse().unwrap_or(0);
            sign * (hours * 3600 + minutes * 60)
        }
    })
}

/// From the request start until the full response is stored. An unfinished
/// flow has no total yet.
pub fn format_elapsed(started: SystemTime, finished: Option<SystemTime>) -> String {
    let Some(finished) = finished else {
        return "—".into();
    };
    match finished.duration_since(started) {
        Ok(duration) => format_duration(duration),
        Err(_) => "—".into(),
    }
}

fn format_duration(duration: Duration) -> String {
    let ms = duration.as_millis();
    if ms >= 1000 {
        format_seconds(ms)
    } else {
        format!("{ms} ms")
    }
}

/// One decimal second, half up. 1_240 ms is `1.2 s`, 1_250 ms is `1.3 s`.
fn format_seconds(ms: u128) -> String {
    let tenths = ms.saturating_add(50) / 100;
    format!("{}.{} s", tenths / 10, tenths % 10)
}

fn format_count(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        let mb = bytes as f64 / (1024.0 * 1024.0);
        if mb >= 99.95 {
            format!("{mb:.0} MB")
        } else {
            format!("{mb:.1} MB")
        }
    } else if bytes >= 1024 {
        let kb = bytes as f64 / 1024.0;
        if kb >= 99.95 {
            format!("{kb:.0} KB")
        } else {
            format!("{kb:.1} KB")
        }
    } else {
        format!("{bytes} B")
    }
}

fn looks_like_json(body: &str) -> bool {
    let trimmed = body.trim_start();
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

pub fn is_form(content_type: Option<&str>, body: &str) -> bool {
    let kind = content_type.unwrap_or("").to_ascii_lowercase();
    kind.contains("x-www-form-urlencoded")
        || (!kind.contains('/') && body.contains('=') && body.contains('&'))
}

pub fn form_pairs(body: &str) -> Vec<(String, String)> {
    body.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(key), percent_decode(value))
        })
        .collect()
}

/// Parses a `multipart/form-data` body into `(field_name, summary_or_value)` pairs
/// when the `Content-Type` header supplies a valid `boundary` parameter.
pub fn multipart_pairs(content_type: Option<&str>, body: &[u8]) -> Option<Vec<(String, String)>> {
    let ct = content_type?;
    if !ct.to_ascii_lowercase().contains("multipart/form-data") {
        return None;
    }
    let boundary = extract_multipart_boundary(ct)?;
    if boundary.is_empty() || body.is_empty() {
        return None;
    }
    let marker = format!("--{boundary}");
    let marker_bytes = marker.as_bytes();
    let mut parts = Vec::new();
    let mut cursor = 0usize;
    while let Some(pos) = find_subslice(&body[cursor..], marker_bytes) {
        let start = cursor + pos + marker_bytes.len();
        if body[start..].starts_with(b"--") {
            break;
        }
        let after_line = if body[start..].starts_with(b"\r\n") {
            start + 2
        } else if body[start..].starts_with(b"\n") {
            start + 1
        } else {
            cursor = start;
            continue;
        };
        let Some(next_rel) = find_subslice(&body[after_line..], marker_bytes) else {
            break;
        };
        let mut part_end = after_line + next_rel;
        if part_end >= after_line + 2 && &body[part_end - 2..part_end] == b"\r\n" {
            part_end -= 2;
        } else if part_end > after_line && body[part_end - 1] == b'\n' {
            part_end -= 1;
        }
        let part = &body[after_line..part_end];
        if let Some(pair) = parse_multipart_part(part) {
            parts.push(pair);
        }
        cursor = after_line + next_rel;
    }
    if parts.is_empty() { None } else { Some(parts) }
}

fn extract_multipart_boundary(content_type: &str) -> Option<String> {
    for segment in content_type.split(';').skip(1) {
        let segment = segment.trim();
        let Some((key, value)) = segment.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("boundary") {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn parse_multipart_part(part: &[u8]) -> Option<(String, String)> {
    let (head_bytes, payload) = if let Some(pos) = find_subslice(part, b"\r\n\r\n") {
        (&part[..pos], &part[pos + 4..])
    } else {
        let pos = find_subslice(part, b"\n\n")?;
        (&part[..pos], &part[pos + 2..])
    };
    let head_str = std::str::from_utf8(head_bytes).ok()?;
    let mut name = String::new();
    let mut filename: Option<String> = None;
    let mut part_ct: Option<String> = None;
    for line in head_str.lines() {
        let line = line.trim();
        let Some((h_name, h_val)) = line.split_once(':') else {
            continue;
        };
        let h_name = h_name.trim();
        let h_val = h_val.trim();
        if h_name.eq_ignore_ascii_case("content-disposition") {
            for param in h_val.split(';').skip(1) {
                let Some((k, v)) = param.trim().split_once('=') else {
                    continue;
                };
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                if k.eq_ignore_ascii_case("name") {
                    name = v.to_string();
                } else if k.eq_ignore_ascii_case("filename") {
                    filename = Some(v.to_string());
                }
            }
        } else if h_name.eq_ignore_ascii_case("content-type") && !h_val.is_empty() {
            part_ct = Some(h_val.to_string());
        }
    }
    if name.is_empty() {
        name = filename.clone().unwrap_or_else(|| "part".to_string());
    }
    let value = match (filename, std::str::from_utf8(payload)) {
        (Some(fname), Ok(text)) if payload.len() <= 256 && !text.contains('\0') => {
            let ct_note = part_ct.as_deref().unwrap_or("text/plain");
            if text.is_empty() {
                format!("[{fname} · {ct_note} · 0 B]")
            } else {
                format!(
                    "[{fname} · {ct_note} · {}] {text}",
                    format_count(payload.len() as u64)
                )
            }
        }
        (Some(fname), _) => {
            let ct_note = part_ct.as_deref().unwrap_or("application/octet-stream");
            format!(
                "[{fname} · {ct_note} · {}]",
                format_count(payload.len() as u64)
            )
        }
        (None, Ok(text)) => text.to_string(),
        (None, Err(_)) => format!(
            "[{} · {}]",
            part_ct.as_deref().unwrap_or("binary"),
            format_count(payload.len() as u64)
        ),
    };
    Some((name, value))
}

fn pretty_form(body: &str) -> String {
    form_pairs(body)
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn pretty_markup(body: &str) -> String {
    let mut out = String::new();
    let mut depth: i32 = 0;
    let mut at_line_start = true;
    for line in body.split_inclusive('>') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let closing = trimmed.starts_with("</");
        let declaration = trimmed.starts_with("<!") || trimmed.starts_with("<?");
        if closing {
            depth = depth.saturating_sub(1);
        }
        if !at_line_start {
            out.push('\n');
        }
        for _ in 0..depth.max(0) {
            out.push_str("  ");
        }
        out.push_str(trimmed);
        at_line_start = false;
        let self_closing = trimmed.ends_with("/>") || declaration;
        if !closing && !self_closing && trimmed.starts_with('<') {
            depth += 1;
        }
    }
    if out.is_empty() {
        body.to_string()
    } else {
        out
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &text[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    index += 3;
                } else {
                    out.push(b'%');
                    index += 1;
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out)
        .unwrap_or_else(|err| String::from_utf8_lossy(err.as_bytes()).into_owned())
}

/// Formats a GraphQL-over-JSON request payload (`{"query": "...", "variables": {...}}`)
/// into a readable multi-line GraphQL block followed by pretty-printed variables.
pub fn pretty_graphql_json(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    let map = value.as_object()?;
    let query = map.get("query")?.as_str()?.trim();
    if !looks_like_graphql_query(query) {
        return None;
    }
    let only_gql_keys = map.keys().all(|k| {
        matches!(
            k.as_str(),
            "query" | "operationName" | "variables" | "extensions"
        )
    });
    if !only_gql_keys {
        return None;
    }
    let mut out = String::new();
    if let Some(op) = map
        .get("operationName")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push_str("# operationName: ");
        out.push_str(op);
        out.push('\n');
    }
    out.push_str(&format_graphql_query(query));
    if let Some(vars) = map.get("variables")
        && !vars.is_null()
        && vars.as_object().is_none_or(|o| !o.is_empty())
        && let Ok(pretty_vars) = serde_json::to_string_pretty(vars)
    {
        out.push_str("\n\n# variables\n");
        out.push_str(&pretty_vars);
    }
    if let Some(ext) = map.get("extensions")
        && !ext.is_null()
        && ext.as_object().is_none_or(|o| !o.is_empty())
        && let Ok(pretty_ext) = serde_json::to_string_pretty(ext)
    {
        out.push_str("\n\n# extensions\n");
        out.push_str(&pretty_ext);
    }
    Some(out)
}

/// Extracts GraphQL operation name(s) from a request's URL query string (GET)
/// or JSON body (POST single object or batched array).
///
/// Order of precedence per operation:
/// 1. Explicit `operationName` field/query param when non-empty.
/// 2. Named `query <Name>`, `mutation <Name>`, or `subscription <Name>` in `query`.
/// 3. `"anonymous"` when `query` is a valid anonymous GraphQL selection set.
pub fn graphql_operations(path: &str, body: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(text) = std::str::from_utf8(body) {
        let trimmed = text.trim();
        if (trimmed.starts_with('{') || trimmed.starts_with('['))
            && let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed)
        {
            match &val {
                serde_json::Value::Object(map) => {
                    if let Some(op) = graphql_op_from_map(map) {
                        out.push(op);
                    }
                }
                serde_json::Value::Array(arr) => {
                    for item in arr {
                        if let Some(map) = item.as_object()
                            && let Some(op) = graphql_op_from_map(map)
                            && !out.contains(&op)
                        {
                            out.push(op);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if out.is_empty() && path.contains('?') {
        let pairs = query_pairs(path);
        if let Some((_, op)) = pairs
            .iter()
            .find(|(k, v)| k == "operationName" && !v.trim().is_empty())
        {
            out.push(op.trim().to_string());
        } else if let Some((_, q)) = pairs.iter().find(|(k, _)| k == "query")
            && let Some(op) = infer_graphql_operation_from_query(q)
        {
            out.push(op);
        }
    }
    out
}

pub fn graphql_operation_label(path: &str, body: &[u8]) -> Option<String> {
    let ops = graphql_operations(path, body);
    (!ops.is_empty()).then(|| ops.join(", "))
}

fn graphql_op_from_map(map: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    let only_gql_keys = map.keys().all(|k| {
        matches!(
            k.as_str(),
            "query" | "operationName" | "variables" | "extensions"
        )
    });
    if !only_gql_keys {
        return None;
    }
    if let Some(op) = map
        .get("operationName")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(op.to_string());
    }
    let query = map.get("query")?.as_str()?;
    infer_graphql_operation_from_query(query)
}

fn infer_graphql_operation_from_query(query: &str) -> Option<String> {
    let mut rest = query;
    loop {
        rest = rest.trim_start();
        if let Some(after_hash) = rest.strip_prefix('#') {
            rest = after_hash.split_once('\n').map_or("", |(_, tail)| tail);
        } else {
            break;
        }
    }
    for kw in ["query", "mutation", "subscription"] {
        if let Some(after_kw) = rest.strip_prefix(kw)
            && after_kw.starts_with(|c: char| c.is_whitespace() || c == '{' || c == '(')
        {
            let after_ws = after_kw.trim_start();
            let ident: String = after_ws
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !ident.is_empty()
                && ident
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            {
                return Some(ident);
            }
            if looks_like_graphql_query(rest) {
                return Some("anonymous".into());
            }
        }
    }
    looks_like_graphql_query(rest).then(|| "anonymous".into())
}

fn looks_like_graphql_query(query: &str) -> bool {
    let trimmed = query.trim_start();
    if !trimmed.contains('{') || !trimmed.contains('}') {
        return false;
    }
    trimmed.starts_with('{')
        || trimmed.starts_with('#')
        || ["query", "mutation", "subscription", "fragment"]
            .iter()
            .any(|kw| {
                trimmed.strip_prefix(kw).is_some_and(|rest| {
                    rest.starts_with(|c: char| c.is_whitespace() || c == '{' || c == '(')
                })
            })
}

fn format_graphql_query(query: &str) -> String {
    let mut out = String::new();
    let mut depth: usize = 0;
    let mut paren_depth: usize = 0;
    let mut in_string = false;
    let mut escaped = false;
    let mut in_comment = false;
    let mut at_line_start = true;
    let mut pending_space = false;

    let push_indent = |buf: &mut String, d: usize| {
        for _ in 0..d {
            buf.push_str("  ");
        }
    };

    for ch in query.chars() {
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                out.push('\n');
                at_line_start = true;
                pending_space = false;
            } else if ch != '\r' {
                out.push(ch);
            }
            continue;
        }
        if in_string {
            if at_line_start {
                push_indent(&mut out, depth);
                at_line_start = false;
            } else if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => {
                if at_line_start {
                    push_indent(&mut out, depth);
                    at_line_start = false;
                } else if pending_space {
                    out.push(' ');
                    pending_space = false;
                }
                in_string = true;
                out.push('"');
            }
            '#' => {
                if at_line_start {
                    push_indent(&mut out, depth);
                    at_line_start = false;
                } else if pending_space {
                    out.push(' ');
                    pending_space = false;
                }
                in_comment = true;
                out.push('#');
            }
            '(' => {
                if at_line_start {
                    push_indent(&mut out, depth);
                    at_line_start = false;
                }
                pending_space = false;
                paren_depth += 1;
                out.push('(');
            }
            ')' => {
                paren_depth = paren_depth.saturating_sub(1);
                pending_space = false;
                out.push(')');
            }
            '{' if paren_depth == 0 => {
                if at_line_start {
                    push_indent(&mut out, depth);
                } else if !out.ends_with(' ') && !out.ends_with('\n') {
                    out.push(' ');
                }
                out.push_str("{\n");
                depth += 1;
                at_line_start = true;
                pending_space = false;
            }
            '}' if paren_depth == 0 => {
                depth = depth.saturating_sub(1);
                if !at_line_start {
                    out.push('\n');
                }
                push_indent(&mut out, depth);
                out.push_str("}\n");
                at_line_start = true;
                pending_space = false;
            }
            '\n' | ',' if paren_depth == 0 => {
                if !at_line_start {
                    out.push('\n');
                    at_line_start = true;
                }
                pending_space = false;
            }
            ',' => {
                out.push_str(", ");
                pending_space = false;
            }
            c if c.is_whitespace() => {
                if !at_line_start {
                    pending_space = true;
                }
            }
            c => {
                if at_line_start {
                    push_indent(&mut out, depth);
                    at_line_start = false;
                } else if pending_space {
                    out.push(' ');
                    pending_space = false;
                }
                out.push(c);
            }
        }
    }
    out.trim().to_string()
}

pub fn is_protobuf(content_type: Option<&str>) -> bool {
    let Some(ct) = content_type else {
        return false;
    };
    let lower = ct.to_ascii_lowercase();
    lower.contains("protobuf")
        || lower.contains("application/grpc")
        || lower.contains("x-google-protobuf")
}

/// Decodes a binary Protobuf or gRPC wire payload without requiring a `.proto` schema.
pub fn decode_protobuf_wire(content_type: Option<&str>, body: &[u8]) -> Option<String> {
    if !is_protobuf(content_type) || body.is_empty() {
        return None;
    }
    let lower = content_type.unwrap_or("").to_ascii_lowercase();
    let is_grpc = lower.contains("grpc");
    if is_grpc && body.len() >= 5 && (body[0] == 0 || body[0] == 1) {
        let mut cursor = 0usize;
        let mut frames = Vec::new();
        let mut valid_frames = true;
        while cursor + 5 <= body.len() {
            let compressed = body[cursor];
            if compressed > 1 {
                valid_frames = false;
                break;
            }
            let frame_len =
                u32::from_be_bytes(body[cursor + 1..cursor + 5].try_into().ok()?) as usize;
            if cursor + 5 + frame_len > body.len() {
                valid_frames = false;
                break;
            }
            let payload = &body[cursor + 5..cursor + 5 + frame_len];
            frames.push((compressed == 1, payload));
            cursor += 5 + frame_len;
        }
        if valid_frames && cursor == body.len() && !frames.is_empty() {
            let mut out = String::new();
            for (idx, (compressed, payload)) in frames.into_iter().enumerate() {
                if idx > 0 {
                    out.push_str("\n\n");
                }
                if compressed {
                    out.push_str(&format!(
                        "# gRPC frame #{} (compressed, {} B)",
                        idx + 1,
                        payload.len()
                    ));
                } else if payload.is_empty() {
                    out.push_str(&format!("# gRPC frame #{} (0 B)", idx + 1));
                } else {
                    let mut decoded = String::new();
                    if decode_proto_fields(payload, 0, &mut decoded) {
                        if idx > 0 {
                            out.push_str(&format!("# gRPC frame #{}\n", idx + 1));
                        }
                        out.push_str(decoded.trim_end());
                    } else {
                        return None;
                    }
                }
            }
            return (!out.is_empty()).then_some(out);
        }
    }
    let mut out = String::new();
    if decode_proto_fields(body, 0, &mut out) && !out.is_empty() {
        Some(out.trim_end().to_string())
    } else {
        None
    }
}

fn decode_proto_fields(bytes: &[u8], depth: usize, out: &mut String) -> bool {
    if depth > 6 || bytes.is_empty() {
        return false;
    }
    let mut cursor = 0usize;
    let mut count = 0usize;
    while cursor < bytes.len() {
        let Some((tag, next)) = read_varint(bytes, cursor) else {
            return false;
        };
        cursor = next;
        let field_num = tag >> 3;
        let wire_type = (tag & 0x07) as u8;
        if field_num == 0 || field_num > 536_870_911 {
            return false;
        }
        let indent = "  ".repeat(depth);
        match wire_type {
            0 => {
                let Some((val, next)) = read_varint(bytes, cursor) else {
                    return false;
                };
                cursor = next;
                out.push_str(&format!("{indent}field #{field_num} (varint): {val}\n"));
            }
            1 => {
                if cursor + 8 > bytes.len() {
                    return false;
                }
                let raw: [u8; 8] = bytes[cursor..cursor + 8].try_into().unwrap();
                cursor += 8;
                let u_val = u64::from_le_bytes(raw);
                out.push_str(&format!("{indent}field #{field_num} (fixed64): {u_val}\n"));
            }
            2 => {
                let Some((len_u64, next)) = read_varint(bytes, cursor) else {
                    return false;
                };
                let len = len_u64 as usize;
                cursor = next;
                if cursor + len > bytes.len() {
                    return false;
                }
                let sub = &bytes[cursor..cursor + len];
                cursor += len;
                if let Ok(s) = std::str::from_utf8(sub)
                    && !s.is_empty()
                    && s.chars()
                        .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
                {
                    out.push_str(&format!("{indent}field #{field_num} (string): {s:?}\n"));
                } else if !sub.is_empty() && depth < 5 {
                    let mut nested = String::new();
                    if decode_proto_fields(sub, depth + 1, &mut nested) && !nested.is_empty() {
                        out.push_str(&format!(
                            "{indent}field #{field_num} (message): {{\n{nested}{indent}}}\n"
                        ));
                    } else {
                        out.push_str(&format!(
                            "{indent}field #{field_num} (bytes, {len} B): {}\n",
                            hex_preview(sub, 24)
                        ));
                    }
                } else {
                    out.push_str(&format!(
                        "{indent}field #{field_num} (bytes, {len} B): {}\n",
                        hex_preview(sub, 24)
                    ));
                }
            }
            5 => {
                if cursor + 4 > bytes.len() {
                    return false;
                }
                let raw: [u8; 4] = bytes[cursor..cursor + 4].try_into().unwrap();
                cursor += 4;
                let u_val = u32::from_le_bytes(raw);
                out.push_str(&format!("{indent}field #{field_num} (fixed32): {u_val}\n"));
            }
            _ => return false,
        }
        count += 1;
        if out.len() > 256 * 1024 {
            break;
        }
    }
    count > 0 && cursor == bytes.len()
}

fn read_varint(bytes: &[u8], mut cursor: usize) -> Option<(u64, usize)> {
    let mut result = 0u64;
    let mut shift = 0u32;
    while cursor < bytes.len() {
        let b = bytes[cursor];
        cursor += 1;
        result |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some((result, cursor));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

fn hex_preview(bytes: &[u8], limit: usize) -> String {
    if bytes.is_empty() {
        return "\"\"".into();
    }
    let shown = bytes.len().min(limit);
    let mut hex = bytes[..shown]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    if bytes.len() > shown {
        hex.push_str(" …");
    }
    hex
}

pub fn is_msgpack(content_type: Option<&str>) -> bool {
    content_type.is_some_and(|ct| ct.to_ascii_lowercase().contains("msgpack"))
}

fn decode_msgpack_value(body: &[u8]) -> Option<serde_json::Value> {
    if body.is_empty() {
        return None;
    }
    let mut pos = 0usize;
    let val = read_msgpack_value(body, &mut pos, 0)?;
    (pos == body.len()).then_some(val)
}

/// Decodes a binary MessagePack payload into pretty-printed JSON.
pub fn decode_msgpack_to_json(body: &[u8]) -> Option<String> {
    let val = decode_msgpack_value(body)?;
    serde_json::to_string_pretty(&val).ok()
}

/// Parses a stored body payload (UTF-8 JSON/GraphQL-JSON or MessagePack) into a JSON value.
pub fn parse_json_body(content_type: Option<&str>, body: &[u8]) -> Option<serde_json::Value> {
    if body.is_empty() {
        return None;
    }
    if is_msgpack(content_type) {
        return decode_msgpack_value(body);
    }
    let text = std::str::from_utf8(body).ok()?;
    let kind = content_type.unwrap_or("").to_ascii_lowercase();
    if kind.contains("json") || kind.contains("graphql") || looks_like_json(text) {
        serde_json::from_str(text).ok()
    } else {
        None
    }
}

fn read_msgpack_value(bytes: &[u8], pos: &mut usize, depth: usize) -> Option<serde_json::Value> {
    if depth > 16 || *pos >= bytes.len() {
        return None;
    }
    let marker = bytes[*pos];
    *pos += 1;
    let take = |pos: &mut usize, n: usize| -> Option<&[u8]> {
        if *pos + n > bytes.len() {
            None
        } else {
            let slice = &bytes[*pos..*pos + n];
            *pos += n;
            Some(slice)
        }
    };
    match marker {
        0x00..=0x7f => Some(serde_json::Value::from(marker)),
        0x80..=0x8f => read_msgpack_map(bytes, pos, (marker & 0x0f) as usize, depth + 1),
        0x90..=0x9f => read_msgpack_array(bytes, pos, (marker & 0x0f) as usize, depth + 1),
        0xa0..=0xbf => {
            let s = take(pos, (marker & 0x1f) as usize)?;
            Some(serde_json::Value::String(
                String::from_utf8_lossy(s).into_owned(),
            ))
        }
        0xc0 => Some(serde_json::Value::Null),
        0xc2 => Some(serde_json::Value::Bool(false)),
        0xc3 => Some(serde_json::Value::Bool(true)),
        0xc4 => {
            let n = take(pos, 1)?[0] as usize;
            let b = take(pos, n)?;
            Some(serde_json::Value::String(format!(
                "<bin {n} B: {}>",
                hex_preview(b, 16)
            )))
        }
        0xc5 => {
            let n = u16::from_be_bytes(take(pos, 2)?.try_into().ok()?) as usize;
            let b = take(pos, n)?;
            Some(serde_json::Value::String(format!(
                "<bin {n} B: {}>",
                hex_preview(b, 16)
            )))
        }
        0xc6 => {
            let n = u32::from_be_bytes(take(pos, 4)?.try_into().ok()?) as usize;
            let b = take(pos, n)?;
            Some(serde_json::Value::String(format!(
                "<bin {n} B: {}>",
                hex_preview(b, 16)
            )))
        }
        0xca => {
            let f = f32::from_be_bytes(take(pos, 4)?.try_into().ok()?) as f64;
            serde_json::Number::from_f64(f).map(serde_json::Value::Number)
        }
        0xcb => {
            let f = f64::from_be_bytes(take(pos, 8)?.try_into().ok()?);
            serde_json::Number::from_f64(f).map(serde_json::Value::Number)
        }
        0xcc => Some(serde_json::Value::from(take(pos, 1)?[0])),
        0xcd => Some(serde_json::Value::from(u16::from_be_bytes(
            take(pos, 2)?.try_into().ok()?,
        ))),
        0xce => Some(serde_json::Value::from(u32::from_be_bytes(
            take(pos, 4)?.try_into().ok()?,
        ))),
        0xcf => Some(serde_json::Value::from(u64::from_be_bytes(
            take(pos, 8)?.try_into().ok()?,
        ))),
        0xd0 => Some(serde_json::Value::from(take(pos, 1)?[0] as i8)),
        0xd1 => Some(serde_json::Value::from(i16::from_be_bytes(
            take(pos, 2)?.try_into().ok()?,
        ))),
        0xd2 => Some(serde_json::Value::from(i32::from_be_bytes(
            take(pos, 4)?.try_into().ok()?,
        ))),
        0xd3 => Some(serde_json::Value::from(i64::from_be_bytes(
            take(pos, 8)?.try_into().ok()?,
        ))),
        0xd9 => {
            let n = take(pos, 1)?[0] as usize;
            let s = take(pos, n)?;
            Some(serde_json::Value::String(
                String::from_utf8_lossy(s).into_owned(),
            ))
        }
        0xda => {
            let n = u16::from_be_bytes(take(pos, 2)?.try_into().ok()?) as usize;
            let s = take(pos, n)?;
            Some(serde_json::Value::String(
                String::from_utf8_lossy(s).into_owned(),
            ))
        }
        0xdb => {
            let n = u32::from_be_bytes(take(pos, 4)?.try_into().ok()?) as usize;
            let s = take(pos, n)?;
            Some(serde_json::Value::String(
                String::from_utf8_lossy(s).into_owned(),
            ))
        }
        0xdc => {
            let n = u16::from_be_bytes(take(pos, 2)?.try_into().ok()?) as usize;
            read_msgpack_array(bytes, pos, n, depth + 1)
        }
        0xdd => {
            let n = u32::from_be_bytes(take(pos, 4)?.try_into().ok()?) as usize;
            read_msgpack_array(bytes, pos, n, depth + 1)
        }
        0xde => {
            let n = u16::from_be_bytes(take(pos, 2)?.try_into().ok()?) as usize;
            read_msgpack_map(bytes, pos, n, depth + 1)
        }
        0xdf => {
            let n = u32::from_be_bytes(take(pos, 4)?.try_into().ok()?) as usize;
            read_msgpack_map(bytes, pos, n, depth + 1)
        }
        0xe0..=0xff => Some(serde_json::Value::from(marker as i8)),
        _ => None,
    }
}

fn read_msgpack_array(
    bytes: &[u8],
    pos: &mut usize,
    len: usize,
    depth: usize,
) -> Option<serde_json::Value> {
    if len > 10_000 {
        return None;
    }
    let mut items = Vec::with_capacity(len.min(256));
    for _ in 0..len {
        items.push(read_msgpack_value(bytes, pos, depth)?);
    }
    Some(serde_json::Value::Array(items))
}

fn read_msgpack_map(
    bytes: &[u8],
    pos: &mut usize,
    len: usize,
    depth: usize,
) -> Option<serde_json::Value> {
    if len > 10_000 {
        return None;
    }
    let mut map = serde_json::Map::with_capacity(len.min(256));
    for _ in 0..len {
        let key_val = read_msgpack_value(bytes, pos, depth)?;
        let key = match key_val {
            serde_json::Value::String(s) => s,
            other => other.to_string(),
        };
        let val = read_msgpack_value(bytes, pos, depth)?;
        map.insert(key, val);
    }
    Some(serde_json::Value::Object(map))
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TimingBreakdown {
    pub dns_ms: Option<u64>,
    pub tcp_ms: Option<u64>,
    pub tls_ms: Option<u64>,
    pub wait_ms: Option<u64>,
    pub download_ms: Option<u64>,
    pub total_ms: u64,
    pub reused: bool,
    pub server_addr: String,
    pub tls_version: String,
    pub client_cert: String,
}

impl TimingBreakdown {
    pub fn has_content(&self) -> bool {
        self.dns_ms.is_some()
            || self.tcp_ms.is_some()
            || self.tls_ms.is_some()
            || self.wait_ms.is_some()
            || self.download_ms.is_some()
            || self.reused
            || !self.server_addr.is_empty()
            || !self.tls_version.is_empty()
            || !self.client_cert.is_empty()
    }

    pub fn has_phases(&self) -> bool {
        self.dns_ms.is_some()
            || self.tcp_ms.is_some()
            || self.tls_ms.is_some()
            || self.wait_ms.is_some()
            || self.download_ms.is_some()
    }
}

pub fn flow_timing_breakdown(flow: &Flow) -> TimingBreakdown {
    let conn = &flow.conn_info;
    let handshake_ms = conn
        .dns_ms
        .unwrap_or(0)
        .saturating_add(conn.tcp_ms.unwrap_or(0))
        .saturating_add(conn.tls_ms.unwrap_or(0));
    let raw_wait = flow
        .response_at
        .and_then(|at| at.duration_since(flow.started).ok())
        .map(|d| d.as_millis() as u64);
    let wait_ms = if conn.dialed_before_start {
        raw_wait
    } else {
        raw_wait.map(|w| w.saturating_sub(handshake_ms))
    };
    let download_ms = match (flow.response_at, flow.finished_at) {
        (Some(r), Some(f)) => f.duration_since(r).ok().map(|d| d.as_millis() as u64),
        _ => None,
    };
    let total_ms = handshake_ms
        .saturating_add(wait_ms.unwrap_or(0))
        .saturating_add(download_ms.unwrap_or(0));
    TimingBreakdown {
        dns_ms: conn.dns_ms,
        tcp_ms: conn.tcp_ms,
        tls_ms: conn.tls_ms,
        wait_ms,
        download_ms,
        total_ms,
        reused: conn.reused,
        server_addr: conn.server_addr.clone(),
        tls_version: conn.tls_version.clone(),
        client_cert: conn.client_cert.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{FlowDraft, Protocol, Session};

    fn flow() -> Flow {
        let mut session = Session::default();
        let id = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "POST".into(),
            host: "example.com".into(),
            path: "/v1?q=a%20b&n=1".into(),
            url: "https://example.com/v1?q=a%20b&n=1".into(),
            request_headers: vec![
                ("Host".into(), "example.com".into()),
                ("Content-Type".into(), "application/json".into()),
            ],
        });
        session.append_body(id, crate::session::Direction::Request, br#"{"a":1}"#);
        session.get(id).unwrap().clone()
    }

    #[test]
    fn curl_quotes_and_skips_host() {
        let command = curl_command(&flow());
        assert!(command.starts_with("curl -X 'POST' 'https://example.com/v1?q=a%20b&n=1'"));
        assert!(!command.contains("Host:"));
        assert!(command.contains("--data-binary '{\"a\":1}'"));
    }

    #[test]
    fn flow_timing_breakdown_computes_phases_and_reused_state() {
        let mut f = flow();
        f.conn_info = crate::session::ConnInfo {
            dns_ms: Some(10),
            tcp_ms: Some(20),
            tls_ms: Some(30),
            reused: false,
            server_addr: "93.184.216.34:443".into(),
            tls_version: "TLS 1.3".into(),
            client_cert: String::new(),
            dialed_before_start: false,
        };
        f.response_at = Some(f.started + std::time::Duration::from_millis(100));
        f.finished_at = Some(f.started + std::time::Duration::from_millis(125));
        let tb = flow_timing_breakdown(&f);
        assert_eq!(tb.dns_ms, Some(10));
        assert_eq!(tb.tcp_ms, Some(20));
        assert_eq!(tb.tls_ms, Some(30));
        assert_eq!(tb.wait_ms, Some(40));
        assert_eq!(tb.download_ms, Some(25));
        assert_eq!(tb.total_ms, 125);
        assert!(!tb.reused);
        assert_eq!(tb.server_addr, "93.184.216.34:443");
        assert_eq!(tb.tls_version, "TLS 1.3");
    }

    #[test]
    fn query_decodes_pairs() {
        let pairs = query_pairs("https://example.com/v1?q=a%20b&n=1");
        assert_eq!(
            pairs,
            vec![("q".into(), "a b".into()), ("n".into(), "1".into())]
        );
    }

    #[test]
    fn pretty_json_and_form() {
        let json = pretty_body(Some("application/json"), r#"{"a":1}"#);
        assert!(json.contains("\n"));
        assert!(json.contains("\"a\""));
        let form = pretty_body(Some("application/x-www-form-urlencoded"), "a=1&b=c+d");
        assert_eq!(form, "a=1\nb=c d");
    }

    #[test]
    fn sse_splits_on_blank_lines() {
        let events = sse_events("event: tick\ndata: one\ndata: two\n\ndata: plain\n\n");
        assert_eq!(
            events,
            vec![
                SseEvent {
                    event: "tick".into(),
                    data: "one\ntwo".into(),
                },
                SseEvent {
                    event: "message".into(),
                    data: "plain".into(),
                },
            ]
        );
    }

    #[test]
    fn pretty_headers_sort_without_changing_case() {
        let mut headers = vec![
            ("X-Trace".into(), "1".into()),
            ("accept".into(), "a".into()),
            ("Content-Type".into(), "b".into()),
            ("Accept".into(), "c".into()),
        ];
        sort_headers_pretty(&mut headers);
        assert_eq!(
            headers
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            vec!["accept", "Accept", "Content-Type", "X-Trace"]
        );
        assert_eq!(headers[0].1, "a");
        assert_eq!(headers[1].1, "c");
    }

    #[test]
    fn pretty_cookie_splits_pairs_and_flags() {
        assert_eq!(
            cookie_pairs("session=abc; theme=dark; HttpOnly"),
            vec![
                ("session".into(), "abc".into()),
                ("theme".into(), "dark".into()),
                ("HttpOnly".into(), String::new()),
            ]
        );
        assert_eq!(
            cookie_pairs("id=a; Path=/; Secure"),
            vec![
                ("id".into(), "a".into()),
                ("Path".into(), "/".into()),
                ("Secure".into(), String::new()),
            ]
        );
        let payload = base64url_encode(br#"{"sub":"a"}"#);
        let header = base64url_encode(br#"{"alg":"none"}"#);
        let token = format!("{header}.{payload}.x");
        let rows = pretty_cookie_rows("Cookie", &format!("session={token}; theme=dark")).unwrap();
        assert_eq!(rows[0].0, "Cookie · session");
        assert!(rows[0].1.starts_with(&token));
        assert!(rows[0].1.contains("\"sub\": \"a\""));
        assert_eq!(rows[1].0, "Cookie · theme");
        assert_eq!(rows[1].1, "dark");
        assert!(pretty_cookie_rows("Accept", "text/plain").is_none());
        assert!(pretty_cookie_rows("Set-Cookie", "id=a; Path=/; HttpOnly").is_none());
        assert!(pretty_cookie_rows("Cookie", "   ").is_none());
    }

    #[test]
    fn pretty_image_hides_hex_until_raw() {
        let hex = "89 50 4e 47 ".to_string();
        let note = "解压失败，以下是原始字节。boom\n十六进制只显示前 4096 字节，共 8000 字节。";
        let pretty = image_body(true, true, hex.clone(), note.into(), 128, Lang::Zh);
        assert!(pretty.text.is_empty());
        assert_eq!(pretty.caption, "共 128 字节");
        assert!(pretty.note.contains("解压失败"), "{}", pretty.note);
        assert!(!pretty.note.contains("十六进制"), "{}", pretty.note);
        let raw = image_body(
            false,
            true,
            hex.clone(),
            "Hex shows the first 4096 of 8000 bytes.".into(),
            128,
            Lang::En,
        );
        assert_eq!(raw.text, hex);
        assert!(raw.caption.is_empty());
        assert!(raw.note.starts_with("Hex shows the first "));
        let plain = image_body(true, false, "{ }".into(), String::new(), 3, Lang::En);
        assert_eq!(plain.text, "{ }");
        assert!(plain.caption.is_empty());
    }

    #[test]
    fn form_pairs_decode_and_keep_flags() {
        assert!(is_form(Some("application/x-www-form-urlencoded"), "a=1"));
        assert!(!is_form(Some("application/json"), "a=1&b=2"));
        assert_eq!(
            form_pairs("name=Ada+Lovelace&on=1"),
            vec![
                ("name".into(), "Ada Lovelace".into()),
                ("on".into(), "1".into()),
            ]
        );
    }

    #[test]
    fn preview_keeps_a_small_icon_and_shrinks_a_large_one() {
        let mut png = vec![0u8; 24];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[16..20].copy_from_slice(&80u32.to_be_bytes());
        png[20..24].copy_from_slice(&40u32.to_be_bytes());
        assert_eq!(image_pixel_size(&png), Some((80, 40)));
        assert_eq!(preview_box(80, 40), (80, 40));
        assert_eq!(preview_box(400, 200), (200, 100));
        assert_eq!(preview_box(100, 400), (50, 200));
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&16u16.to_le_bytes());
        gif.extend_from_slice(&32u16.to_le_bytes());
        assert_eq!(image_pixel_size(&gif), Some((16, 32)));
    }

    #[test]
    fn ico_preview_extracts_png_and_decodes_dib_entries() {
        use std::io::Read;

        assert_eq!(image_extension("image/x-icon"), Some("ico"));
        assert_eq!(image_extension("image/vnd.microsoft.icon"), Some("ico"));
        assert_eq!(image_extension("image/ico; charset=binary"), Some("ico"));
        assert_eq!(image_extension("application/json"), None);

        // 1. Multi-entry ICO: 1x1 24-bit DIB + 2x2 32-bit BGRA DIB (picks 2x2)
        let mut dib_1x1 = Vec::new();
        dib_1x1.extend_from_slice(&40u32.to_le_bytes());
        dib_1x1.extend_from_slice(&1i32.to_le_bytes());
        dib_1x1.extend_from_slice(&2i32.to_le_bytes());
        dib_1x1.extend_from_slice(&1u16.to_le_bytes());
        dib_1x1.extend_from_slice(&24u16.to_le_bytes());
        dib_1x1.extend_from_slice(&[0u8; 24]);
        dib_1x1.extend_from_slice(&[0x11, 0x22, 0x33, 0x00]); // padded 24-bit row
        dib_1x1.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // AND mask row

        let mut dib_2x2 = Vec::new();
        dib_2x2.extend_from_slice(&40u32.to_le_bytes());
        dib_2x2.extend_from_slice(&2i32.to_le_bytes());
        dib_2x2.extend_from_slice(&4i32.to_le_bytes()); // height * 2
        dib_2x2.extend_from_slice(&1u16.to_le_bytes());
        dib_2x2.extend_from_slice(&32u16.to_le_bytes());
        dib_2x2.extend_from_slice(&[0u8; 24]);
        // Bottom row (y = 1): blue opaque, green semi-transparent
        dib_2x2.extend_from_slice(&[255, 0, 0, 255, 0, 255, 0, 128]);
        // Top row (y = 0): red opaque, transparent
        dib_2x2.extend_from_slice(&[0, 0, 255, 255, 0, 0, 0, 0]);
        dib_2x2.extend_from_slice(&[0u8; 8]); // AND mask (2 rows * 4 bytes)

        let off1 = 6 + 32;
        let off2 = off1 + dib_1x1.len();
        let mut ico = Vec::new();
        ico.extend_from_slice(&[0, 0, 1, 0, 2, 0]);
        // Entry 1: 1x1
        ico.extend_from_slice(&[1, 1, 0, 0, 1, 0, 24, 0]);
        ico.extend_from_slice(&(dib_1x1.len() as u32).to_le_bytes());
        ico.extend_from_slice(&(off1 as u32).to_le_bytes());
        // Entry 2: 2x2
        ico.extend_from_slice(&[2, 2, 0, 0, 1, 0, 32, 0]);
        ico.extend_from_slice(&(dib_2x2.len() as u32).to_le_bytes());
        ico.extend_from_slice(&(off2 as u32).to_le_bytes());
        ico.extend_from_slice(&dib_1x1);
        ico.extend_from_slice(&dib_2x2);

        assert_eq!(image_pixel_size(&ico), Some((2, 2)));
        let png = ico_to_png(&ico).expect("valid 2x2 PNG from ICO");
        assert_eq!(image_pixel_size(&png), Some((2, 2)));
        // Verify IDAT decompresses to top-down RGBA rows
        let idat_pos = png.windows(4).position(|w| w == b"IDAT").unwrap();
        let idat_len = u32::from_be_bytes(png[idat_pos - 4..idat_pos].try_into().unwrap()) as usize;
        let mut raw = Vec::new();
        flate2::read::ZlibDecoder::new(&png[idat_pos + 4..idat_pos + 4 + idat_len])
            .read_to_end(&mut raw)
            .unwrap();
        assert_eq!(
            raw,
            vec![
                0, 255, 0, 0, 255, 0, 0, 0, 0, // top row: red opaque, transparent
                0, 0, 0, 255, 255, 0, 255, 0, 128, // bottom row: blue opaque, green 128
            ]
        );

        // 2. Legacy 32-bit BGR0 ICO where alpha channel is all zero and AND mask controls shape
        let mut bgr0 = Vec::new();
        bgr0.extend_from_slice(&40u32.to_le_bytes());
        bgr0.extend_from_slice(&2i32.to_le_bytes());
        bgr0.extend_from_slice(&2i32.to_le_bytes()); // 2x1 -> height*2 = 2
        bgr0.extend_from_slice(&1u16.to_le_bytes());
        bgr0.extend_from_slice(&32u16.to_le_bytes());
        bgr0.extend_from_slice(&[0u8; 24]);
        bgr0.extend_from_slice(&[0x10, 0x20, 0x30, 0, 0x40, 0x50, 0x60, 0]);
        bgr0.extend_from_slice(&[0b0100_0000, 0, 0, 0]); // x=0 opaque (0), x=1 transparent (1)
        let mut ico_bgr0 = Vec::new();
        ico_bgr0.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
        ico_bgr0.extend_from_slice(&[2, 1, 0, 0, 1, 0, 32, 0]);
        ico_bgr0.extend_from_slice(&(bgr0.len() as u32).to_le_bytes());
        ico_bgr0.extend_from_slice(&22u32.to_le_bytes());
        ico_bgr0.extend_from_slice(&bgr0);
        let png_bgr0 = ico_to_png(&ico_bgr0).expect("valid BGR0 ICO");
        let idat_pos = png_bgr0.windows(4).position(|w| w == b"IDAT").unwrap();
        let idat_len =
            u32::from_be_bytes(png_bgr0[idat_pos - 4..idat_pos].try_into().unwrap()) as usize;
        let mut raw_bgr0 = Vec::new();
        flate2::read::ZlibDecoder::new(&png_bgr0[idat_pos + 4..idat_pos + 4 + idat_len])
            .read_to_end(&mut raw_bgr0)
            .unwrap();
        assert_eq!(
            raw_bgr0,
            vec![0, 0x30, 0x20, 0x10, 255, 0x60, 0x50, 0x40, 0]
        );

        // 3. Embedded 256x256 PNG inside ICO (directory width/height 0 == 256)
        let embedded = png_rgba(256, 256, &vec![0u8; 256 * 256 * 4]).unwrap();
        let mut ico_png = Vec::new();
        ico_png.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
        ico_png.extend_from_slice(&[0, 0, 0, 0, 1, 0, 32, 0]);
        ico_png.extend_from_slice(&(embedded.len() as u32).to_le_bytes());
        ico_png.extend_from_slice(&22u32.to_le_bytes());
        ico_png.extend_from_slice(&embedded);
        assert_eq!(image_pixel_size(&ico_png), Some((256, 256)));
        assert_eq!(preview_box(256, 256), (200, 200));
        assert_eq!(ico_to_png(&ico_png).as_deref(), Some(embedded.as_slice()));

        // 4. Truncated / invalid bytes return None
        assert_eq!(image_pixel_size(b"\x00\x00\x01\x00\x01\x00truncated"), None);
        assert_eq!(ico_to_png(b"not an icon"), None);
    }

    #[test]
    fn editor_language_follows_the_content_type() {
        assert_eq!(
            editor_language(Some("application/json; charset=utf-8"), "{\"a\":1}"),
            Some("json")
        );
        assert_eq!(editor_language(Some("text/html"), "<p>"), Some("html"));
        assert_eq!(
            editor_language(Some("text/markdown"), "# hi"),
            Some("markdown")
        );
        assert_eq!(editor_language(Some("text/xml"), "<a/>"), Some("xml"));
        assert!(parse_text_view_doc(Some("text/html"), b"<h1>Hello</h1><p>World</p>").is_some());
        assert!(parse_text_view_doc(Some("text/markdown"), b"# Hello\n\n- item").is_some());
        assert!(
            parse_text_view_doc(
                Some("text/html"),
                b"<!doctype html><html><head><script src='a.js'></script></head><body><div id='app'></div></body></html>"
            )
            .is_none()
        );
        assert!(parse_text_view_doc(Some("application/json"), b"{\"a\":1}").is_none());
        let long = "a\n".repeat(50);
        assert_eq!(
            editor_language(Some("text/plain"), &long),
            Some("plaintext")
        );
        assert_eq!(editor_language(Some("image/png"), "not text"), None);
        assert_eq!(editor_language(Some("text/plain"), "short"), None);
        assert_eq!(
            editor_language(Some("text/plain"), "{\"a\":1}"),
            Some("json")
        );
        assert_eq!(editor_language(None, "[\n  1\n]"), Some("json"));
        assert_eq!(editor_language(Some("image/png"), "{\"a\":1}"), None);
        assert_eq!(editor_language(None, ""), None);
    }

    #[test]
    fn gap_and_clock_keep_milliseconds() {
        let start = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(format_elapsed(start, None), "—");
        assert_eq!(
            format_elapsed(start, Some(start + Duration::from_millis(45))),
            "45 ms"
        );
        assert_eq!(
            format_elapsed(start, Some(start + Duration::from_millis(1_000))),
            "1.0 s"
        );
        assert_eq!(
            format_elapsed(start, Some(start + Duration::from_millis(1_240))),
            "1.2 s"
        );
        assert_eq!(
            format_elapsed(start, Some(start + Duration::from_millis(1_250))),
            "1.3 s"
        );
        assert_eq!(format_gap(None, start), "0 ms");
        assert_eq!(
            format_gap(Some(start + Duration::from_secs(2)), start),
            "0 ms"
        );
        assert_eq!(
            format_gap(Some(start), start + Duration::from_millis(45)),
            "45 ms"
        );
        assert_eq!(
            format_gap(Some(start), start + Duration::from_millis(1_240)),
            "1.2 s"
        );
        assert_eq!(
            format_gap(Some(start), start + Duration::from_millis(10_500)),
            "10.5 s"
        );
        assert_eq!(
            format_gap(Some(start), start + Duration::from_millis(90_000)),
            "1.5 m"
        );
        let clock = format_clock(start + Duration::from_millis(123));
        assert!(clock.ends_with(".123"), "{clock}");
        assert_eq!(clock.len(), 12);
    }

    #[test]
    fn port_rejects_zero_and_blank() {
        assert_eq!(parse_port("8877"), Some(8877));
        assert_eq!(parse_port(" 80 "), Some(80));
        assert_eq!(parse_port("0"), None);
        assert_eq!(parse_port(""), None);
        assert_eq!(parse_port("65536"), None);
    }

    #[test]
    fn snippets_env_and_curl_round_trip() {
        let flow = flow();
        let fetch = fetch_snippet(&flow);
        assert!(fetch.contains("fetch(\"https://example.com/v1?q=a%20b&n=1\""));
        assert!(fetch.contains("\"Content-Type\""));
        assert!(!fetch.contains("Host"));
        let python = python_requests(&flow);
        assert!(python.contains("import requests"));
        assert!(python.contains("\"POST\","));
        let env = proxy_env_export(8877, "/tmp/zniff/ca.crt");
        assert!(env.contains("export https_proxy=http://127.0.0.1:8877"));
        assert!(env.contains("export ALL_PROXY=http://127.0.0.1:8877"));
        assert!(env.contains("export NODE_EXTRA_CA_CERTS='/tmp/zniff/ca.crt'"));
        assert!(env.contains("export CARGO_HTTP_CAINFO='/tmp/zniff/ca.crt'"));
        assert!(env.contains("export DENO_CERT='/tmp/zniff/ca.crt'"));
        assert!(env.contains(
            "export JAVA_TOOL_OPTIONS='-Dhttp.proxyHost=127.0.0.1 -Dhttp.proxyPort=8877 -Dhttps.proxyHost=127.0.0.1 -Dhttps.proxyPort=8877'"
        ));
        let auth_env = proxy_env_export_with_auth(8877, "/tmp/zniff/ca.crt", "alice:p@ss:1");
        assert!(auth_env.contains("export https_proxy=http://alice:p%40ss%3A1@127.0.0.1:8877"));
        assert!(auth_env.contains("export ALL_PROXY=http://alice:p%40ss%3A1@127.0.0.1:8877"));
        assert_eq!(parse_repeat_count(""), 5);
        assert_eq!(parse_repeat_count("0"), 1);
        assert_eq!(parse_repeat_count("12"), 12);
        assert_eq!(parse_repeat_count("999"), 100);
        assert_eq!(parse_repeat_concurrency(""), 1);
        assert_eq!(parse_repeat_concurrency("0"), 1);
        assert_eq!(parse_repeat_concurrency("4"), 4);
        assert_eq!(parse_repeat_concurrency("99"), 20);
        assert_eq!(parse_repeat_interval_ms(""), 200);
        assert_eq!(parse_repeat_interval_ms("0"), 0);
        assert_eq!(parse_repeat_interval_ms("500"), 500);
        assert_eq!(parse_repeat_interval_ms("120000"), 60_000);
        assert!(message_matches_filter(
            "客户端 文本",
            "{\"op\":\"sub\"}",
            "sub",
            1,
            1
        ));
        assert!(!message_matches_filter(
            "客户端 文本",
            "{\"op\":\"sub\"}",
            "sub",
            2,
            1
        ));
        assert!(!message_matches_filter(
            "客户端 文本",
            "{\"op\":\"sub\"}",
            "sub",
            1,
            2
        ));
        assert!(message_matches_filter("源站 ping", "", "", 2, 3));
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(2048), "2.0 KB");
        let parsed = parse_curl(
            "curl -X POST 'https://example.com/v1' -H 'Accept: text/plain' --data-raw '{\"a\":1}'",
        )
        .unwrap();
        assert_eq!(parsed.method, "POST");
        assert_eq!(parsed.url, "https://example.com/v1");
        assert_eq!(parsed.headers, vec![("Accept".into(), "text/plain".into())]);
        assert_eq!(parsed.body, "{\"a\":1}");
        let implied = parse_curl("curl https://example.com/v1 -d hi").unwrap();
        assert_eq!(implied.method, "POST");
        assert_eq!(implied.body, "hi");
    }

    #[test]
    fn types_jwt_and_diff() {
        let ts = typescript_interface(r#"{"user":{"id":1},"tags":["a"]}"#).unwrap();
        assert!(ts.contains("export interface Response"));
        assert!(ts.contains("user: User;"));
        assert!(ts.contains("tags: string[];"));
        let payload = base64url_encode(br#"{"sub":"a","exp":1700000000}"#);
        let header = base64url_encode(br#"{"alg":"none"}"#);
        let hint =
            annotate_value("Authorization", &format!("Bearer {header}.{payload}.x")).unwrap();
        assert!(hint.contains("\"sub\": \"a\""));
        assert!(hint.contains("2023-11-14 22:13:20 UTC"));
        let stamp = annotate_value("X-Time", "1700000000").unwrap();
        assert!(stamp.contains("2023-11-14 22:13:20 UTC"));
        let encoded = base64_encode(b"hello-zniff");
        let decoded = annotate_value("X-Data", &encoded).unwrap();
        assert!(decoded.contains("hello-zniff"));
        let diff = line_diff("a\nb\nc", "a\nc\nd");
        assert!(
            diff.iter()
                .any(|line| line.kind == DiffKind::Remove && line.text == "b")
        );
        assert!(
            diff.iter()
                .any(|line| line.kind == DiffKind::Add && line.text == "d")
        );
    }

    fn base64url_encode(bytes: &[u8]) -> String {
        base64_encode(bytes)
            .replace('+', "-")
            .replace('/', "_")
            .replace('=', "")
    }

    fn base64_encode(bytes: &[u8]) -> String {
        const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        let mut index = 0;
        while index + 3 <= bytes.len() {
            let n = ((bytes[index] as u32) << 16)
                | ((bytes[index + 1] as u32) << 8)
                | bytes[index + 2] as u32;
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
            out.push(TABLE[(n & 63) as usize] as char);
            index += 3;
        }
        if index < bytes.len() {
            let left = bytes.len() - index;
            let mut n = (bytes[index] as u32) << 16;
            if left == 2 {
                n |= (bytes[index + 1] as u32) << 8;
            }
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            if left == 2 {
                out.push(TABLE[((n >> 6) & 63) as usize] as char);
                out.push('=');
            } else {
                out.push('=');
                out.push('=');
            }
        }
        out
    }

    #[test]
    fn multipart_form_data_extracts_text_and_file_parts() {
        let ct = Some("multipart/form-data; boundary=----ZniffBoundary7MA4YWxkTrZu0gW");
        let raw = b"------ZniffBoundary7MA4YWxkTrZu0gW\r\n\
Content-Disposition: form-data; name=\"scene\"\r\n\r\n\
avatar\r\n\
------ZniffBoundary7MA4YWxkTrZu0gW\r\n\
Content-Disposition: form-data; name=\"file\"; filename=\"logo.png\"\r\n\
Content-Type: image/png\r\n\r\n\
\x89PNG\r\n\x1a\n\r\n\
------ZniffBoundary7MA4YWxkTrZu0gW--\r\n";
        let pairs = multipart_pairs(ct, raw).expect("multipart pairs");
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0], ("scene".to_string(), "avatar".to_string()));
        assert_eq!(pairs[1].0, "file");
        assert!(pairs[1].1.contains("logo.png"));
        assert!(pairs[1].1.contains("image/png"));
    }

    #[test]
    fn graphql_protobuf_msgpack_and_body_search_work() {
        let gql_json = r#"{"operationName":"GetUser","query":"query GetUser($id: ID!) { user(id: $id) { id name profile { avatar } } }","variables":{"id":"42"}}"#;
        let pretty = pretty_body(Some("application/json"), gql_json);
        assert!(pretty.contains("# operationName: GetUser"));
        assert!(pretty.contains("query GetUser($id: ID!) {"));
        assert!(pretty.contains("  user(id: $id) {"));
        assert!(pretty.contains("    id"));
        assert!(pretty.contains("# variables\n{\n  \"id\": \"42\"\n}"));

        // Protobuf wire: field #1 varint 150 (0x08 0x96 0x01), field #2 string "zniff" (0x12 0x05 b"zniff")
        let proto = b"\x08\x96\x01\x12\x05zniff";
        let decoded_proto =
            pretty_binary_body(Some("application/x-protobuf"), proto).expect("protobuf decode");
        assert!(decoded_proto.contains("field #1 (varint): 150"));
        assert!(decoded_proto.contains("field #2 (string): \"zniff\""));

        // gRPC frame: 0x00 (uncompressed) + 0x0000000a (10 bytes) + proto
        let mut grpc = vec![0x00, 0x00, 0x00, 0x00, proto.len() as u8];
        grpc.extend_from_slice(proto);
        let decoded_grpc =
            pretty_binary_body(Some("application/grpc+proto"), &grpc).expect("grpc decode");
        assert!(decoded_grpc.contains("field #1 (varint): 150"));
        assert!(decoded_grpc.contains("field #2 (string): \"zniff\""));

        // MessagePack: {"ok": true, "n": 7} -> 0x82 (map 2), 0xa2 "ok", 0xc3 (true), 0xa1 "n", 0x07
        let msgpack = b"\x82\xa2ok\xc3\xa1n\x07";
        let decoded_mp =
            pretty_binary_body(Some("application/msgpack"), msgpack).expect("msgpack decode");
        assert!(decoded_mp.contains("\"ok\": true"));
        assert!(decoded_mp.contains("\"n\": 7"));

        let (hits, filtered) = filter_body_lines("alpha\nBeta match\ngamma MATCH two", "match");
        assert_eq!(hits, 2);
        assert_eq!(filtered, "[L2] Beta match\n[L3] gamma MATCH two");
    }

    #[test]
    fn mermaid_sequence_diagram_deduplicates_hosts_and_sanitizes_messages() {
        let mut session = Session::default();
        let start = UNIX_EPOCH + Duration::from_secs(1_700_000_000);

        let id1 = session.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "GET".into(),
            host: "api.example.com".into(),
            path: "/api/v1/items?page=1&filter=a;b#sec".into(),
            url: "https://api.example.com/api/v1/items?page=1&filter=a;b#sec".into(),
            request_headers: vec![],
        });
        session.set_response(id1, 200, vec![], None);
        session.add_transfer(id1, crate::session::Direction::Response, 1229);
        let _ = session.set_comment(id1, "Load list; step #1\nnext line");

        let id2 = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "POST".into(),
            host: "auth.example.com:8443".into(),
            path: "/oauth/token".into(),
            url: "https://auth.example.com:8443/oauth/token".into(),
            request_headers: vec![],
        });
        session.set_error(id2, "connection reset; peer #2");

        let id3 = session.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "GET".into(),
            host: "api.example.com".into(),
            path: "/api/v1/stream".into(),
            url: "https://api.example.com/api/v1/stream".into(),
            request_headers: vec![],
        });

        let mut f1 = session.get(id1).unwrap().clone();
        f1.started = start;
        f1.finished_at = Some(start + Duration::from_millis(45));
        let mut f2 = session.get(id2).unwrap().clone();
        f2.started = start + Duration::from_millis(60);
        f2.finished_at = Some(start + Duration::from_millis(70));
        let mut f3 = session.get(id3).unwrap().clone();
        f3.started = start + Duration::from_millis(80);
        f3.finished_at = None;

        let flows = [&f1, &f2, &f3];
        let diagram = mermaid_sequence_diagram(&flows, true, false);
        assert!(diagram.starts_with(
            "sequenceDiagram\n    autonumber\n    participant C as Client\n    participant H1 as api.example.com\n    participant H2 as \"auth.example.com:8443\""
        ));
        // Deduplicated H1 (only 2 host participants total)
        assert!(!diagram.contains("participant H3"));
        // Sanitized comment note
        assert!(diagram.contains("Note over C,H1: Load list#59; step #35;1 next line"));
        // Stripped query string when include_query == false, with activation `+` and `[+0 ms]`
        assert!(diagram.contains("C->>+H1: GET /api/v1/items [+0 ms]\n"));
        assert!(!diagram.contains("page=1"));
        // Response arrow with deactivation `-`, status, elapsed time, and size
        assert!(diagram.contains("H1-->>-C: 200 OK (45 ms, 1.2 KB)"));
        // Failed request and error response arrows with sanitized error
        assert!(diagram.contains("C-x+H2: POST /oauth/token [+60 ms]"));
        assert!(diagram.contains("H2--x-C: ERR (connection reset#59; peer #35;2)"));
        // In-progress response arrow
        assert!(diagram.contains("C->>+H1: GET /api/v1/stream [+80 ms]\n    H1-->>-C: ..."));

        // With include_query == true and include_response == false
        let req_only = mermaid_sequence_diagram(&flows, false, true);
        assert!(req_only.contains("C->>H1: GET /api/v1/items?page=1&filter=a#59;b#35;sec [+0 ms]"));
        assert!(!req_only.contains("-->>"));
        assert!(!req_only.contains("--x"));

        let md = mermaid_markdown_block(&req_only);
        assert!(md.starts_with("```mermaid\nsequenceDiagram\n"));
        assert!(md.ends_with("\n```"));

        // Concurrent overlapping requests: slow request starts at +0ms (500ms duration),
        // fast request on the same host starts at +2ms (48ms duration) and finishes first at +50ms!
        let mut slow = f1.clone();
        slow.id = 10;
        slow.path = "/api/v1/slow".into();
        slow.comment.clear();
        slow.started = start;
        slow.finished_at = Some(start + Duration::from_millis(500));
        let mut fast = f1.clone();
        fast.id = 11;
        fast.path = "/api/v1/fast".into();
        fast.comment.clear();
        fast.started = start + Duration::from_millis(2);
        fast.finished_at = Some(start + Duration::from_millis(50));
        let concurrent_data = build_sequence_diagram_data(&[&slow, &fast], true, false);
        assert_eq!(concurrent_data.hosts, vec!["api.example.com".to_string()]);
        assert_eq!(concurrent_data.steps.len(), 4);
        assert!(concurrent_data.steps[0].is_request);
        assert!(concurrent_data.steps[1].is_request);
        assert!(!concurrent_data.steps[2].is_request);
        assert!(!concurrent_data.steps[3].is_request);
        assert_eq!(concurrent_data.steps[1].active_hosts, vec![true]);
        assert_eq!(concurrent_data.bars.len(), 2);
        assert_eq!(concurrent_data.bars[0].start_ms, 0.0);
        assert_eq!(concurrent_data.bars[0].end_ms, 500.0);
        assert_eq!(concurrent_data.bars[1].start_ms, 2.0);
        assert_eq!(concurrent_data.bars[1].end_ms, 50.0);
        let concurrent = concurrent_data.mermaid;
        let idx_req_slow = concurrent
            .find("C->>+H1: GET /api/v1/slow [+0 ms]")
            .unwrap();
        let idx_req_fast = concurrent
            .find("C->>+H1: GET /api/v1/fast [+2 ms]")
            .unwrap();
        let idx_res_fast = concurrent
            .find("H1-->>-C: 200 OK /api/v1/fast (48 ms, 1.2 KB)")
            .unwrap();
        let idx_res_slow = concurrent
            .find("H1-->>-C: 200 OK /api/v1/slow (500 ms, 1.2 KB)")
            .unwrap();
        assert!(idx_req_slow < idx_req_fast);
        assert!(idx_req_fast < idx_res_fast);
        assert!(idx_res_fast < idx_res_slow);

        // Empty slice edge case
        let empty = mermaid_sequence_diagram(&[], true, false);
        assert_eq!(
            empty,
            "sequenceDiagram\n    autonumber\n    participant C as Client"
        );

        // Fallback host extraction from URL when flow.host is empty, and status without body/elapsed
        let id4 = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "HEAD".into(),
            host: String::new(),
            path: String::new(),
            url: "https://[::1]:8443/healthz?probe=1".into(),
            request_headers: vec![],
        });
        session.set_response(id4, 204, vec![], None);
        let mut f4 = session.get(id4).unwrap().clone();
        f4.finished_at = None;
        let single = mermaid_sequence_diagram(&[&f4], true, false);
        assert!(single.contains("participant H1 as \"[::1]:8443\""));
        assert!(single.contains("C->>+H1: HEAD /healthz\n"));
        assert!(single.contains("H1-->>-C: 204 No Content"));

        // URL without slash before query string and relative path without leading slash
        let id5 = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: String::new(),
            path: String::new(),
            url: "https://user:pass@search.example.com?q=rust".into(),
            request_headers: vec![],
        });
        let f5 = session.get(id5).unwrap().clone();
        assert_eq!(resolve_flow_host(&f5), "search.example.com");
        assert_eq!(resolve_flow_target(&f5, false), "/");
        assert_eq!(resolve_flow_target(&f5, true), "/?q=rust");

        // Automatic data dependency & stage DAG analysis (Cookie + Token + QueryParam + Stages)
        let dep_login_id = session.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "POST".into(),
            host: "auth.example.com".into(),
            path: "/v1/login".into(),
            url: "https://auth.example.com/v1/login".into(),
            request_headers: vec![],
        });
        session.set_response(
            dep_login_id,
            200,
            vec![
                (
                    "Set-Cookie".into(),
                    "sid=sess_987654321; Path=/; HttpOnly".into(),
                ),
                ("Content-Type".into(), "application/json".into()),
            ],
            None,
        );
        session.append_body(
            dep_login_id,
            crate::session::Direction::Response,
            br#"{"access_token":"tok_abcdef123456","order_id":"ord_55667788"}"#,
        );
        let mut dep_login = session.get(dep_login_id).unwrap().clone();
        dep_login.started = start;
        dep_login.finished_at = Some(start + Duration::from_millis(40));

        let dep_profile_id = session.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "GET".into(),
            host: "api.example.com".into(),
            path: "/v1/profile".into(),
            url: "https://api.example.com/v1/profile".into(),
            request_headers: vec![
                ("Cookie".into(), "sid=sess_987654321".into()),
                ("Authorization".into(), "Bearer tok_abcdef123456".into()),
            ],
        });
        session.set_response(dep_profile_id, 200, vec![], None);
        let mut dep_profile = session.get(dep_profile_id).unwrap().clone();
        dep_profile.started = start + Duration::from_millis(50);
        dep_profile.finished_at = Some(start + Duration::from_millis(90));

        let dep_order_id = session.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "GET".into(),
            host: "api.example.com".into(),
            path: "/v1/orders?id=ord_55667788".into(),
            url: "https://api.example.com/v1/orders?id=ord_55667788".into(),
            request_headers: vec![("Authorization".into(), "Bearer tok_abcdef123456".into())],
        });
        session.set_response(dep_order_id, 200, vec![], None);
        let mut dep_order = session.get(dep_order_id).unwrap().clone();
        dep_order.started = start + Duration::from_millis(52);
        dep_order.finished_at = Some(start + Duration::from_millis(160));

        let dag = build_sequence_diagram_data(&[&dep_login, &dep_profile, &dep_order], true, true);
        assert_eq!(dag.stages.len(), 2);
        assert_eq!(dag.stages[0].bar_indices, vec![0]);
        assert_eq!(dag.stages[1].bar_indices, vec![1, 2]);
        assert!(
            dag.bars[1]
                .deps
                .iter()
                .any(|d| d.from_index == 0 && d.kind == DepKind::Cookie)
        );
        assert!(
            dag.bars[1]
                .deps
                .iter()
                .any(|d| d.from_index == 0 && d.kind == DepKind::Token)
        );
        assert!(
            dag.bars[2]
                .deps
                .iter()
                .any(|d| d.from_index == 0 && d.kind == DepKind::QueryParam)
        );
        // Critical path: login (#0, ends 40ms) -> order (#2, ends 160ms)
        assert!(dag.bars[0].on_critical_path);
        assert!(!dag.bars[1].on_critical_path);
        assert!(dag.bars[2].on_critical_path);
        assert!(dag.flowchart_mermaid.starts_with("flowchart TD\n"));
    }

    #[test]
    fn graphql_operations_extract_explicit_inferred_anonymous_and_batched() {
        let explicit =
            br#"{"operationName":"GetUser","query":"query GetUser($id:ID!){user(id:$id){name}}"}"#;
        assert_eq!(graphql_operations("/graphql", explicit), vec!["GetUser"]);
        assert_eq!(
            graphql_operation_label("/graphql", explicit).as_deref(),
            Some("GetUser")
        );

        let inferred = br#"{"query":"mutation UpdateCart($input: CartInput!) { updateCart(input: $input) { id } }"}"#;
        assert_eq!(graphql_operations("/graphql", inferred), vec!["UpdateCart"]);

        let anon = br#"{"query":"{ viewer { id } }"}"#;
        assert_eq!(graphql_operations("/graphql", anon), vec!["anonymous"]);

        let batched = br#"[{"operationName":"GetHeader","query":"query GetHeader{h}"},{"query":"query GetFooter { f }"}]"#;
        assert_eq!(
            graphql_operations("/graphql", batched),
            vec!["GetHeader", "GetFooter"]
        );
        assert_eq!(
            graphql_operation_label("/graphql", batched).as_deref(),
            Some("GetHeader, GetFooter")
        );

        let get_query = "/graphql?query=query%20ViewerProfile%20%7B%20me%20%7B%20id%20%7D%20%7D";
        assert_eq!(graphql_operations(get_query, b""), vec!["ViewerProfile"]);
    }

    #[test]
    fn rust_and_go_snippets_and_serde_struct() {
        let mut session = crate::session::Session::default();
        let id = session.open(crate::session::FlowDraft {
            protocol: crate::session::Protocol::Http2,
            method: "POST".into(),
            host: "api.example.com".into(),
            path: "/v1/items".into(),
            url: "https://api.example.com/v1/items".into(),
            request_headers: vec![
                ("Host".into(), "api.example.com".into()),
                ("Content-Type".into(), "application/json".into()),
                ("X-Trace".into(), "t1".into()),
            ],
        });
        session.append_body(id, crate::session::Direction::Request, br#"{"name":"box"}"#);
        let flow = session.get(id).unwrap().clone();

        let rust_code = rust_reqwest(&flow);
        assert!(rust_code.contains("reqwest::Method::POST"));
        assert!(rust_code.contains("\"https://api.example.com/v1/items\""));
        assert!(rust_code.contains(".header(\"Content-Type\", \"application/json\")"));
        assert!(!rust_code.contains("\"Host\""));
        assert!(rust_code.contains(".body(\"{\\\"name\\\":\\\"box\\\"}\")"));

        let go_code = go_http(&flow);
        assert!(go_code.contains("\"net/http\""));
        assert!(go_code.contains("\"strings\""));
        assert!(go_code.contains("http.NewRequest(\"POST\", \"https://api.example.com/v1/items\""));
        assert!(go_code.contains("req.Header.Set(\"X-Trace\", \"t1\")"));

        let rust_struct = rust_serde_struct(
            r#"{"userId": 42, "type": "admin", "1st-tag": true, "profile": {"display_name": "Ada"}}"#,
        )
        .unwrap();
        assert!(rust_struct.contains("use serde::{Deserialize, Serialize};"));
        assert!(rust_struct.contains("pub struct Profile {"));
        assert!(rust_struct.contains("pub display_name: String,"));
        assert!(rust_struct.contains("pub struct Response {"));
        assert!(rust_struct.contains("#[serde(rename = \"userId\")]"));
        assert!(rust_struct.contains("pub user_id: i64,"));
        assert!(rust_struct.contains("pub r#type: String,"));
        assert!(rust_struct.contains("#[serde(rename = \"1st-tag\")]"));
        assert!(rust_struct.contains("pub f_1st_tag: bool,"));
        assert!(rust_serde_struct("123").is_none());

        assert_eq!(hex_dump_raw(&[0x00, 0xab, 0xff]), "00 ab ff");
    }

    #[test]
    fn ws_message_detection_and_direction_filtering() {
        // Both Chinese and English labels are identified as WS messages
        assert!(is_ws_message("客户端 文本"));
        assert!(is_ws_message("client text"));
        assert!(is_ws_message("源站 二进制"));
        assert!(is_ws_message("origin binary"));
        assert!(is_ws_message("↑ 客户端"));
        assert!(is_ws_message("↓ origin"));

        // Direction helpers
        assert!(is_ws_client_message("客户端 文本"));
        assert!(is_ws_client_message("client text"));
        assert!(is_ws_client_message("↑ 客户端"));
        assert!(!is_ws_client_message("源站 二进制"));
        assert!(!is_ws_client_message("origin binary"));

        assert!(is_ws_origin_message("源站 二进制"));
        assert!(is_ws_origin_message("origin binary"));
        assert!(is_ws_origin_message("↓ origin"));
        assert!(!is_ws_origin_message("客户端 文本"));
        assert!(!is_ws_origin_message("client text"));

        // SSE events and truncated markers are NOT WS messages
        assert!(!is_ws_message("message"));
        assert!(!is_ws_message("ping_event"));
        assert!(!is_ws_message("…"));

        // Direction filtering with English and Chinese labels
        assert!(message_matches_filter(
            "client text",
            "{\"cmd\":\"login\"}",
            "",
            1,
            0
        ));
        assert!(!message_matches_filter(
            "client text",
            "{\"cmd\":\"login\"}",
            "",
            2,
            0
        ));
        assert!(message_matches_filter("origin binary", "", "", 2, 2));
        assert!(!message_matches_filter("origin binary", "", "", 1, 2));
        assert!(message_matches_filter("客户端 文本", "hello", "", 1, 1));
        assert!(!message_matches_filter("源站 文本", "hello", "", 1, 1));
    }

    #[test]
    fn rust_string_escapes_all_ascii_control_characters() {
        // 1. Single control character verification (0x00..=0x1F and 0x7F)
        for b in 0u8..=0x1f {
            let s = (b as char).to_string();
            let lit = rust_string(&s);
            assert!(lit.starts_with('"') && lit.ends_with('"'));
            assert!(
                !lit.contains(r"\b"),
                "Byte 0x{:02x} must not emit \\b: {}",
                b,
                lit
            );
            assert!(
                !lit.contains(r"\f"),
                "Byte 0x{:02x} must not emit \\f: {}",
                b,
                lit
            );
            assert!(
                !lit.contains(r"\u00"),
                "Byte 0x{:02x} must not emit unbraced \\u00: {}",
                b,
                lit
            );
        }

        // 2. Exact token assertions
        assert_eq!(rust_string(""), "\"\"");
        assert_eq!(rust_string("\0"), "\"\\0\"");
        assert_eq!(rust_string("\x08"), "\"\\u{8}\"");
        assert_eq!(rust_string("\x0c"), "\"\\u{c}\"");
        assert_eq!(rust_string("\t"), "\"\\t\"");
        assert_eq!(rust_string("\n"), "\"\\n\"");
        assert_eq!(rust_string("\r"), "\"\\r\"");
        assert_eq!(rust_string("\""), "\"\\\"\"");
        assert_eq!(rust_string("\\"), "\"\\\\\"");
        assert_eq!(rust_string("'"), "\"'\"");
        assert_eq!(
            rust_string("hello\x08world\x0c!"),
            "\"hello\\u{8}world\\u{c}!\""
        );
        assert_eq!(rust_string("中文测试 🦀"), "\"中文测试 🦀\"");
    }

    #[test]
    fn rust_reqwest_and_serde_struct_control_characters() {
        let mut session = Session::default();
        let id = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "CUSTOM\x08".into(),
            host: "api.example.com".into(),
            path: "/v1/\x08\x0c".into(),
            url: "https://api.example.com/v1/\x08\x0c".into(),
            request_headers: vec![("X-Ctrl\x01".into(), "val\x08\x0c\r\n\t".into())],
        });
        session.append_body(
            id,
            crate::session::Direction::Request,
            b"payload\x08\x0c\0end",
        );
        let flow = session.get(id).unwrap().clone();

        let code = rust_reqwest(&flow);
        assert!(code.contains("\"CUSTOM\\u{8}\""));
        assert!(code.contains("\"https://api.example.com/v1/\\u{8}\\u{c}\""));
        assert!(code.contains(".header(\"X-Ctrl\\u{1}\", \"val\\u{8}\\u{c}\\r\\n\\t\")"));
        assert!(code.contains(".body(\"payload\\u{8}\\u{c}\\0end\")"));
        assert!(!code.contains(r"\b"));
        assert!(!code.contains(r"\f"));

        let rust_struct = rust_serde_struct(r#"{"key\b1": 1, "key\f2": "val"}"#).unwrap();
        assert!(rust_struct.contains(r#"#[serde(rename = "key\u{8}1")]"#));
        assert!(rust_struct.contains(r#"#[serde(rename = "key\u{c}2")]"#));
        assert!(!rust_struct.contains(r"\b"));
        assert!(!rust_struct.contains(r"\f"));
    }

    #[test]
    fn format_count_large_values_drop_decimal() {
        assert_eq!(format_count(0), "0 B");
        assert_eq!(format_count(1024), "1.0 KB");
        assert_eq!(format_count(99 * 1024), "99.0 KB");
        assert_eq!(format_count(100 * 1024), "100 KB");
        assert_eq!(format_count(256 * 1024), "256 KB");
        assert_eq!(format_count(1024 * 1024), "1.0 MB");
        assert_eq!(format_count(99 * 1024 * 1024), "99.0 MB");
        assert_eq!(format_count(100 * 1024 * 1024), "100 MB");
    }

    #[test]
    fn py_string_escapes_trailing_quotes_crlf_and_control_chars() {
        assert_eq!(py_string("ends-with-'"), r#""ends-with-'""#);
        assert_eq!(py_string("triple ''' quotes"), r#""triple ''' quotes""#);
        assert_eq!(
            py_string("line1\r\nline2\t\"\\"),
            r#""line1\r\nline2\t\"\\""#
        );
        assert_eq!(py_string("nul\0and7\x07\x7f"), r#""nul\x00and7\x07\x7f""#);
    }

    #[test]
    fn codegen_handles_digit_leading_keys_and_avoids_rust_prelude_shadowing() {
        let sample = r#"{
            "1st_stage": {"ok": true},
            "string": {"label": "hi"},
            "vec": [{"n": 1}],
            "option": {"flag": null},
            "result": {"code": 200},
            "box": {"size": 4}
        }"#;
        let ts = typescript_interface(sample).unwrap();
        assert!(ts.contains("export interface N1stStage {"), "{ts}");
        assert!(ts.contains("\"1st_stage\": N1stStage;"), "{ts}");

        let rs = rust_serde_struct(sample).unwrap();
        assert!(rs.contains("pub struct N1stStage {"), "{rs}");
        assert!(rs.contains("pub struct StringValue {"), "{rs}");
        assert!(rs.contains("pub struct VecValue {"), "{rs}");
        assert!(rs.contains("pub struct OptionValue {"), "{rs}");
        assert!(rs.contains("pub struct ResultValue {"), "{rs}");
        assert!(rs.contains("pub struct BoxValue {"), "{rs}");
        assert!(!rs.contains("pub struct String {"), "{rs}");
        assert!(!rs.contains("pub struct Vec {"), "{rs}");
        assert!(!rs.contains("pub struct Option {"), "{rs}");
    }

    #[test]
    fn parse_curl_handles_line_continuations_ansi_c_quotes_and_non_request_flags() {
        // Multi-line cURL with both \\\n and \\\r\n continuations and '\'' single-quote escape
        let multiline = concat!(
            "curl -X POST 'https://api.example.com/it'\\''s' \\\n",
            "  -o /tmp/out.json --connect-timeout 5 \\\r\n",
            "  -m 10 --write-out '%{http_code}' \\\n",
            "  -H 'Content-Type: application/json' \\\r\n",
            "  --data-raw '{\"ok\":true}'"
        );
        let req = parse_curl(multiline).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.url, "https://api.example.com/it's");
        assert_eq!(
            req.headers,
            vec![("Content-Type".into(), "application/json".into())]
        );
        assert_eq!(req.body, "{\"ok\":true}");

        // Chrome DevTools "Copy as cURL (bash)" ANSI-C $'...' quoting + unquoted \x escapes
        let chrome_curl = r#"curl https://api.example.com/search\?q=1\&p=2 -H $'X-Note: it\'s\tok' --data-raw $'line1\r\n\xe4\xb8\xad\u6587' --output=/tmp/ignored.bin --max-time=30"#;
        let req2 = parse_curl(chrome_curl).unwrap();
        assert_eq!(req2.method, "POST");
        assert_eq!(req2.url, "https://api.example.com/search?q=1&p=2");
        assert_eq!(req2.headers, vec![("X-Note".into(), "it's\tok".into())]);
        assert_eq!(req2.body, "line1\r\n中文");
    }

    #[test]
    fn parse_curl_supports_json_head_multi_data_urlencode_and_get() {
        // 1. --json <data> and --json=<data>: POST default + Content-Type & Accept defaults
        let json_req =
            parse_curl(r#"curl --json '{"name":"zniff","v":1}' https://api.example.com/v1/apps"#)
                .unwrap();
        assert_eq!(json_req.method, "POST");
        assert_eq!(json_req.url, "https://api.example.com/v1/apps");
        assert_eq!(json_req.body, r#"{"name":"zniff","v":1}"#);
        assert_eq!(
            json_req.headers,
            vec![
                ("Content-Type".into(), "application/json".into()),
                ("Accept".into(), "application/json".into()),
            ]
        );

        // --json=<data> with explicit -X PATCH and case-insensitive pre-existing headers (even after --json)
        let json_override = parse_curl(
            r#"curl --json='{"patched":true}' -X PATCH -H 'content-type: application/merge-patch+json' -H 'ACCEPT: application/problem+json' https://api.example.com/v1/apps/1"#,
        )
        .unwrap();
        assert_eq!(json_override.method, "PATCH");
        assert_eq!(json_override.body, r#"{"patched":true}"#);
        assert_eq!(
            json_override.headers,
            vec![
                ("content-type".into(), "application/merge-patch+json".into()),
                ("ACCEPT".into(), "application/problem+json".into()),
            ]
        );

        // 2. -I / --head (and combined -sI) + -X override
        let head_short = parse_curl("curl -I https://example.com/healthz").unwrap();
        assert_eq!(head_short.method, "HEAD");
        assert_eq!(head_short.url, "https://example.com/healthz");
        assert!(head_short.body.is_empty());

        let head_long = parse_curl("curl --head https://example.com/healthz").unwrap();
        assert_eq!(head_long.method, "HEAD");

        let head_combined = parse_curl("curl -sIL https://example.com/healthz").unwrap();
        assert_eq!(head_combined.method, "HEAD");

        let head_explicit = parse_curl("curl -I -X GET https://example.com/healthz").unwrap();
        assert_eq!(head_explicit.method, "GET");

        // 3. Multiple -d / --data* flags joined with `&`, while single segment stays verbatim
        let multi_d = parse_curl(
            "curl https://example.com/form -d 'a=1' --data 'b=2' --data-raw 'c=3' --data-binary 'd=4' --data-ascii 'e=5'",
        )
        .unwrap();
        assert_eq!(multi_d.method, "POST");
        assert_eq!(multi_d.body, "a=1&b=2&c=3&d=4&e=5");

        let single_d = parse_curl("curl https://example.com/raw -d 'plain-body-no-amp'").unwrap();
        assert_eq!(single_d.method, "POST");
        assert_eq!(single_d.body, "plain-body-no-amp");

        // 4. --data-urlencode (name=content, =content, bare content, and inline --data-urlencode=...)
        let urlencoded = parse_curl(
            "curl https://example.com/submit --data-urlencode 'q=hello world & 100%' --data-urlencode '=raw=val' --data-urlencode='bare text' -d 'ok=1'",
        )
        .unwrap();
        assert_eq!(urlencoded.method, "POST");
        assert_eq!(
            urlencoded.body,
            "q=hello%20world%20%26%20100%25&raw%3Dval&bare%20text&ok=1"
        );

        // 5. -G / --get moves data to URL query (? or &), preserves #fragment, clears body, defaults to GET (unless -X)
        let get_fresh = parse_curl(
            "curl -G https://api.example.com/search#results -d 'page=2' --data-urlencode 'tag=a&b'",
        )
        .unwrap();
        assert_eq!(get_fresh.method, "GET");
        assert_eq!(
            get_fresh.url,
            "https://api.example.com/search?page=2&tag=a%26b#results"
        );
        assert!(get_fresh.body.is_empty());

        let get_existing_q = parse_curl(
            "curl --get 'https://api.example.com/search?lang=zh' -d 'limit=20' -d 'sort=desc'",
        )
        .unwrap();
        assert_eq!(get_existing_q.method, "GET");
        assert_eq!(
            get_existing_q.url,
            "https://api.example.com/search?lang=zh&limit=20&sort=desc"
        );
        assert!(get_existing_q.body.is_empty());

        let get_explicit_post =
            parse_curl("curl -G -X POST https://api.example.com/search -d 'dry_run=1'").unwrap();
        assert_eq!(get_explicit_post.method, "POST");
        assert_eq!(
            get_explicit_post.url,
            "https://api.example.com/search?dry_run=1"
        );
        assert!(get_explicit_post.body.is_empty());
    }
}
