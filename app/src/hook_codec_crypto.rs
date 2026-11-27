//! Native Rust byte codec and cryptographic helpers exposed to QuickJS hooks
//! via `zniff.codec.*` and `zniff.crypto.*`.
//!
//! Reuses crates already linked into `zniff` (`ring`, `flate2`, `zstd`, and
//! `crate::rule_vars` / `crate::session` helpers) for 0 B binary bloat.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use ring::aead::{AES_128_GCM, AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use rquickjs::function::Opt;
use rquickjs::{
    ArrayBuffer, Ctx, Exception, Function, Object, Result as JsResult, TypedArray, Value,
};

use crate::rule_vars::{hmac_sha256, sha256};
use crate::session::{BODY_CAP, base64_decode, base64_encode};

/// Maximum input payload accepted by codec/crypto bridge calls (2 MiB).
pub const MAX_CODEC_INPUT_BYTES: usize = 2 * BODY_CAP;

/// Maximum decompressed output size allowed from `gunzip` / `zstdDecompress` (2 MiB).
pub const MAX_DECOMPRESS_OUTPUT_BYTES: usize = 2 * BODY_CAP;

pub(crate) fn extract_bytes_arg<'js>(cx: &Ctx<'js>, val: Value<'js>) -> JsResult<Vec<u8>> {
    if let Ok(ta) = TypedArray::<u8>::from_value(val.clone()) {
        let bytes = unsafe { ta.as_bytes() }
            .ok_or_else(|| Exception::throw_type(cx, "Uint8Array is detached"))?;
        if bytes.len() > MAX_CODEC_INPUT_BYTES {
            return Err(Exception::throw_range(
                cx,
                "byte input exceeds 2 MiB bridge limit",
            ));
        }
        return Ok(bytes.to_vec());
    }
    if let Some(ab) = ArrayBuffer::from_value(val) {
        let bytes = unsafe { ab.as_bytes() }
            .ok_or_else(|| Exception::throw_type(cx, "ArrayBuffer is detached"))?;
        if bytes.len() > MAX_CODEC_INPUT_BYTES {
            return Err(Exception::throw_range(
                cx,
                "byte input exceeds 2 MiB bridge limit",
            ));
        }
        return Ok(bytes.to_vec());
    }
    Err(Exception::throw_type(
        cx,
        "expected Uint8Array or ArrayBuffer",
    ))
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

pub(crate) fn hex_decode_str(text: &str) -> Result<Vec<u8>, String> {
    let clean: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !clean.len().is_multiple_of(2) {
        return Err("hex string must have an even number of nibbles".to_string());
    }
    if clean.len() / 2 > MAX_CODEC_INPUT_BYTES {
        return Err("decoded hex exceeds 2 MiB bridge limit".to_string());
    }
    let mut out = Vec::with_capacity(clean.len() / 2);
    for pair in clean.as_chunks::<2>().0 {
        let hi = hex_val(pair[0]).ok_or_else(|| "invalid hex character".to_string())?;
        let lo = hex_val(pair[1]).ok_or_else(|| "invalid hex character".to_string())?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

const fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn gzip_compress(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() > MAX_CODEC_INPUT_BYTES {
        return Err("gzip input exceeds 2 MiB limit".to_string());
    }
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(bytes)
        .map_err(|err| format!("gzip compress failed: {err}"))?;
    enc.finish()
        .map_err(|err| format!("gzip finish failed: {err}"))
}

pub(crate) fn gzip_decompress(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let decoder = GzDecoder::new(bytes);
    read_capped(decoder, "gunzip")
}

pub(crate) fn zstd_decompress(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let decoder = zstd::stream::read::Decoder::new(bytes)
        .map_err(|err| format!("zstd init failed: {err}"))?;
    read_capped(decoder, "zstdDecompress")
}

fn read_capped<R: Read>(reader: R, label: &str) -> Result<Vec<u8>, String> {
    let mut limited = reader.take((MAX_DECOMPRESS_OUTPUT_BYTES + 1) as u64);
    let mut out = Vec::new();
    limited
        .read_to_end(&mut out)
        .map_err(|err| format!("{label} failed: {err}"))?;
    if out.len() > MAX_DECOMPRESS_OUTPUT_BYTES {
        return Err(format!(
            "{label} output exceeds {MAX_DECOMPRESS_OUTPUT_BYTES} byte bomb limit"
        ));
    }
    Ok(out)
}

pub(crate) fn aes_gcm_encrypt(
    algo: &'static ring::aead::Algorithm,
    key_bytes: &[u8],
    nonce_bytes: &[u8],
    plaintext: &[u8],
    aad_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    if key_bytes.len() != algo.key_len() {
        return Err(format!(
            "AES-GCM key must be {} bytes (got {})",
            algo.key_len(),
            key_bytes.len()
        ));
    }
    if nonce_bytes.len() != 12 {
        return Err(format!(
            "AES-GCM nonce must be 12 bytes (got {})",
            nonce_bytes.len()
        ));
    }
    if plaintext.len() > MAX_CODEC_INPUT_BYTES {
        return Err("plaintext exceeds 2 MiB bridge limit".to_string());
    }
    let unbound = UnboundKey::new(algo, key_bytes).map_err(|_| "invalid AES key".to_string())?;
    let key = LessSafeKey::new(unbound);
    let nonce = Nonce::try_assume_unique_for_key(nonce_bytes)
        .map_err(|_| "invalid AES-GCM nonce".to_string())?;
    let mut buf = plaintext.to_vec();
    key.seal_in_place_append_tag(nonce, Aad::from(aad_bytes), &mut buf)
        .map_err(|_| "AES-GCM encrypt failed".to_string())?;
    Ok(buf)
}

pub(crate) fn aes_gcm_decrypt(
    algo: &'static ring::aead::Algorithm,
    key_bytes: &[u8],
    nonce_bytes: &[u8],
    ciphertext_with_tag: &[u8],
    aad_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    if key_bytes.len() != algo.key_len() {
        return Err(format!(
            "AES-GCM key must be {} bytes (got {})",
            algo.key_len(),
            key_bytes.len()
        ));
    }
    if nonce_bytes.len() != 12 {
        return Err(format!(
            "AES-GCM nonce must be 12 bytes (got {})",
            nonce_bytes.len()
        ));
    }
    if ciphertext_with_tag.len() > MAX_CODEC_INPUT_BYTES {
        return Err("ciphertext exceeds 2 MiB bridge limit".to_string());
    }
    let unbound = UnboundKey::new(algo, key_bytes).map_err(|_| "invalid AES key".to_string())?;
    let key = LessSafeKey::new(unbound);
    let nonce = Nonce::try_assume_unique_for_key(nonce_bytes)
        .map_err(|_| "invalid AES-GCM nonce".to_string())?;
    let mut buf = ciphertext_with_tag.to_vec();
    let plain_len = key
        .open_in_place(nonce, Aad::from(aad_bytes), &mut buf)
        .map_err(|_| "AES-GCM authentication failed".to_string())?
        .len();
    buf.truncate(plain_len);
    Ok(buf)
}

/// Builds the frozen `zniff.codec` JS helper object.
pub(crate) fn build_codec_object<'js>(cx: &Ctx<'js>) -> JsResult<Object<'js>> {
    let obj = Object::new(cx.clone())?;

    obj.set(
        "utf8Encode",
        Function::new(cx.clone(), |cx: Ctx<'js>, text: String| {
            if text.len() > MAX_CODEC_INPUT_BYTES {
                return Err(Exception::throw_range(
                    &cx,
                    "text input exceeds 2 MiB bridge limit",
                ));
            }
            to_uint8_array(&cx, text.into_bytes())
        })?,
    )?;
    obj.set(
        "utf8Decode",
        Function::new(
            cx.clone(),
            |cx: Ctx<'js>, val: Value<'js>| -> JsResult<String> {
                let bytes = extract_bytes_arg(&cx, val)?;
                Ok(String::from_utf8_lossy(&bytes).into_owned())
            },
        )?,
    )?;
    obj.set(
        "base64Encode",
        Function::new(
            cx.clone(),
            |cx: Ctx<'js>, val: Value<'js>| -> JsResult<String> {
                let bytes = extract_bytes_arg(&cx, val)?;
                Ok(base64_encode(&bytes))
            },
        )?,
    )?;
    obj.set(
        "base64Decode",
        Function::new(cx.clone(), |cx: Ctx<'js>, text: String| {
            let decoded = base64_decode(&text)
                .ok_or_else(|| Exception::throw_type(&cx, "invalid base64 input"))?;
            if decoded.len() > MAX_CODEC_INPUT_BYTES {
                return Err(Exception::throw_range(
                    &cx,
                    "decoded base64 exceeds 2 MiB bridge limit",
                ));
            }
            to_uint8_array(&cx, decoded)
        })?,
    )?;
    obj.set(
        "hexEncode",
        Function::new(
            cx.clone(),
            |cx: Ctx<'js>, val: Value<'js>| -> JsResult<String> {
                let bytes = extract_bytes_arg(&cx, val)?;
                Ok(hex_encode_bytes(&bytes))
            },
        )?,
    )?;
    obj.set(
        "hexDecode",
        Function::new(cx.clone(), |cx: Ctx<'js>, text: String| {
            let decoded = hex_decode_str(&text).map_err(|err| Exception::throw_type(&cx, &err))?;
            to_uint8_array(&cx, decoded)
        })?,
    )?;
    obj.set(
        "gzip",
        Function::new(cx.clone(), |cx: Ctx<'js>, val: Value<'js>| {
            let bytes = extract_bytes_arg(&cx, val)?;
            let out = gzip_compress(&bytes).map_err(|err| Exception::throw_message(&cx, &err))?;
            to_uint8_array(&cx, out)
        })?,
    )?;
    obj.set(
        "gunzip",
        Function::new(cx.clone(), |cx: Ctx<'js>, val: Value<'js>| {
            let bytes = extract_bytes_arg(&cx, val)?;
            let out = gzip_decompress(&bytes).map_err(|err| Exception::throw_message(&cx, &err))?;
            to_uint8_array(&cx, out)
        })?,
    )?;
    obj.set(
        "zstdDecompress",
        Function::new(cx.clone(), |cx: Ctx<'js>, val: Value<'js>| {
            let bytes = extract_bytes_arg(&cx, val)?;
            let out = zstd_decompress(&bytes).map_err(|err| Exception::throw_message(&cx, &err))?;
            to_uint8_array(&cx, out)
        })?,
    )?;

    Ok(obj)
}

/// Builds the frozen `zniff.crypto` JS helper object.
pub(crate) fn build_crypto_object<'js>(cx: &Ctx<'js>) -> JsResult<Object<'js>> {
    let obj = Object::new(cx.clone())?;

    obj.set(
        "sha256",
        Function::new(cx.clone(), |cx: Ctx<'js>, val: Value<'js>| {
            let bytes = extract_bytes_arg(&cx, val)?;
            to_uint8_array(&cx, sha256(&bytes).to_vec())
        })?,
    )?;
    obj.set(
        "hmacSha256",
        Function::new(
            cx.clone(),
            |cx: Ctx<'js>, key: Value<'js>, data: Value<'js>| {
                let k = extract_bytes_arg(&cx, key)?;
                let d = extract_bytes_arg(&cx, data)?;
                to_uint8_array(&cx, hmac_sha256(&k, &d).to_vec())
            },
        )?,
    )?;

    register_aes_gcm_fn(&obj, cx, "aes128GcmEncrypt", &AES_128_GCM, true)?;
    register_aes_gcm_fn(&obj, cx, "aes128GcmDecrypt", &AES_128_GCM, false)?;
    register_aes_gcm_fn(&obj, cx, "aes256GcmEncrypt", &AES_256_GCM, true)?;
    register_aes_gcm_fn(&obj, cx, "aes256GcmDecrypt", &AES_256_GCM, false)?;

    Ok(obj)
}

fn register_aes_gcm_fn<'js>(
    obj: &Object<'js>,
    cx: &Ctx<'js>,
    name: &str,
    algo: &'static ring::aead::Algorithm,
    encrypt: bool,
) -> JsResult<()> {
    let f = Function::new(
        cx.clone(),
        move |cx: Ctx<'js>,
              key: Value<'js>,
              nonce: Value<'js>,
              payload: Value<'js>,
              aad: Opt<Value<'js>>| {
            let k = extract_bytes_arg(&cx, key)?;
            let n = extract_bytes_arg(&cx, nonce)?;
            let p = extract_bytes_arg(&cx, payload)?;
            let a = match aad.0 {
                Some(v) if !v.is_undefined() && !v.is_null() => extract_bytes_arg(&cx, v)?,
                _ => Vec::new(),
            };
            let res = if encrypt {
                aes_gcm_encrypt(algo, &k, &n, &p, &a)
            } else {
                aes_gcm_decrypt(algo, &k, &n, &p, &a)
            };
            let out = res.map_err(|err| Exception::throw_message(&cx, &err))?;
            to_uint8_array(&cx, out)
        },
    )?;
    obj.set(name, f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_and_crypto_roundtrips_and_bomb_protection() {
        let raw = br#"{"host":"api.vicanso.dev","seq":42}"#;
        let hex = hex_encode_bytes(raw);
        assert_eq!(hex_decode_str(&hex).unwrap(), raw);
        assert!(hex_decode_str("abc").is_err());

        let gz = gzip_compress(raw).unwrap();
        assert_eq!(gzip_decompress(&gz).unwrap(), raw);

        // Bomb guard: > 2 MiB zeros compressed via gzip must be rejected safely.
        let oversized = vec![0_u8; MAX_DECOMPRESS_OUTPUT_BYTES + 4096];
        let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(&oversized).unwrap();
        let bomb_gz = enc.finish().unwrap();
        assert!(gzip_decompress(&bomb_gz).is_err());

        // AES-128-GCM and AES-256-GCM roundtrips.
        let key16 = [0x11_u8; 16];
        let key32 = [0x22_u8; 32];
        let nonce = [0x33_u8; 12];
        let ct128 = aes_gcm_encrypt(&AES_128_GCM, &key16, &nonce, raw, b"vicanso").unwrap();
        assert_eq!(
            aes_gcm_decrypt(&AES_128_GCM, &key16, &nonce, &ct128, b"vicanso").unwrap(),
            raw
        );
        assert!(aes_gcm_decrypt(&AES_128_GCM, &key16, &nonce, &ct128, b"tampered").is_err());

        let ct256 = aes_gcm_encrypt(&AES_256_GCM, &key32, &nonce, raw, b"").unwrap();
        assert_eq!(
            aes_gcm_decrypt(&AES_256_GCM, &key32, &nonce, &ct256, b"").unwrap(),
            raw
        );
        assert!(aes_gcm_encrypt(&AES_256_GCM, &key16, &nonce, raw, b"").is_err());
    }
}
