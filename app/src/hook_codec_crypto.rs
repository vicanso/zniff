//! Shared QuickJS byte/hex boundary helpers and `zniff.codec` / `zniff.crypto`
//! re-exports for hook bridges.

use rquickjs::{ArrayBuffer, Ctx, Exception, Result as JsResult, TypedArray, Value};

use crate::session::BODY_CAP;

/// Maximum input payload accepted by codec/crypto bridge calls (2 MiB).
pub const MAX_CODEC_INPUT_BYTES: usize = 2 * BODY_CAP;

/// Maximum decompressed output size allowed from decompressors (2 MiB).
pub const MAX_DECOMPRESS_OUTPUT_BYTES: usize = 2 * BODY_CAP;

pub(crate) use crate::hook_codec::build_codec_object;
pub(crate) use crate::hook_crypto::build_crypto_object;

#[cfg(test)]
pub(crate) use crate::hook_codec::gzip_compress;

#[cfg(test)]
pub(crate) fn aes_gcm_encrypt(
    algo: &'static ring::aead::Algorithm,
    key_bytes: &[u8],
    nonce_bytes: &[u8],
    plaintext: &[u8],
    aad_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    if plaintext.len() > MAX_CODEC_INPUT_BYTES {
        return Err("plaintext exceeds 2 MiB bridge limit".to_string());
    }
    crate::hook_crypto::aead_encrypt(
        algo,
        "AES-GCM",
        key_bytes,
        nonce_bytes,
        plaintext,
        aad_bytes,
    )
}

pub(crate) fn check_codec_input_cap<'js>(cx: &Ctx<'js>, len: usize) -> JsResult<()> {
    if len > MAX_CODEC_INPUT_BYTES {
        Err(Exception::throw_range(cx, "input exceeds 2 MiB limit"))
    } else {
        Ok(())
    }
}

fn copy_bounded<'js>(cx: &Ctx<'js>, bytes: Option<&[u8]>, detached: &str) -> JsResult<Vec<u8>> {
    let bytes = bytes.ok_or_else(|| Exception::throw_type(cx, detached))?;
    check_codec_input_cap(cx, bytes.len())?;
    Ok(bytes.to_vec())
}

fn extract_binary<'js>(cx: &Ctx<'js>, val: Value<'js>, err: &str) -> JsResult<Vec<u8>> {
    if let Ok(ta) = TypedArray::<u8>::from_value(val.clone()) {
        return copy_bounded(cx, unsafe { ta.as_bytes() }, "Uint8Array is detached");
    }
    if let Some(ab) = ArrayBuffer::from_value(val) {
        return copy_bounded(cx, unsafe { ab.as_bytes() }, "ArrayBuffer is detached");
    }
    Err(Exception::throw_type(cx, err))
}

pub(crate) fn extract_bytes_arg<'js>(cx: &Ctx<'js>, val: Value<'js>) -> JsResult<Vec<u8>> {
    extract_binary(cx, val, "expected Uint8Array or ArrayBuffer")
}

pub(crate) fn extract_body_input<'js>(cx: &Ctx<'js>, val: Value<'js>) -> JsResult<Vec<u8>> {
    if let Some(s) = val.as_string() {
        let text = s.to_string()?;
        check_codec_input_cap(cx, text.len())?;
        return Ok(text.into_bytes());
    }
    extract_binary(cx, val, "expected Uint8Array, ArrayBuffer, or string")
}

pub(crate) fn to_uint8_array<'js>(cx: &Ctx<'js>, bytes: Vec<u8>) -> JsResult<TypedArray<'js, u8>> {
    TypedArray::<u8>::new(cx.clone(), bytes)
}

pub(crate) fn hex_encode_bytes(bytes: &[u8]) -> String {
    const LUT: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(LUT[(b >> 4) as usize] as char);
        out.push(LUT[(b & 0x0f) as usize] as char);
    }
    out
}

pub(crate) const fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn hex_decode_str(text: &str) -> Result<Vec<u8>, String> {
    if text.len() > MAX_CODEC_INPUT_BYTES * 4 {
        return Err("hex input exceeds limit".to_string());
    }
    let clean: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !clean.len().is_multiple_of(2) {
        return Err("hex string must have even nibbles".to_string());
    }
    if clean.len() / 2 > MAX_CODEC_INPUT_BYTES {
        return Err("decoded hex exceeds 2 MiB limit".to_string());
    }
    clean
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[a, b]| match (hex_val(*a), hex_val(*b)) {
            (Some(hi), Some(lo)) => Ok((hi << 4) | lo),
            _ => Err("invalid hex character".to_string()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rquickjs::{Context, Runtime};

    #[test]
    fn shared_byte_helpers_hex_and_caps() {
        let raw = br#"{"host":"api.vicanso.dev","seq":42}"#;
        let hex = hex_encode_bytes(raw);
        assert_eq!(hex_decode_str(&hex).unwrap(), raw);
        assert!(hex_decode_str("abc").is_err());

        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        ctx.with(|cx| {
            let s_val = cx.eval::<Value<'_>, _>("'api.vicanso.dev'").unwrap();
            assert_eq!(extract_body_input(&cx, s_val).unwrap(), b"api.vicanso.dev");
            let s_val2 = cx.eval::<Value<'_>, _>("'api.vicanso.dev'").unwrap();
            assert!(extract_bytes_arg(&cx, s_val2).is_err());

            let ta_val = cx
                .eval::<Value<'_>, _>("new Uint8Array([1, 2, 3])")
                .unwrap();
            assert_eq!(extract_bytes_arg(&cx, ta_val).unwrap(), [1, 2, 3]);
            let ab_val = cx
                .eval::<Value<'_>, _>("new Uint8Array([4, 5]).buffer")
                .unwrap();
            assert_eq!(extract_body_input(&cx, ab_val).unwrap(), [4, 5]);
        });
    }
}
