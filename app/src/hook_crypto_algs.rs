//! Pure-Rust RFC 1321 MD5, RFC 2104 HMAC-MD5, and FIPS 197 AES-128/AES-256 CBC
//! with strict PKCS#7 padding for `zniff.crypto.*`.

const MD5_SHIFTS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

const MD5_K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

#[rustfmt::skip]
const SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

#[rustfmt::skip]
const INV_SBOX: [u8; 256] = [
    0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38, 0xbf, 0x40, 0xa3, 0x9e, 0x81, 0xf3, 0xd7, 0xfb,
    0x7c, 0xe3, 0x39, 0x82, 0x9b, 0x2f, 0xff, 0x87, 0x34, 0x8e, 0x43, 0x44, 0xc4, 0xde, 0xe9, 0xcb,
    0x54, 0x7b, 0x94, 0x32, 0xa6, 0xc2, 0x23, 0x3d, 0xee, 0x4c, 0x95, 0x0b, 0x42, 0xfa, 0xc3, 0x4e,
    0x08, 0x2e, 0xa1, 0x66, 0x28, 0xd9, 0x24, 0xb2, 0x76, 0x5b, 0xa2, 0x49, 0x6d, 0x8b, 0xd1, 0x25,
    0x72, 0xf8, 0xf6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xd4, 0xa4, 0x5c, 0xcc, 0x5d, 0x65, 0xb6, 0x92,
    0x6c, 0x70, 0x48, 0x50, 0xfd, 0xed, 0xb9, 0xda, 0x5e, 0x15, 0x46, 0x57, 0xa7, 0x8d, 0x9d, 0x84,
    0x90, 0xd8, 0xab, 0x00, 0x8c, 0xbc, 0xd3, 0x0a, 0xf7, 0xe4, 0x58, 0x05, 0xb8, 0xb3, 0x45, 0x06,
    0xd0, 0x2c, 0x1e, 0x8f, 0xca, 0x3f, 0x0f, 0x02, 0xc1, 0xaf, 0xbd, 0x03, 0x01, 0x13, 0x8a, 0x6b,
    0x3a, 0x91, 0x11, 0x41, 0x4f, 0x67, 0xdc, 0xea, 0x97, 0xf2, 0xcf, 0xce, 0xf0, 0xb4, 0xe6, 0x73,
    0x96, 0xac, 0x74, 0x22, 0xe7, 0xad, 0x35, 0x85, 0xe2, 0xf9, 0x37, 0xe8, 0x1c, 0x75, 0xdf, 0x6e,
    0x47, 0xf1, 0x1a, 0x71, 0x1d, 0x29, 0xc5, 0x89, 0x6f, 0xb7, 0x62, 0x0e, 0xaa, 0x18, 0xbe, 0x1b,
    0xfc, 0x56, 0x3e, 0x4b, 0xc6, 0xd2, 0x79, 0x20, 0x9a, 0xdb, 0xc0, 0xfe, 0x78, 0xcd, 0x5a, 0xf4,
    0x1f, 0xdd, 0xa8, 0x33, 0x88, 0x07, 0xc7, 0x31, 0xb1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xec, 0x5f,
    0x60, 0x51, 0x7f, 0xa9, 0x19, 0xb5, 0x4a, 0x0d, 0x2d, 0xe5, 0x7a, 0x9f, 0x93, 0xc9, 0x9c, 0xef,
    0xa0, 0xe0, 0x3b, 0x4d, 0xae, 0x2a, 0xf5, 0xb0, 0xc8, 0xeb, 0xbb, 0x3c, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2b, 0x04, 0x7e, 0xba, 0x77, 0xd6, 0x26, 0xe1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0c, 0x7d,
];

const RCON: [u8; 11] = [
    0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36,
];

/// Pure-Rust RFC 1321 MD5 digest.
pub(crate) fn md5(data: &[u8]) -> [u8; 16] {
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity((data.len() + 9 + 63) & !63);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() + 8) % 64 != 0 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_le_bytes());

    let mut m = [0_u32; 16];
    for chunk in padded.as_chunks::<64>().0 {
        for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            m[i] = u32::from_le_bytes(*word);
        }
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | ((!b) & d), i),
                16..=31 => ((d & b) | ((!d) & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | (!d)), (7 * i) % 16),
            };
            let temp = d;
            d = c;
            c = b;
            let sum = a.wrapping_add(f).wrapping_add(MD5_K[i]).wrapping_add(m[g]);
            b = b.wrapping_add(sum.rotate_left(MD5_SHIFTS[i]));
            a = temp;
        }
        for (acc, val) in state.iter_mut().zip([a, b, c, d]) {
            *acc = acc.wrapping_add(val);
        }
    }

    let mut out = [0_u8; 16];
    for (slot, val) in out.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        slot.copy_from_slice(&val.to_le_bytes());
    }
    out
}

/// Pure-Rust RFC 2104 HMAC-MD5 with 64-byte block size.
pub(crate) fn hmac_md5(key: &[u8], data: &[u8]) -> [u8; 16] {
    let mut key_block = [0_u8; 64];
    if key.len() > 64 {
        let hashed = md5(key);
        key_block[..16].copy_from_slice(&hashed);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Vec::with_capacity(64 + data.len());
    for b in key_block {
        inner.push(b ^ 0x36);
    }
    inner.extend_from_slice(data);
    let inner_hash = md5(&inner);

    let mut outer = Vec::with_capacity(64 + 16);
    for b in key_block {
        outer.push(b ^ 0x5c);
    }
    outer.extend_from_slice(&inner_hash);
    md5(&outer)
}

const fn xtime(b: u8) -> u8 {
    (b << 1) ^ (((b >> 7) & 1).wrapping_mul(0x1b))
}

const fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut res = 0_u8;
    let mut i = 0;
    while i < 8 {
        if (b & 1) != 0 {
            res ^= a;
        }
        a = xtime(a);
        b >>= 1;
        i += 1;
    }
    res
}

fn expand_aes_key(key: &[u8]) -> ([[u8; 16]; 15], usize) {
    let nk = key.len() / 4;
    let nr = nk + 6;
    let total_words = 4 * (nr + 1);
    let mut w = [[0_u8; 4]; 60];
    for (i, word) in w.iter_mut().take(nk).enumerate() {
        word.copy_from_slice(&key[i * 4..i * 4 + 4]);
    }
    for i in nk..total_words {
        let mut temp = w[i - 1];
        if i % nk == 0 {
            temp = [
                SBOX[temp[1] as usize] ^ RCON[i / nk],
                SBOX[temp[2] as usize],
                SBOX[temp[3] as usize],
                SBOX[temp[0] as usize],
            ];
        } else if nk > 6 && i % nk == 4 {
            temp = [
                SBOX[temp[0] as usize],
                SBOX[temp[1] as usize],
                SBOX[temp[2] as usize],
                SBOX[temp[3] as usize],
            ];
        }
        for j in 0..4 {
            w[i][j] = w[i - nk][j] ^ temp[j];
        }
    }
    let mut rk = [[0_u8; 16]; 15];
    for (r, slot) in rk.iter_mut().take(nr + 1).enumerate() {
        for c in 0..4 {
            slot[c * 4..c * 4 + 4].copy_from_slice(&w[r * 4 + c]);
        }
    }
    (rk, nr)
}

fn add_round_key(state: &mut [u8; 16], rk: &[u8; 16]) {
    for (s, k) in state.iter_mut().zip(rk.iter()) {
        *s ^= *k;
    }
}

fn aes_encrypt_block(rk: &[[u8; 16]; 15], nr: usize, input: [u8; 16]) -> [u8; 16] {
    let mut s = input;
    add_round_key(&mut s, &rk[0]);
    for (round, round_key) in rk.iter().enumerate().take(nr + 1).skip(1) {
        for b in &mut s {
            *b = SBOX[*b as usize];
        }
        // ShiftRows (column-major 4x4 state).
        s = [
            s[0], s[5], s[10], s[15], s[4], s[9], s[14], s[3], s[8], s[13], s[2], s[7], s[12],
            s[1], s[6], s[11],
        ];
        if round < nr {
            for c in 0..4 {
                let idx = c * 4;
                let [a0, a1, a2, a3] = [s[idx], s[idx + 1], s[idx + 2], s[idx + 3]];
                s[idx] = xtime(a0) ^ gf_mul(a1, 3) ^ a2 ^ a3;
                s[idx + 1] = a0 ^ xtime(a1) ^ gf_mul(a2, 3) ^ a3;
                s[idx + 2] = a0 ^ a1 ^ xtime(a2) ^ gf_mul(a3, 3);
                s[idx + 3] = gf_mul(a0, 3) ^ a1 ^ a2 ^ xtime(a3);
            }
        }
        add_round_key(&mut s, round_key);
    }
    s
}

fn aes_decrypt_block(rk: &[[u8; 16]; 15], nr: usize, input: [u8; 16]) -> [u8; 16] {
    let mut s = input;
    add_round_key(&mut s, &rk[nr]);
    for round in (0..nr).rev() {
        // InvShiftRows (column-major 4x4 state).
        s = [
            s[0], s[13], s[10], s[7], s[4], s[1], s[14], s[11], s[8], s[5], s[2], s[15], s[12],
            s[9], s[6], s[3],
        ];
        for b in &mut s {
            *b = INV_SBOX[*b as usize];
        }
        add_round_key(&mut s, &rk[round]);
        if round > 0 {
            for c in 0..4 {
                let idx = c * 4;
                let [a0, a1, a2, a3] = [s[idx], s[idx + 1], s[idx + 2], s[idx + 3]];
                s[idx] = gf_mul(a0, 0x0e) ^ gf_mul(a1, 0x0b) ^ gf_mul(a2, 0x0d) ^ gf_mul(a3, 0x09);
                s[idx + 1] =
                    gf_mul(a0, 0x09) ^ gf_mul(a1, 0x0e) ^ gf_mul(a2, 0x0b) ^ gf_mul(a3, 0x0d);
                s[idx + 2] =
                    gf_mul(a0, 0x0d) ^ gf_mul(a1, 0x09) ^ gf_mul(a2, 0x0e) ^ gf_mul(a3, 0x0b);
                s[idx + 3] =
                    gf_mul(a0, 0x0b) ^ gf_mul(a1, 0x0d) ^ gf_mul(a2, 0x09) ^ gf_mul(a3, 0x0e);
            }
        }
    }
    s
}

fn aes_cbc_encrypt_blocks(
    rk: &[[u8; 16]; 15],
    nr: usize,
    iv: &[u8; 16],
    aligned: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(aligned.len());
    let mut prev = *iv;
    for chunk in aligned.as_chunks::<16>().0 {
        let mut block = *chunk;
        for (b, p) in block.iter_mut().zip(prev.iter()) {
            *b ^= *p;
        }
        let encrypted = aes_encrypt_block(rk, nr, block);
        out.extend_from_slice(&encrypted);
        prev = encrypted;
    }
    out
}

fn aes_cbc_decrypt_blocks(
    rk: &[[u8; 16]; 15],
    nr: usize,
    iv: &[u8; 16],
    aligned: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(aligned.len());
    let mut prev = *iv;
    for chunk in aligned.as_chunks::<16>().0 {
        let mut plain = aes_decrypt_block(rk, nr, *chunk);
        for (b, p) in plain.iter_mut().zip(prev.iter()) {
            *b ^= *p;
        }
        out.extend_from_slice(&plain);
        prev = *chunk;
    }
    out
}

fn validate_cbc_key_iv(
    expected_key_len: usize,
    label: &str,
    key: &[u8],
    iv: &[u8],
) -> Result<[u8; 16], String> {
    if key.len() != expected_key_len {
        return Err(format!(
            "{label} key must be {expected_key_len} bytes (got {})",
            key.len()
        ));
    }
    iv.try_into()
        .map_err(|_| format!("{label} IV must be 16 bytes (got {})", iv.len()))
}

fn pkcs7_unpad(buf: &mut Vec<u8>) -> Result<(), String> {
    let Some(&pad_byte) = buf.last() else {
        return Err("AES-CBC ciphertext is empty".to_string());
    };
    let pad = pad_byte as usize;
    if pad == 0 || pad > 16 || pad > buf.len() {
        return Err("invalid PKCS#7 padding length".to_string());
    }
    let start = buf.len() - pad;
    let mut diff = 0_u8;
    for &b in &buf[start..] {
        diff |= b ^ pad_byte;
    }
    if diff != 0 {
        return Err("invalid PKCS#7 padding bytes".to_string());
    }
    buf.truncate(start);
    Ok(())
}

fn aes_cbc_encrypt_inner(
    expected_key_len: usize,
    label: &str,
    key: &[u8],
    iv: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, String> {
    let iv_arr = validate_cbc_key_iv(expected_key_len, label, key, iv)?;
    let pad = 16 - (plaintext.len() % 16);
    let mut padded = Vec::with_capacity(plaintext.len() + pad);
    padded.extend_from_slice(plaintext);
    padded.resize(plaintext.len() + pad, pad as u8);
    let (rk, nr) = expand_aes_key(key);
    Ok(aes_cbc_encrypt_blocks(&rk, nr, &iv_arr, &padded))
}

fn aes_cbc_decrypt_inner(
    expected_key_len: usize,
    label: &str,
    key: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, String> {
    let iv_arr = validate_cbc_key_iv(expected_key_len, label, key, iv)?;
    if ciphertext.is_empty() || !ciphertext.len().is_multiple_of(16) {
        return Err(format!(
            "{label} ciphertext must be a non-empty multiple of 16 bytes"
        ));
    }
    let (rk, nr) = expand_aes_key(key);
    let mut plain = aes_cbc_decrypt_blocks(&rk, nr, &iv_arr, ciphertext);
    pkcs7_unpad(&mut plain)?;
    Ok(plain)
}

/// AES-128-CBC encryption with PKCS#7 padding.
pub(crate) fn aes128_cbc_encrypt(
    key: &[u8],
    iv: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, String> {
    aes_cbc_encrypt_inner(16, "AES-128-CBC", key, iv, plaintext)
}

/// AES-128-CBC decryption with strict PKCS#7 unpadding.
pub(crate) fn aes128_cbc_decrypt(
    key: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, String> {
    aes_cbc_decrypt_inner(16, "AES-128-CBC", key, iv, ciphertext)
}

/// AES-256-CBC encryption with PKCS#7 padding.
pub(crate) fn aes256_cbc_encrypt(
    key: &[u8],
    iv: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, String> {
    aes_cbc_encrypt_inner(32, "AES-256-CBC", key, iv, plaintext)
}

/// AES-256-CBC decryption with strict PKCS#7 unpadding.
pub(crate) fn aes256_cbc_decrypt(
    key: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, String> {
    aes_cbc_decrypt_inner(32, "AES-256-CBC", key, iv, ciphertext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook_codec_crypto::{hex_decode_str, hex_encode_bytes};

    #[test]
    fn md5_rfc1321_vectors() {
        let cases = [
            ("", "d41d8cd98f00b204e9800998ecf8427e"),
            ("a", "0cc175b9c0f1b6a831c399e269772661"),
            ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            (
                "abcdefghijklmnopqrstuvwxyz",
                "c3fcd3d76192e4007dfb496cca67e13b",
            ),
            (
                "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
                "d174ab98d277d9f5a5611c2c9f419d9f",
            ),
            (
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "57edf4a22be3c955ac49da2e2107b67a",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(hex_encode_bytes(&md5(input.as_bytes())), expected);
        }
    }

    #[test]
    fn hmac_md5_rfc2104_vectors() {
        let cases: [(&[u8], &[u8], &str); 7] = [
            (&[0x0b; 16], b"Hi There", "9294727a3638bb1c13f48ef8158bfc9d"),
            (
                b"Jefe",
                b"what do ya want for nothing?",
                "750c783e6ab0b503eaa86e310a5db738",
            ),
            (&[0xaa; 16], &[0xdd; 50], "56be34521d144c88dbb8c733f0e8b3f6"),
            (
                &(1..=25_u8).collect::<Vec<_>>(),
                &[0xcd; 50],
                "697eaf0aca3a3aea3a75164746ffaa79",
            ),
            (
                &[0x0c; 16],
                b"Test With Truncation",
                "56461ef2342edc00f9bab995690efd4c",
            ),
            (
                &[0xaa; 80],
                b"Test Using Larger Than Block-Size Key - Hash Key First",
                "6b1ab7fe4bd7bf8f0b62e6ce61b9d0cd",
            ),
            (
                &[0xaa; 80],
                b"Test Using Larger Than Block-Size Key and Larger Than One Block-Size Data",
                "6f630fad67cda0ee1fb1f562db3aa53e",
            ),
        ];
        for (key, data, expected) in cases {
            assert_eq!(hex_encode_bytes(&hmac_md5(key, data)), expected);
        }
    }

    #[test]
    fn aes_cbc_nist_sp800_38a_f21_f25_and_pkcs7() {
        for (i, &s) in SBOX.iter().enumerate() {
            assert_eq!(INV_SBOX[s as usize], i as u8);
        }
        let iv: [u8; 16] = hex_decode_str("000102030405060708090a0b0c0d0e0f")
            .unwrap()
            .try_into()
            .unwrap();
        let pt = hex_decode_str(
            "6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51\
             30c81c46a35ce411e5fbc1191a0a52eff69f2445df4f9b17ad2b417be66c3710",
        )
        .unwrap();

        // NIST SP 800-38A F.2.1 AES-128-CBC.
        let k128 = hex_decode_str("2b7e151628aed2a6abf7158809cf4f3c").unwrap();
        let ct128_raw_expected = hex_decode_str(
            "7649abac8119b246cee98e9b12e9197d5086cb9b507219ee95db113a917678b2\
             73bed6b8e3c1743b7116e69e222295163ff1caa1681fac09120eca307586e1a7",
        )
        .unwrap();
        let (rk128, nr128) = expand_aes_key(&k128);
        let raw128 = aes_cbc_encrypt_blocks(&rk128, nr128, &iv, &pt);
        assert_eq!(raw128, ct128_raw_expected);
        assert_eq!(aes_cbc_decrypt_blocks(&rk128, nr128, &iv, &raw128), pt);

        let ct128_pkcs7 = aes128_cbc_encrypt(&k128, &iv, &pt).unwrap();
        assert_eq!(
            hex_encode_bytes(&ct128_pkcs7),
            "7649abac8119b246cee98e9b12e9197d5086cb9b507219ee95db113a917678b2\
             73bed6b8e3c1743b7116e69e222295163ff1caa1681fac09120eca307586e1a7\
             8cb82807230e1321d3fae00d18cc2012"
        );
        assert_eq!(aes128_cbc_decrypt(&k128, &iv, &ct128_pkcs7).unwrap(), pt);

        // NIST SP 800-38A F.2.5 AES-256-CBC.
        let k256 =
            hex_decode_str("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4")
                .unwrap();
        let ct256_raw_expected = hex_decode_str(
            "f58c4c04d6e5f1ba779eabfb5f7bfbd69cfc4e967edb808d679f777bc6702c7d\
             39f23369a9d9bacfa530e26304231461b2eb05e2c39be9fcda6c19078c6a9d1b",
        )
        .unwrap();
        let (rk256, nr256) = expand_aes_key(&k256);
        let raw256 = aes_cbc_encrypt_blocks(&rk256, nr256, &iv, &pt);
        assert_eq!(raw256, ct256_raw_expected);
        assert_eq!(aes_cbc_decrypt_blocks(&rk256, nr256, &iv, &raw256), pt);

        let ct256_pkcs7 = aes256_cbc_encrypt(&k256, &iv, &pt).unwrap();
        assert_eq!(
            hex_encode_bytes(&ct256_pkcs7),
            "f58c4c04d6e5f1ba779eabfb5f7bfbd69cfc4e967edb808d679f777bc6702c7d\
             39f23369a9d9bacfa530e26304231461b2eb05e2c39be9fcda6c19078c6a9d1b\
             3f461796d6b0d6b2e0c2a72b4d80e644"
        );
        assert_eq!(aes256_cbc_decrypt(&k256, &iv, &ct256_pkcs7).unwrap(), pt);
    }

    #[test]
    fn aes_cbc_pkcs7_rejects_tampered_padding_and_bad_sizes() {
        let key16 = b"0123456789abcdef";
        let iv16 = b"fedcba9876543210";
        let empty_ct = aes128_cbc_encrypt(key16, iv16, b"").unwrap();
        assert_eq!(empty_ct.len(), 16);
        assert_eq!(aes128_cbc_decrypt(key16, iv16, &empty_ct).unwrap(), b"");

        let msg = b"host=api.vicanso.dev&sig=12345";
        let ct = aes128_cbc_encrypt(key16, iv16, msg).unwrap();
        assert_eq!(aes128_cbc_decrypt(key16, iv16, &ct).unwrap(), msg);

        // Flipping final padding byte or forcing pad=0 / pad>16 must be rejected.
        let mut tampered = ct.clone();
        *tampered.last_mut().unwrap() ^= 0x01;
        assert!(aes128_cbc_decrypt(key16, iv16, &tampered).is_err());

        let (rk, nr) = expand_aes_key(key16);
        for bad_pad in [
            [0x00_u8; 16],
            [0x11_u8; 16],
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 0x01, 0x02],
        ] {
            let forged = aes_cbc_encrypt_blocks(&rk, nr, iv16, &bad_pad);
            assert!(aes128_cbc_decrypt(key16, iv16, &forged).is_err());
        }
        assert!(aes128_cbc_decrypt(key16, iv16, b"").is_err());
        assert!(aes128_cbc_decrypt(key16, iv16, &ct[..15]).is_err());
        assert!(aes128_cbc_encrypt(b"short", iv16, msg).is_err());
        assert!(aes256_cbc_encrypt(key16, iv16, msg).is_err());
        assert!(aes128_cbc_encrypt(key16, b"short", msg).is_err());
    }
}
