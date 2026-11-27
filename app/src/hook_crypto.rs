//! QuickJS `zniff.crypto.*` host bindings backed by `ring`, `crate::rule_vars`,
//! and pure-Rust MD5 / AES-CBC primitives in `crate::hook_crypto_algs`.

use std::num::NonZeroU32;

use ring::aead::{
    AES_128_GCM, AES_256_GCM, Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey,
};
use ring::digest::{self, SHA1_FOR_LEGACY_USE_ONLY, SHA384, SHA512};
use ring::hkdf::{HKDF_SHA256, KeyType, Salt};
use ring::hmac::{self, HMAC_SHA1_FOR_LEGACY_USE_ONLY, HMAC_SHA384, HMAC_SHA512};
use ring::pbkdf2::{PBKDF2_HMAC_SHA256, derive as pbkdf2_derive};
use ring::rand::{SecureRandom, SystemRandom};
use rquickjs::function::Opt;
use rquickjs::{Ctx, Exception, Function, Object, Result as JsResult, Value};

use crate::hook_codec_crypto::{
    extract_body_input, extract_bytes_arg, hex_encode_bytes, to_uint8_array,
};
use crate::hook_crypto_algs::{
    aes128_cbc_decrypt, aes128_cbc_encrypt, aes256_cbc_decrypt, aes256_cbc_encrypt, hmac_md5, md5,
};
use crate::rule_vars::{hmac_sha256, sha256};

const MAX_PBKDF2_ITERATIONS: usize = 600_000;
const MAX_KDF_KEY_LEN: usize = 1024;
const MAX_RANDOM_BYTES: usize = 65_536;

#[derive(Clone, Copy)]
struct HkdfLen(usize);

impl KeyType for HkdfLen {
    fn len(&self) -> usize {
        self.0
    }
}

fn extract_optional_body_input<'js>(cx: &Ctx<'js>, opt: Opt<Value<'js>>) -> JsResult<Vec<u8>> {
    match opt.0 {
        Some(v) if !v.is_undefined() && !v.is_null() => extract_body_input(cx, v),
        _ => Ok(Vec::new()),
    }
}

fn extract_bounded_int<'js>(
    cx: &Ctx<'js>,
    val: Value<'js>,
    name: &str,
    min: usize,
    max: usize,
) -> JsResult<usize> {
    let Some(n) = val.as_number() else {
        return Err(Exception::throw_type(
            cx,
            &format!("{name} must be a number"),
        ));
    };
    if !n.is_finite() || n.fract() != 0.0 || n < (min as f64) || n > (max as f64) {
        return Err(Exception::throw_range(
            cx,
            &format!("{name} must be an integer in {min}..={max}"),
        ));
    }
    Ok(n as usize)
}

fn ring_hmac(algo: hmac::Algorithm, key: &[u8], data: &[u8]) -> hmac::Tag {
    let k = hmac::Key::new(algo, key);
    hmac::sign(&k, data)
}

fn prepare_aead(
    algo: &'static ring::aead::Algorithm,
    label: &str,
    key_bytes: &[u8],
    nonce_bytes: &[u8],
) -> Result<(LessSafeKey, Nonce), String> {
    if key_bytes.len() != algo.key_len() {
        return Err(format!(
            "{label} key must be {} bytes (got {})",
            algo.key_len(),
            key_bytes.len()
        ));
    }
    if nonce_bytes.len() != algo.nonce_len() {
        return Err(format!(
            "{label} nonce must be {} bytes (got {})",
            algo.nonce_len(),
            nonce_bytes.len()
        ));
    }
    let unbound = UnboundKey::new(algo, key_bytes).map_err(|_| format!("invalid {label} key"))?;
    let nonce = Nonce::try_assume_unique_for_key(nonce_bytes)
        .map_err(|_| format!("invalid {label} nonce"))?;
    Ok((LessSafeKey::new(unbound), nonce))
}

pub(crate) fn aead_encrypt(
    algo: &'static ring::aead::Algorithm,
    label: &str,
    key_bytes: &[u8],
    nonce_bytes: &[u8],
    plaintext: &[u8],
    aad_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    let (key, nonce) = prepare_aead(algo, label, key_bytes, nonce_bytes)?;
    let mut buf = plaintext.to_vec();
    key.seal_in_place_append_tag(nonce, Aad::from(aad_bytes), &mut buf)
        .map_err(|_| format!("{label} encrypt failed"))?;
    Ok(buf)
}

fn aead_decrypt(
    algo: &'static ring::aead::Algorithm,
    label: &str,
    key_bytes: &[u8],
    nonce_bytes: &[u8],
    ciphertext_with_tag: &[u8],
    aad_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    let (key, nonce) = prepare_aead(algo, label, key_bytes, nonce_bytes)?;
    let mut buf = ciphertext_with_tag.to_vec();
    let plain_len = key
        .open_in_place(nonce, Aad::from(aad_bytes), &mut buf)
        .map_err(|_| format!("{label} authentication failed"))?
        .len();
    buf.truncate(plain_len);
    Ok(buf)
}

fn register_digest_pair<'js, F>(
    obj: &Object<'js>,
    cx: &Ctx<'js>,
    raw_name: &str,
    hex_name: &str,
    digest_fn: F,
) -> JsResult<()>
where
    F: Fn(&[u8]) -> Vec<u8> + Copy + 'static,
{
    obj.set(
        raw_name,
        Function::new(cx.clone(), move |cx: Ctx<'js>, val: Value<'js>| {
            let bytes = extract_body_input(&cx, val)?;
            to_uint8_array(&cx, digest_fn(&bytes))
        })?,
    )?;
    obj.set(
        hex_name,
        Function::new(
            cx.clone(),
            move |cx: Ctx<'js>, val: Value<'js>| -> JsResult<String> {
                let bytes = extract_body_input(&cx, val)?;
                Ok(hex_encode_bytes(&digest_fn(&bytes)))
            },
        )?,
    )
}

fn register_hmac_pair<'js, F>(
    obj: &Object<'js>,
    cx: &Ctx<'js>,
    raw_name: &str,
    hex_name: &str,
    hmac_fn: F,
) -> JsResult<()>
where
    F: Fn(&[u8], &[u8]) -> Vec<u8> + Copy + 'static,
{
    obj.set(
        raw_name,
        Function::new(
            cx.clone(),
            move |cx: Ctx<'js>, key: Value<'js>, data: Value<'js>| {
                let k = extract_body_input(&cx, key)?;
                let d = extract_body_input(&cx, data)?;
                to_uint8_array(&cx, hmac_fn(&k, &d))
            },
        )?,
    )?;
    obj.set(
        hex_name,
        Function::new(
            cx.clone(),
            move |cx: Ctx<'js>, key: Value<'js>, data: Value<'js>| -> JsResult<String> {
                let k = extract_body_input(&cx, key)?;
                let d = extract_body_input(&cx, data)?;
                Ok(hex_encode_bytes(&hmac_fn(&k, &d)))
            },
        )?,
    )
}

fn register_aead_pair<'js>(
    obj: &Object<'js>,
    cx: &Ctx<'js>,
    enc_name: &str,
    dec_name: &str,
    algo: &'static ring::aead::Algorithm,
    label: &'static str,
) -> JsResult<()> {
    obj.set(
        enc_name,
        Function::new(
            cx.clone(),
            move |cx: Ctx<'js>,
                  key: Value<'js>,
                  nonce: Value<'js>,
                  plaintext: Value<'js>,
                  aad: Opt<Value<'js>>| {
                let k = extract_body_input(&cx, key)?;
                let n = extract_body_input(&cx, nonce)?;
                let p = extract_body_input(&cx, plaintext)?;
                let a = extract_optional_body_input(&cx, aad)?;
                let out = aead_encrypt(algo, label, &k, &n, &p, &a)
                    .map_err(|err| Exception::throw_message(&cx, &err))?;
                to_uint8_array(&cx, out)
            },
        )?,
    )?;
    obj.set(
        dec_name,
        Function::new(
            cx.clone(),
            move |cx: Ctx<'js>,
                  key: Value<'js>,
                  nonce: Value<'js>,
                  ciphertext: Value<'js>,
                  aad: Opt<Value<'js>>| {
                let k = extract_body_input(&cx, key)?;
                let n = extract_body_input(&cx, nonce)?;
                let c = extract_bytes_arg(&cx, ciphertext)?;
                let a = extract_optional_body_input(&cx, aad)?;
                let out = aead_decrypt(algo, label, &k, &n, &c, &a)
                    .map_err(|err| Exception::throw_message(&cx, &err))?;
                to_uint8_array(&cx, out)
            },
        )?,
    )
}

type CbcFn = fn(&[u8], &[u8], &[u8]) -> Result<Vec<u8>, String>;

fn register_cbc_pair<'js>(
    obj: &Object<'js>,
    cx: &Ctx<'js>,
    enc_name: &str,
    dec_name: &str,
    enc_fn: CbcFn,
    dec_fn: CbcFn,
) -> JsResult<()> {
    obj.set(
        enc_name,
        Function::new(
            cx.clone(),
            move |cx: Ctx<'js>, key: Value<'js>, iv: Value<'js>, plaintext: Value<'js>| {
                let k = extract_body_input(&cx, key)?;
                let iv_bytes = extract_body_input(&cx, iv)?;
                let p = extract_body_input(&cx, plaintext)?;
                let out =
                    enc_fn(&k, &iv_bytes, &p).map_err(|err| Exception::throw_message(&cx, &err))?;
                to_uint8_array(&cx, out)
            },
        )?,
    )?;
    obj.set(
        dec_name,
        Function::new(
            cx.clone(),
            move |cx: Ctx<'js>, key: Value<'js>, iv: Value<'js>, ciphertext: Value<'js>| {
                let k = extract_body_input(&cx, key)?;
                let iv_bytes = extract_body_input(&cx, iv)?;
                let c = extract_bytes_arg(&cx, ciphertext)?;
                let out =
                    dec_fn(&k, &iv_bytes, &c).map_err(|err| Exception::throw_message(&cx, &err))?;
                to_uint8_array(&cx, out)
            },
        )?,
    )
}

fn register_kdf_fns<'js>(obj: &Object<'js>, cx: &Ctx<'js>) -> JsResult<()> {
    obj.set(
        "pbkdf2Sha256",
        Function::new(
            cx.clone(),
            |cx: Ctx<'js>,
             password: Value<'js>,
             salt: Value<'js>,
             iterations: Value<'js>,
             key_len: Value<'js>| {
                let pw = extract_body_input(&cx, password)?;
                let salt_bytes = extract_body_input(&cx, salt)?;
                let iters =
                    extract_bounded_int(&cx, iterations, "iterations", 1, MAX_PBKDF2_ITERATIONS)?;
                let len = extract_bounded_int(&cx, key_len, "keyLen", 1, MAX_KDF_KEY_LEN)?;
                let mut out = vec![0_u8; len];
                let nz_iters = NonZeroU32::new(iters as u32)
                    .ok_or_else(|| Exception::throw_range(&cx, "iterations must be >= 1"))?;
                pbkdf2_derive(PBKDF2_HMAC_SHA256, nz_iters, &salt_bytes, &pw, &mut out);
                to_uint8_array(&cx, out)
            },
        )?,
    )?;
    obj.set(
        "hkdfSha256",
        Function::new(
            cx.clone(),
            |cx: Ctx<'js>,
             secret: Value<'js>,
             salt: Value<'js>,
             info: Value<'js>,
             key_len: Value<'js>| {
                let ikm = extract_body_input(&cx, secret)?;
                let salt_bytes = extract_body_input(&cx, salt)?;
                let info_bytes = extract_body_input(&cx, info)?;
                let len = extract_bounded_int(&cx, key_len, "keyLen", 1, MAX_KDF_KEY_LEN)?;
                let prk = Salt::new(HKDF_SHA256, &salt_bytes).extract(&ikm);
                let info_ref = [info_bytes.as_slice()];
                let okm = prk
                    .expand(&info_ref, HkdfLen(len))
                    .map_err(|_| Exception::throw_range(&cx, "hkdfSha256 keyLen exceeds limit"))?;
                let mut out = vec![0_u8; len];
                okm.fill(&mut out)
                    .map_err(|_| Exception::throw_message(&cx, "hkdfSha256 expand failed"))?;
                to_uint8_array(&cx, out)
            },
        )?,
    )
}

fn register_compare_and_rand_fns<'js>(obj: &Object<'js>, cx: &Ctx<'js>) -> JsResult<()> {
    obj.set(
        "timingSafeEqual",
        Function::new(
            cx.clone(),
            |cx: Ctx<'js>, a: Value<'js>, b: Value<'js>| -> JsResult<bool> {
                let lhs = extract_body_input(&cx, a)?;
                let rhs = extract_body_input(&cx, b)?;
                #[allow(deprecated)]
                Ok(ring::constant_time::verify_slices_are_equal(&lhs, &rhs).is_ok())
            },
        )?,
    )?;
    obj.set(
        "randomBytes",
        Function::new(cx.clone(), |cx: Ctx<'js>, len: Value<'js>| {
            let n = extract_bounded_int(&cx, len, "len", 0, MAX_RANDOM_BYTES)?;
            let mut out = vec![0_u8; n];
            if n > 0 {
                SystemRandom::new()
                    .fill(&mut out)
                    .map_err(|_| Exception::throw_message(&cx, "CSPRNG randomBytes failed"))?;
            }
            to_uint8_array(&cx, out)
        })?,
    )?;
    obj.set(
        "uuidV4",
        Function::new(cx.clone(), |cx: Ctx<'js>| -> JsResult<String> {
            let mut bytes = [0_u8; 16];
            SystemRandom::new()
                .fill(&mut bytes)
                .map_err(|_| Exception::throw_message(&cx, "CSPRNG uuidV4 failed"))?;
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            Ok(format!(
                "{}-{}-{}-{}-{}",
                hex_encode_bytes(&bytes[0..4]),
                hex_encode_bytes(&bytes[4..6]),
                hex_encode_bytes(&bytes[6..8]),
                hex_encode_bytes(&bytes[8..10]),
                hex_encode_bytes(&bytes[10..16]),
            ))
        })?,
    )
}

/// Builds the frozen `zniff.crypto` JS helper object.
pub(crate) fn build_crypto_object<'js>(cx: &Ctx<'js>) -> JsResult<Object<'js>> {
    let obj = Object::new(cx.clone())?;

    register_digest_pair(&obj, cx, "md5", "md5Hex", |b| md5(b).to_vec())?;
    register_digest_pair(&obj, cx, "sha256", "sha256Hex", |b| sha256(b).to_vec())?;
    for (raw, hex, alg) in [
        ("sha1", "sha1Hex", &SHA1_FOR_LEGACY_USE_ONLY),
        ("sha384", "sha384Hex", &SHA384),
        ("sha512", "sha512Hex", &SHA512),
    ] {
        register_digest_pair(&obj, cx, raw, hex, move |b| {
            digest::digest(alg, b).as_ref().to_vec()
        })?;
    }

    register_hmac_pair(&obj, cx, "hmacMd5", "hmacMd5Hex", |k, d| {
        hmac_md5(k, d).to_vec()
    })?;
    register_hmac_pair(&obj, cx, "hmacSha256", "hmacSha256Hex", |k, d| {
        hmac_sha256(k, d).to_vec()
    })?;
    for (raw, hex, alg) in [
        ("hmacSha1", "hmacSha1Hex", HMAC_SHA1_FOR_LEGACY_USE_ONLY),
        ("hmacSha384", "hmacSha384Hex", HMAC_SHA384),
        ("hmacSha512", "hmacSha512Hex", HMAC_SHA512),
    ] {
        register_hmac_pair(&obj, cx, raw, hex, move |k, d| {
            ring_hmac(alg, k, d).as_ref().to_vec()
        })?;
    }

    for (enc, dec, algo, label) in [
        (
            "aes128GcmEncrypt",
            "aes128GcmDecrypt",
            &AES_128_GCM,
            "AES-128-GCM",
        ),
        (
            "aes256GcmEncrypt",
            "aes256GcmDecrypt",
            &AES_256_GCM,
            "AES-256-GCM",
        ),
        (
            "chacha20Poly1305Encrypt",
            "chacha20Poly1305Decrypt",
            &CHACHA20_POLY1305,
            "ChaCha20-Poly1305",
        ),
    ] {
        register_aead_pair(&obj, cx, enc, dec, algo, label)?;
    }

    for (enc, dec, enc_fn, dec_fn) in [
        (
            "aes128CbcEncrypt",
            "aes128CbcDecrypt",
            aes128_cbc_encrypt as CbcFn,
            aes128_cbc_decrypt as CbcFn,
        ),
        (
            "aes256CbcEncrypt",
            "aes256CbcDecrypt",
            aes256_cbc_encrypt as CbcFn,
            aes256_cbc_decrypt as CbcFn,
        ),
    ] {
        register_cbc_pair(&obj, cx, enc, dec, enc_fn, dec_fn)?;
    }

    register_kdf_fns(&obj, cx)?;
    register_compare_and_rand_fns(&obj, cx)?;
    Ok(obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rquickjs::{Context, Runtime};

    fn with_crypto_js<R>(f: impl FnOnce(Ctx<'_>) -> R) -> R {
        let rt = Runtime::new().expect("QuickJS runtime");
        let ctx = Context::full(&rt).expect("QuickJS context");
        ctx.with(|cx| {
            let crypto = build_crypto_object(&cx).expect("build zniff.crypto");
            cx.globals().set("crypto", crypto).expect("set crypto");
            f(cx)
        })
    }

    #[test]
    fn quickjs_digests_hmacs_and_body_input_parity() {
        with_crypto_js(|cx| {
            let ok: bool = cx
                .eval(
                    r#"
                const msg = "api.vicanso.dev";
                const enc = new Uint8Array([97,112,105,46,118,105,99,97,110,115,111,46,100,101,118]);
                const ab = enc.buffer;
                crypto.md5Hex("") === "d41d8cd98f00b204e9800998ecf8427e"
                  && crypto.md5Hex("abc") === "900150983cd24fb0d6963f7d28e17f72"
                  && crypto.md5(msg).length === 16
                  && crypto.sha1Hex("abc") === "a9993e364706816aba3e25717850c26c9cd0d89d"
                  && crypto.sha1(ab).length === 20
                  && crypto.sha256Hex(msg) === crypto.sha256Hex(enc)
                  && crypto.sha256(msg).length === 32
                  && crypto.sha384Hex("abc") === "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
                  && crypto.sha512Hex("abc") === "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
                  && crypto.hmacMd5Hex("Jefe", "what do ya want for nothing?") === "750c783e6ab0b503eaa86e310a5db738"
                  && crypto.hmacSha1Hex("Jefe", "what do ya want for nothing?") === "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79"
                  && crypto.hmacSha256Hex("Jefe", "what do ya want for nothing?") === "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
                  && crypto.hmacSha384Hex("Jefe", "what do ya want for nothing?") === "af45d2e376484031617f78d2b58a6b1b9c7ef464f5a01b47e42ec3736322445e8e2240ca5e69e2c78b3239ecfab21649"
                  && crypto.hmacSha512Hex("Jefe", "what do ya want for nothing?") === "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737"
                  && crypto.hmacMd5("k", msg).length === 16
                  && crypto.hmacSha1("k", msg).length === 20
                  && crypto.hmacSha1Hex("k", msg) === crypto.hmacSha1Hex(new Uint8Array([107]), enc)
                  && crypto.hmacSha256("k", msg).length === 32
                  && crypto.hmacSha384("k", msg).length === 48
                  && crypto.hmacSha512("k", msg).length === 64
            "#,
                )
                .expect("eval digests and hmacs");
            assert!(ok);
        });
    }

    #[test]
    fn quickjs_aead_cbc_kdf_compare_and_csprng() {
        with_crypto_js(|cx| {
            let ok: bool = cx
                .eval(
                    r#"
                const pt = "host=api.vicanso.dev&ts=1";
                const nonce12 = "123456789012";
                const gcm128 = crypto.aes128GcmEncrypt("0123456789abcdef", nonce12, pt, "aad");
                const gcm256 = crypto.aes256GcmEncrypt("0123456789abcdef0123456789abcdef", nonce12, pt);
                const chacha = crypto.chacha20Poly1305Encrypt("0123456789abcdef0123456789abcdef", nonce12, pt, "vicanso");
                const decCh = crypto.chacha20Poly1305Decrypt("0123456789abcdef0123456789abcdef", nonce12, chacha.buffer, "vicanso");

                const cbc128 = crypto.aes128CbcEncrypt("0123456789abcdef", "fedcba9876543210", pt);
                const decCbc128 = crypto.aes128CbcDecrypt("0123456789abcdef", "fedcba9876543210", cbc128);
                const cbc256 = crypto.aes256CbcEncrypt("0123456789abcdef0123456789abcdef", "fedcba9876543210", pt);
                const decCbc256 = crypto.aes256CbcDecrypt("0123456789abcdef0123456789abcdef", "fedcba9876543210", cbc256);

                const pb = crypto.pbkdf2Sha256("password", "salt", 2, 32);
                const toHex = u => Array.from(u, b => b.toString(16).padStart(2, "0")).join("");
                const ikm = new Uint8Array(22).fill(0x0b);
                const hk1 = crypto.hkdfSha256(ikm, Uint8Array.from({length:13}, (_,i)=>i), Uint8Array.from({length:10}, (_,i)=>0xf0+i), 42);
                const hk3 = crypto.hkdfSha256(ikm, "", "", 42);
                const rnd0 = crypto.randomBytes(0);
                const rnd32 = crypto.randomBytes(32);
                const u1 = crypto.uuidV4();
                const u2 = crypto.uuidV4();
                const uuidRe = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

                crypto.aes128GcmDecrypt("0123456789abcdef", nonce12, gcm128, "aad").length === pt.length
                  && crypto.aes256GcmDecrypt("0123456789abcdef0123456789abcdef", nonce12, gcm256).length === pt.length
                  && crypto.timingSafeEqual(decCh, pt)
                  && crypto.timingSafeEqual(decCbc128, pt)
                  && crypto.timingSafeEqual(decCbc256, pt)
                  && !crypto.timingSafeEqual("abc", "abd")
                  && !crypto.timingSafeEqual("abc", "ab")
                  && toHex(pb) === "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"
                  && toHex(hk1) === "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
                  && toHex(hk3) === "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d9d201395faa4b61a96c8"
                  && rnd0.length === 0
                  && rnd32.length === 32
                  && uuidRe.test(u1) && uuidRe.test(u2) && u1 !== u2
            "#,
                )
                .expect("eval aead cbc kdf csprng");
            assert!(ok);
        });
    }

    #[test]
    fn quickjs_rejects_invalid_bounds_and_ciphertext_strings() {
        with_crypto_js(|cx| {
            let checks: Vec<bool> = cx
                .eval(
                    r#"
                const mustThrow = (fn, name) => {
                  try { fn(); return false; } catch (e) { return !name || e.name === name; }
                };
                [
                  mustThrow(() => crypto.pbkdf2Sha256("p", "s", 0, 32), "RangeError"),
                  mustThrow(() => crypto.pbkdf2Sha256("p", "s", 600001, 32), "RangeError"),
                  mustThrow(() => crypto.pbkdf2Sha256("p", "s", 1, 0), "RangeError"),
                  mustThrow(() => crypto.pbkdf2Sha256("p", "s", 1, 1025), "RangeError"),
                  mustThrow(() => crypto.pbkdf2Sha256("p", "s", 1.5, 16), "RangeError"),
                  mustThrow(() => crypto.pbkdf2Sha256("p", "s", NaN, 16), "RangeError"),
                  mustThrow(() => crypto.pbkdf2Sha256("p", "s", Infinity, 16), "RangeError"),
                  mustThrow(() => crypto.hkdfSha256("s", "", "i", 0), "RangeError"),
                  mustThrow(() => crypto.hkdfSha256("s", "", "i", 1025), "RangeError"),
                  mustThrow(() => crypto.randomBytes(-1), "RangeError"),
                  mustThrow(() => crypto.randomBytes(65537), "RangeError"),
                  crypto.randomBytes(65536).length === 65536,
                  crypto.hkdfSha256("s", "", "i", 1024).length === 1024,
                  crypto.pbkdf2Sha256("p", "s", 1, 1024).length === 1024,
                  mustThrow(() => crypto.aes128CbcDecrypt("0123456789abcdef", "fedcba9876543210", "not-bytes"), "TypeError"),
                  mustThrow(() => crypto.chacha20Poly1305Decrypt("0123456789abcdef0123456789abcdef", "123456789012", "not-bytes"), "TypeError"),
                  mustThrow(() => crypto.chacha20Poly1305Decrypt("0123456789abcdef0123456789abcdef", "123456789012", new Uint8Array(24)), "Error"),
                ]
            "#,
                )
                .expect("eval error bounds");
            assert!(checks.into_iter().all(|v| v));
        });
    }
}
