//! Native Rust byte codec, compression, URL/form, JWT, and schema-less Protobuf
//! wire inspection helpers exposed to QuickJS hooks via `zniff.codec.*`.
//!
//! Reuses crates already linked into `zniff` (`flate2`, `zstd`,
//! `brotli_decompressor`, `serde_json`, `rquickjs`, and `crate::session`)
//! with zero new Cargo dependencies.

use std::io::{Read, Write};

use brotli_decompressor::Decompressor as BrotliDecompressor;
use flate2::Compression;
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use rquickjs::function::{IntoJsFunc, Opt};
use rquickjs::{
    Array, ArrayBuffer, Ctx, Exception, FromJs, Function, Object, Result as JsResult, TypedArray,
    Value,
};
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::hook_codec_crypto::{
    MAX_CODEC_INPUT_BYTES, MAX_DECOMPRESS_OUTPUT_BYTES, check_codec_input_cap, extract_body_input,
    extract_bytes_arg, hex_decode_str, hex_encode_bytes, hex_val, to_uint8_array,
};
use crate::session::{base64_decode, base64_encode};

const MAX_PROTO_WIRE_DEPTH: usize = 16;
const MAX_PROTO_WIRE_ENTRIES: usize = 512;
const MAX_PROTO_FIELD_NUM: u64 = (1_u64 << 29) - 1;
const JS_MAX_SAFE_INT: u64 = (1_u64 << 53) - 1;

fn decode_b64(text: &str, url_safe_only: bool) -> Result<Vec<u8>, String> {
    if text.len() > MAX_CODEC_INPUT_BYTES * 4 {
        return Err("base64 input exceeds bridge limit".to_string());
    }
    if url_safe_only {
        let clean: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        if clean.iter().any(|&b| b == b'+' || b == b'/') {
            return Err("invalid base64url character".to_string());
        }
        let no_pad_len = clean.iter().rposition(|&b| b != b'=').map_or(0, |i| i + 1);
        let pad_n = clean.len() - no_pad_len;
        let rem = no_pad_len % 4;
        if clean[..no_pad_len].contains(&b'=')
            || (pad_n > 0 && !(matches!(rem, 2 | 3) && rem + pad_n == 4))
        {
            return Err("invalid base64url padding".to_string());
        }
    }
    let decoded = base64_decode(text).ok_or_else(|| "invalid base64 input".to_string())?;
    if decoded.len() > MAX_CODEC_INPUT_BYTES {
        return Err("decoded base64 exceeds 2 MiB bridge limit".to_string());
    }
    Ok(decoded)
}

fn read_capped<R: Read>(reader: R, label: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    reader
        .take((MAX_DECOMPRESS_OUTPUT_BYTES + 1) as u64)
        .read_to_end(&mut out)
        .map_err(|e| format!("{label} failed: {e}"))?;
    if out.len() > MAX_DECOMPRESS_OUTPUT_BYTES {
        return Err(format!(
            "{label} output exceeds {MAX_DECOMPRESS_OUTPUT_BYTES} byte bomb limit"
        ));
    }
    Ok(out)
}

fn flate_c(bytes: &[u8], kind: u8) -> Result<Vec<u8>, String> {
    if bytes.len() > MAX_CODEC_INPUT_BYTES {
        return Err("compress input exceeds 2 MiB limit".to_string());
    }
    let lvl = Compression::default();
    let res = match kind {
        0 => {
            let mut e = GzEncoder::new(Vec::new(), lvl);
            e.write_all(bytes).and_then(|()| e.finish())
        }
        1 => {
            let mut e = DeflateEncoder::new(Vec::new(), lvl);
            e.write_all(bytes).and_then(|()| e.finish())
        }
        _ => {
            let mut e = ZlibEncoder::new(Vec::new(), lvl);
            e.write_all(bytes).and_then(|()| e.finish())
        }
    };
    res.map_err(|e| format!("compress failed: {e}"))
}

pub(crate) fn gzip_compress(bytes: &[u8]) -> Result<Vec<u8>, String> {
    flate_c(bytes, 0)
}

pub(crate) fn zstd_decompress(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let dec =
        zstd::stream::read::Decoder::new(bytes).map_err(|e| format!("zstd init failed: {e}"))?;
    read_capped(dec, "zstdDecompress")
}

fn pct_encode(out: &mut String, text: &str, form_mode: bool) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for &b in text.as_bytes() {
        let safe = b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'.')
            || b == if form_mode { b'*' } else { b'~' };
        if form_mode && b == b' ' {
            out.push('+');
        } else if safe {
            out.push(b as char);
        } else {
            out.extend([
                '%',
                HEX[(b >> 4) as usize] as char,
                HEX[(b & 0x0f) as usize] as char,
            ]);
        }
    }
}

fn pct_decode(text: &str, plus_as_space: bool) -> String {
    let bytes = text.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(bytes.len()), 0usize);
    while i < bytes.len() {
        let b = bytes[i];
        if plus_as_space && b == b'+' {
            out.push(b' ');
            i += 1;
        } else if b == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn extract_form_entries<'js>(cx: &Ctx<'js>, val: Value<'js>) -> JsResult<Vec<(String, String)>> {
    let err_t = |msg: &str| Exception::throw_type(cx, msg);
    if val.is_array() {
        let arr = Array::from_value(val)?;
        let mut pairs = Vec::with_capacity(arr.len());
        for item in arr.iter::<Value<'js>>() {
            let p = Array::from_value(item?).map_err(|_| err_t("entries must be [k, v]"))?;
            pairs.push((
                p.get(0).map_err(|_| err_t("key must be string"))?,
                p.get(1).map_err(|_| err_t("value must be string"))?,
            ));
        }
        return Ok(pairs);
    }
    if val.is_object()
        && TypedArray::<u8>::from_value(val.clone()).is_err()
        && ArrayBuffer::from_value(val.clone()).is_none()
        && let Some(obj) = val.as_object()
    {
        let mut pairs = Vec::new();
        for prop in obj.props::<String, Value<'js>>() {
            let (k, v) = prop?;
            if !v.is_null() && !v.is_undefined() {
                pairs.push((
                    k,
                    String::from_js(cx, v).map_err(|_| err_t("values must be strings"))?,
                ));
            }
        }
        return Ok(pairs);
    }
    Err(err_t("formUrlEncode expected [k, v][] or object"))
}

fn decode_jwt<'js>(cx: &Ctx<'js>, token: &str) -> JsResult<Value<'js>> {
    check_codec_input_cap(cx, token.len())?;
    let trimmed = token.trim();
    let raw = match trimmed.split_once(char::is_whitespace) {
        Some((p, rest)) if p.eq_ignore_ascii_case("bearer") => rest.trim_start(),
        _ => trimmed,
    };
    let parts: Vec<&str> = raw.split('.').collect();
    let [hb, pb, sb] = parts.as_slice() else {
        return Ok(Value::new_null(cx.clone()));
    };
    let parse_part = |s: &str| -> Option<Value<'js>> {
        let raw = decode_b64(s, true).ok()?;
        let txt = std::str::from_utf8(&raw).ok()?;
        cx.json_parse(txt.as_bytes()).ok().or_else(|| {
            if cx.has_exception() {
                let _ = cx.catch();
            }
            None
        })
    };
    if raw.bytes().any(|b| b.is_ascii_whitespace()) || hb.is_empty() || pb.is_empty() {
        return Ok(Value::new_null(cx.clone()));
    }
    let (Some(header), Some(payload), Ok(sr)) =
        (parse_part(hb), parse_part(pb), decode_b64(sb, true))
    else {
        return Ok(Value::new_null(cx.clone()));
    };
    let out = Object::new(cx.clone())?;
    out.set("header", header)?;
    out.set("payload", payload)?;
    out.set("signingInput", format!("{hb}.{pb}"))?;
    out.set("signatureHex", hex_encode_bytes(&sr))?;
    Ok(out.into_value())
}

enum WireErr {
    Invalid,
    Limit(&'static str),
}

fn read_varint(buf: &[u8], pos: &mut usize) -> Result<u64, WireErr> {
    let (mut acc, mut shift) = (0_u64, 0_u32);
    while *pos < buf.len() && shift < 64 {
        let b = buf[*pos];
        *pos += 1;
        if shift == 63 && b > 1 {
            return Err(WireErr::Invalid);
        }
        acc |= u64::from(b & 0x7f) << shift;
        if (b & 0x80) == 0 {
            return Ok(acc);
        }
        shift += 7;
    }
    Err(WireErr::Invalid)
}

fn read_fixed<const N: usize>(buf: &[u8], pos: &mut usize) -> Result<[u8; N], WireErr> {
    let raw: [u8; N] = buf
        .get(*pos..*pos + N)
        .and_then(|s| s.try_into().ok())
        .ok_or(WireErr::Invalid)?;
    *pos += N;
    Ok(raw)
}

fn decode_len_delimited(
    slice: &[u8],
    depth: usize,
    entries: &mut usize,
) -> Result<JsonValue, WireErr> {
    if !slice.is_empty() && depth < MAX_PROTO_WIRE_DEPTH {
        let saved = *entries;
        match decode_proto_msg(slice, depth + 1, entries) {
            Ok(nested) if !nested.is_empty() => return Ok(JsonValue::Object(nested)),
            Ok(_) | Err(WireErr::Invalid) => *entries = saved,
            Err(err @ WireErr::Limit(_)) => return Err(err),
        }
    }
    if let Ok(s) = std::str::from_utf8(slice)
        && !s
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Ok(JsonValue::String(s.to_string()));
    }
    Ok(JsonValue::String(format!("0x{}", hex_encode_bytes(slice))))
}

fn decode_proto_msg(
    buf: &[u8],
    depth: usize,
    entries: &mut usize,
) -> Result<JsonMap<String, JsonValue>, WireErr> {
    let (mut map, mut pos) = (JsonMap::new(), 0_usize);
    while pos < buf.len() {
        let tag = read_varint(buf, &mut pos)?;
        let (field_num, wire_type) = (tag >> 3, (tag & 0x07) as u8);
        if field_num == 0 || field_num > MAX_PROTO_FIELD_NUM {
            return Err(WireErr::Invalid);
        }
        *entries += 1;
        if *entries > MAX_PROTO_WIRE_ENTRIES {
            return Err(WireErr::Limit(
                "protobuf wire exceeds 512 field entries limit",
            ));
        }
        let val = match wire_type {
            0 => match read_varint(buf, &mut pos)? {
                v if v > JS_MAX_SAFE_INT => JsonValue::String(v.to_string()),
                v => JsonValue::Number(v.into()),
            },
            1 => JsonValue::String(format!(
                "0x{:016x}",
                u64::from_le_bytes(read_fixed::<8>(buf, &mut pos)?)
            )),
            2 => {
                let len =
                    usize::try_from(read_varint(buf, &mut pos)?).map_err(|_| WireErr::Invalid)?;
                let slice = buf
                    .get(pos..pos.saturating_add(len))
                    .filter(|s| s.len() == len)
                    .ok_or(WireErr::Invalid)?;
                pos += len;
                decode_len_delimited(slice, depth, entries)?
            }
            5 => JsonValue::Number(u32::from_le_bytes(read_fixed::<4>(buf, &mut pos)?).into()),
            _ => return Err(WireErr::Invalid),
        };
        let key = field_num.to_string();
        match map.get_mut(&key) {
            Some(JsonValue::Array(arr)) => arr.push(val),
            Some(old) => *old = JsonValue::Array(vec![old.take(), val]),
            None => {
                map.insert(key, val);
            }
        }
    }
    Ok(map)
}

pub(crate) fn protobuf_decode_wire(bytes: &[u8]) -> Result<JsonValue, String> {
    if bytes.len() > MAX_CODEC_INPUT_BYTES {
        return Err("protobuf input exceeds 2 MiB bridge limit".to_string());
    }
    let mut entries = 0_usize;
    match decode_proto_msg(bytes, 1, &mut entries) {
        Ok(map) => Ok(JsonValue::Object(map)),
        Err(WireErr::Invalid) => Err("invalid protobuf wire payload".to_string()),
        Err(WireErr::Limit(msg)) => Err(msg.to_string()),
    }
}

struct Binder<'a, 'js>(&'a Object<'js>, &'a Ctx<'js>);

impl<'a, 'js> Binder<'a, 'js> {
    fn bind<P, F: IntoJsFunc<'js, P> + 'js>(&self, name: &str, f: F) -> JsResult<()> {
        self.0.set(name, Function::new(self.1.clone(), f)?)
    }

    fn body_str(&self, name: &str, f: fn(&[u8]) -> String) -> JsResult<()> {
        self.bind(
            name,
            move |cx: Ctx<'js>, val: Value<'js>| -> JsResult<String> {
                Ok(f(&extract_body_input(&cx, val)?))
            },
        )
    }

    fn str_str(&self, name: &str, f: fn(&str) -> String) -> JsResult<()> {
        self.bind(name, move |cx: Ctx<'js>, s: String| -> JsResult<String> {
            check_codec_input_cap(&cx, s.len())?;
            Ok(f(&s))
        })
    }

    fn str_bytes(&self, name: &str, f: fn(&str) -> Result<Vec<u8>, String>) -> JsResult<()> {
        self.bind(name, move |cx: Ctx<'js>, text: String| {
            let out = f(&text).map_err(|e| {
                if e.contains("limit") {
                    Exception::throw_range(&cx, &e)
                } else {
                    Exception::throw_type(&cx, &e)
                }
            })?;
            to_uint8_array(&cx, out)
        })
    }

    fn bytes_io(
        &self,
        name: &str,
        enc: bool,
        f: fn(&[u8]) -> Result<Vec<u8>, String>,
    ) -> JsResult<()> {
        self.bind(name, move |cx: Ctx<'js>, val: Value<'js>| {
            let b = (if enc {
                extract_body_input
            } else {
                extract_bytes_arg
            })(&cx, val)?;
            to_uint8_array(&cx, f(&b).map_err(|e| Exception::throw_message(&cx, &e))?)
        })
    }
}

fn zstd_compress_js<'js>(
    cx: Ctx<'js>,
    val: Value<'js>,
    opt: Opt<Value<'js>>,
) -> JsResult<TypedArray<'js, u8>> {
    let bytes = extract_body_input(&cx, val)?;
    let lvl = if let Some(v) = opt.0.filter(|v| !v.is_undefined() && !v.is_null()) {
        let n =
            f64::from_js(&cx, v).map_err(|_| Exception::throw_type(&cx, "level must be number"))?;
        if !n.is_finite() {
            return Err(Exception::throw_range(&cx, "level must be finite"));
        }
        (n.round() as i32).clamp(1, 9)
    } else {
        3
    };
    let out = zstd::stream::encode_all(bytes.as_slice(), lvl)
        .map_err(|e| Exception::throw_message(&cx, &format!("zstdCompress failed: {e}")))?;
    to_uint8_array(&cx, out)
}

fn form_encode_js<'js>(cx: Ctx<'js>, val: Value<'js>) -> JsResult<String> {
    let mut out = String::new();
    for (idx, (k, v)) in extract_form_entries(&cx, val)?.iter().enumerate() {
        if idx > 0 {
            out.push('&');
        }
        pct_encode(&mut out, k, true);
        out.push('=');
        pct_encode(&mut out, v, true);
        check_codec_input_cap(&cx, out.len())?;
    }
    Ok(out)
}

fn form_decode_js<'js>(cx: Ctx<'js>, query: String) -> JsResult<Array<'js>> {
    check_codec_input_cap(&cx, query.len())?;
    let trimmed = query.strip_prefix('?').unwrap_or(&query);
    let arr = Array::new(cx.clone())?;
    for (idx, seg) in trimmed.split('&').filter(|s| !s.is_empty()).enumerate() {
        let (k, v) = seg.split_once('=').unwrap_or((seg, ""));
        let pair = Array::new(cx.clone())?;
        pair.set(0, pct_decode(k, true))?;
        pair.set(1, pct_decode(v, true))?;
        arr.set(idx, pair)?;
    }
    Ok(arr)
}

fn register_text_and_compression<'js>(b: &Binder<'_, 'js>) -> JsResult<()> {
    b.bind("utf8Encode", |cx: Ctx<'js>, s: String| {
        check_codec_input_cap(&cx, s.len())?;
        to_uint8_array(&cx, s.into_bytes())
    })?;
    b.bind(
        "utf8Decode",
        |cx: Ctx<'js>, v: Value<'js>| -> JsResult<String> {
            Ok(String::from_utf8_lossy(&extract_bytes_arg(&cx, v)?).into_owned())
        },
    )?;
    b.body_str("base64Encode", base64_encode)?;
    b.str_bytes("base64Decode", |s| decode_b64(s, false))?;
    b.body_str("base64UrlEncode", |d| {
        base64_encode(d)
            .trim_end_matches('=')
            .replace('+', "-")
            .replace('/', "_")
    })?;
    b.str_bytes("base64UrlDecode", |s| decode_b64(s, true))?;
    b.body_str("hexEncode", hex_encode_bytes)?;
    b.str_bytes("hexDecode", hex_decode_str)?;
    b.bytes_io("gzip", true, gzip_compress)?;
    b.bytes_io("gunzip", false, |d| {
        read_capped(GzDecoder::new(d), "gunzip")
    })?;
    b.bytes_io("deflate", true, |d| flate_c(d, 1))?;
    b.bytes_io("inflate", false, |d| {
        read_capped(DeflateDecoder::new(d), "inflate")
    })?;
    b.bytes_io("zlibCompress", true, |d| flate_c(d, 2))?;
    b.bytes_io("zlibDecompress", false, |d| {
        read_capped(ZlibDecoder::new(d), "zlibDecompress")
    })?;
    b.bind("zstdCompress", zstd_compress_js)?;
    b.bytes_io("zstdDecompress", false, zstd_decompress)?;
    b.bytes_io("brotliDecompress", false, |d| {
        read_capped(BrotliDecompressor::new(d, 4096), "brotliDecompress")
    })
}

fn register_web_and_wire<'js>(b: &Binder<'_, 'js>) -> JsResult<()> {
    b.str_str("urlEncode", |s| {
        let mut o = String::with_capacity(s.len());
        pct_encode(&mut o, s, false);
        o
    })?;
    b.str_str("urlDecode", |s| pct_decode(s, false))?;
    b.bind("formUrlEncode", form_encode_js)?;
    b.bind("formUrlDecode", form_decode_js)?;
    b.bind("crc32", |cx: Ctx<'js>, val: Value<'js>| -> JsResult<f64> {
        let mut crc = flate2::Crc::new();
        crc.update(&extract_body_input(&cx, val)?);
        Ok(f64::from(crc.sum()))
    })?;
    b.bind("jwtDecode", |cx: Ctx<'js>, s: String| decode_jwt(&cx, &s))?;
    b.bind("protobufDecodeWire", |cx: Ctx<'js>, val: Value<'js>| {
        let json = protobuf_decode_wire(&extract_bytes_arg(&cx, val)?)
            .map_err(|e| Exception::throw_message(&cx, &e))?;
        let encoded = serde_json::to_string(&json)
            .map_err(|e| Exception::throw_message(&cx, &e.to_string()))?;
        cx.json_parse(encoded.into_bytes())
    })
}

/// Builds the `zniff.codec` JS helper object.
pub(crate) fn build_codec_object<'js>(cx: &Ctx<'js>) -> JsResult<Object<'js>> {
    let obj = Object::new(cx.clone())?;
    let binder = Binder(&obj, cx);
    register_text_and_compression(&binder)?;
    register_web_and_wire(&binder)?;
    Ok(obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rquickjs::{Context, Runtime};

    fn eval_js<T: for<'js> FromJs<'js>>(script: &str) -> T {
        let rt = Runtime::new().expect("quickjs runtime");
        let ctx = Context::full(&rt).expect("quickjs context");
        ctx.with(|cx| {
            let zniff = Object::new(cx.clone()).unwrap();
            zniff
                .set("codec", build_codec_object(&cx).unwrap())
                .unwrap();
            cx.globals().set("zniff", zniff).unwrap();
            cx.eval(script).expect("eval script")
        })
    }

    fn eval_js_fails(script: &str) -> bool {
        let rt = Runtime::new().expect("quickjs runtime");
        let ctx = Context::full(&rt).expect("quickjs context");
        ctx.with(|cx| {
            let zniff = Object::new(cx.clone()).unwrap();
            zniff
                .set("codec", build_codec_object(&cx).unwrap())
                .unwrap();
            cx.globals().set("zniff", zniff).unwrap();
            cx.eval::<Value<'_>, _>(script).is_err()
        })
    }

    fn encode_varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
    }

    fn encode_tag(field_num: u32, wire_type: u8, out: &mut Vec<u8>) {
        encode_varint((u64::from(field_num) << 3) | u64::from(wire_type), out);
    }

    fn encode_ld_field(field_num: u32, payload: &[u8], out: &mut Vec<u8>) {
        encode_tag(field_num, 2, out);
        encode_varint(payload.len() as u64, out);
        out.extend_from_slice(payload);
    }

    #[test]
    fn text_base64_base64url_and_hex_roundtrips_via_quickjs() {
        let ok: bool = eval_js(
            r#"
            const src = "https://api.vicanso.dev/v1/解码?k=你好";
            const bytes = zniff.codec.utf8Encode(src);
            const ab = bytes.buffer.slice(0);
            const sub = new Uint8Array(bytes.buffer, 0, 5);
            const b64 = zniff.codec.base64Encode(src);
            const b64Ab = zniff.codec.base64Encode(ab);
            const b64Url = zniff.codec.base64UrlEncode(new Uint8Array([0xfb, 0xff, 0xbf]));
            const b64UrlDec = zniff.codec.base64UrlDecode("  -_-- \n");
            const paddedUrl = zniff.codec.base64UrlDecode("YQ==");
            const hexStr = zniff.codec.hexEncode("vicanso.dev");
            const hexBack = zniff.codec.utf8Decode(zniff.codec.hexDecode(hexStr));
            b64 === b64Ab
              && zniff.codec.utf8Decode(zniff.codec.base64Decode(b64)) === src
              && zniff.codec.utf8Decode(sub) === "https"
              && b64Url === "-_-_" && !b64Url.includes("=")
              && b64UrlDec.length === 3 && b64UrlDec[0] === 0xfb && b64UrlDec[1] === 0xff
              && zniff.codec.utf8Decode(paddedUrl) === "a"
              && hexBack === "vicanso.dev"
            "#,
        );
        assert!(ok);
        assert!(eval_js_fails(r#"zniff.codec.base64UrlDecode("a+b/")"#));
        assert!(eval_js_fails(r#"zniff.codec.base64UrlDecode("YQ=")"#));
        assert!(eval_js_fails(r#"zniff.codec.hexDecode("abc")"#));
        assert!(eval_js_fails(
            r#"zniff.codec.utf8Encode("x".repeat(2 * 1024 * 1024 + 1))"#
        ));
    }

    #[test]
    fn compression_roundtrips_and_zip_bomb_protection_all_formats() {
        let ok: bool = eval_js(
            r#"
            const sample = JSON.stringify({ host: "api.vicanso.dev", items: [1, 2, 3, 4] });
            const g = zniff.codec.utf8Decode(zniff.codec.gunzip(zniff.codec.gzip(sample)));
            const d = zniff.codec.utf8Decode(zniff.codec.inflate(zniff.codec.deflate(sample)));
            const z = zniff.codec.utf8Decode(zniff.codec.zlibDecompress(zniff.codec.zlibCompress(sample)));
            const zsDefault = zniff.codec.utf8Decode(zniff.codec.zstdDecompress(zniff.codec.zstdCompress(sample)));
            const zsClamp = zniff.codec.utf8Decode(zniff.codec.zstdDecompress(zniff.codec.zstdCompress(sample, 99)));
            g === sample && d === sample && z === sample && zsDefault === sample && zsClamp === sample
            "#,
        );
        assert!(ok);

        let plain = br#"{"host":"cdn.vicanso.dev","ok":true}"#;
        let mut br_bytes = Vec::new();
        {
            let mut br = brotli::CompressorWriter::new(&mut br_bytes, 4096, 5, 22);
            br.write_all(plain).unwrap();
        }
        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        let br_out: String = ctx.with(|cx| {
            let zniff = Object::new(cx.clone()).unwrap();
            zniff
                .set("codec", build_codec_object(&cx).unwrap())
                .unwrap();
            cx.globals().set("zniff", zniff).unwrap();
            cx.globals()
                .set("brInput", to_uint8_array(&cx, br_bytes).unwrap())
                .unwrap();
            cx.eval("zniff.codec.utf8Decode(zniff.codec.brotliDecompress(brInput))")
                .unwrap()
        });
        assert_eq!(br_out.as_bytes(), plain);

        let bomb = vec![0_u8; MAX_DECOMPRESS_OUTPUT_BYTES + 4096];
        let gz_bomb = {
            let mut e = GzEncoder::new(Vec::new(), Compression::fast());
            e.write_all(&bomb).unwrap();
            e.finish().unwrap()
        };
        let def_bomb = {
            let mut e = DeflateEncoder::new(Vec::new(), Compression::fast());
            e.write_all(&bomb).unwrap();
            e.finish().unwrap()
        };
        let zlib_bomb = {
            let mut e = ZlibEncoder::new(Vec::new(), Compression::fast());
            e.write_all(&bomb).unwrap();
            e.finish().unwrap()
        };
        let zstd_bomb = zstd::stream::encode_all(bomb.as_slice(), 1).unwrap();
        let mut br_bomb = Vec::new();
        {
            let mut br = brotli::CompressorWriter::new(&mut br_bomb, 4096, 1, 22);
            br.write_all(&bomb).unwrap();
        }
        for (method, compressed) in [
            ("gunzip", gz_bomb),
            ("inflate", def_bomb),
            ("zlibDecompress", zlib_bomb),
            ("zstdDecompress", zstd_bomb),
            ("brotliDecompress", br_bomb),
        ] {
            let blocked: bool = ctx.with(|cx| {
                cx.globals()
                    .set("bombIn", to_uint8_array(&cx, compressed).unwrap())
                    .unwrap();
                cx.eval::<Value<'_>, _>(format!("zniff.codec.{method}(bombIn)"))
                    .is_err()
            });
            assert!(blocked, "{method} must reject zip bombs > 2 MiB");
        }
    }

    #[test]
    fn url_form_crc32_and_jwt_decode_behaviors() {
        let ok: bool = eval_js(
            r#"
            const urlEnc = zniff.codec.urlEncode("https://api.vicanso.dev/q?a=1 2&t=你好!~");
            const urlDec = zniff.codec.urlDecode("a+b%20%E4%BD%A0%E5%A5%BD%21");
            const formArr = zniff.codec.formUrlEncode([["host", "api.vicanso.dev"], ["q", "hello world!"]]);
            const formObj = zniff.codec.formUrlEncode({ host: "api.vicanso.dev", q: "hello world!" });
            const pairs = zniff.codec.formUrlDecode("?host=api.vicanso.dev&q=hello+world%21&flag");
            const crc = zniff.codec.crc32("123456789");

            const hdr = zniff.codec.base64UrlEncode(JSON.stringify({ alg: "HS256", typ: "JWT" }));
            const pay = zniff.codec.base64UrlEncode(JSON.stringify({ iss: "auth.vicanso.dev", sub: "u1" }));
            const sig = zniff.codec.base64UrlEncode(new Uint8Array([0xde, 0xad, 0xbe, 0xef]));
            const jwt = zniff.codec.jwtDecode("  bearer  " + hdr + "." + pay + "." + sig + "  ");
            const badFmt = zniff.codec.jwtDecode("only.two");
            const badJson = zniff.codec.jwtDecode(zniff.codec.base64UrlEncode("{bad") + "." + pay + "." + sig);

            urlEnc === "https%3A%2F%2Fapi.vicanso.dev%2Fq%3Fa%3D1%202%26t%3D%E4%BD%A0%E5%A5%BD%21~"
              && urlDec === "a+b 你好!"
              && formArr === "host=api.vicanso.dev&q=hello+world%21"
              && formObj === formArr
              && JSON.stringify(pairs) === '[["host","api.vicanso.dev"],["q","hello world!"],["flag",""]]'
              && crc === 0xCBF43926
              && jwt !== null
              && jwt.header.alg === "HS256"
              && jwt.payload.iss === "auth.vicanso.dev"
              && jwt.signingInput === hdr + "." + pay
              && jwt.signatureHex === "deadbeef"
              && badFmt === null
              && badJson === null
            "#,
        );
        assert!(ok);
        assert!(eval_js_fails(
            r#"zniff.codec.formUrlEncode(new Uint8Array([1, 2]))"#
        ));
    }

    #[test]
    fn protobuf_decode_wire_types_nesting_repeated_and_limits() {
        let mut nested = Vec::new();
        encode_ld_field(1, b"api.vicanso.dev", &mut nested);
        encode_tag(2, 0, &mut nested);
        encode_varint(200, &mut nested);

        let mut payload = Vec::new();
        encode_tag(1, 0, &mut payload);
        encode_varint(42, &mut payload);
        encode_tag(2, 0, &mut payload);
        encode_varint((1_u64 << 53) + 7, &mut payload);
        encode_tag(3, 1, &mut payload);
        payload.extend_from_slice(&0x0102_0304_0506_0708_u64.to_le_bytes());
        encode_tag(4, 5, &mut payload);
        payload.extend_from_slice(&0x1122_3344_u32.to_le_bytes());
        encode_ld_field(5, &nested, &mut payload);
        encode_ld_field(6, b"a.vicanso.dev", &mut payload);
        encode_ld_field(6, b"b.vicanso.dev", &mut payload);
        encode_ld_field(7, &[0xff, 0x00, 0xab], &mut payload);

        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        let ok: bool = ctx.with(|cx| {
            let zniff = Object::new(cx.clone()).unwrap();
            zniff
                .set("codec", build_codec_object(&cx).unwrap())
                .unwrap();
            cx.globals().set("zniff", zniff).unwrap();
            cx.globals()
                .set("wire", to_uint8_array(&cx, payload).unwrap())
                .unwrap();
            cx.eval(
                r#"
                const d = zniff.codec.protobufDecodeWire(wire);
                d["1"] === 42
                  && d["2"] === "9007199254740999"
                  && d["3"] === "0x0102030405060708"
                  && d["4"] === 0x11223344
                  && d["5"]["1"] === "api.vicanso.dev"
                  && d["5"]["2"] === 200
                  && Array.isArray(d["6"])
                  && d["6"][0] === "a.vicanso.dev"
                  && d["6"][1] === "b.vicanso.dev"
                  && d["7"] === "0xff00ab"
                "#,
            )
            .unwrap()
        });
        assert!(ok);

        let mut deep = Vec::new();
        encode_tag(1, 0, &mut deep);
        encode_varint(7, &mut deep);
        for _ in 0..18 {
            let mut outer = Vec::new();
            encode_ld_field(1, &deep, &mut outer);
            deep = outer;
        }
        let decoded_deep = protobuf_decode_wire(&deep).expect("depth capped cleanly");
        let mut cursor = &decoded_deep;
        for _ in 0..16 {
            cursor = cursor.get("1").expect("nested layer");
        }
        assert!(cursor.as_str().is_some_and(|s| s.starts_with("0x")));

        let mut many = Vec::new();
        for i in 1..=520_u32 {
            encode_tag(i, 0, &mut many);
            encode_varint(1, &mut many);
        }
        assert!(
            protobuf_decode_wire(&many)
                .unwrap_err()
                .contains("512 field entries")
        );
    }
}
