//! Streaming Content-Encoding decode for the side copy.
//!
//! The forwarded bytes stay compressed. Only the recorded copy is decoded.
//! The cap is on decoded bytes.

use std::io::Write;

use flate2::write::{DeflateDecoder, GzDecoder, ZlibDecoder};

use crate::session::BODY_CAP;

#[derive(Debug)]
pub struct Decoder {
    layers: Vec<Layer>,
    emitted: usize,
    truncated: bool,
    error: Option<String>,
    finished: bool,
}

#[derive(Debug)]
struct CappedBuf {
    buf: Vec<u8>,
    limit: usize,
    hit_limit: bool,
}

impl CappedBuf {
    fn new(limit: usize) -> Self {
        Self {
            buf: Vec::new(),
            limit,
            hit_limit: false,
        }
    }

    fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
    }

    fn take(&mut self) -> (Vec<u8>, bool) {
        let hit = self.hit_limit;
        self.hit_limit = false;
        (std::mem::take(&mut self.buf), hit)
    }
}

impl Write for CappedBuf {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let room = self.limit.saturating_sub(self.buf.len());
        if room > 0 {
            let n = data.len().min(room);
            self.buf.extend_from_slice(&data[..n]);
        }
        if data.len() > room {
            self.hit_limit = true;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

enum DeflateInner {
    Sniff(Vec<u8>),
    Zlib(ZlibDecoder<CappedBuf>),
    Raw(DeflateDecoder<CappedBuf>),
}

enum Layer {
    Identity,
    Gzip(GzDecoder<CappedBuf>),
    Deflate(DeflateInner),
    Brotli(Box<brotli_decompressor::DecompressorWriter<CappedBuf>>),
    Zstd(zstd::stream::write::Decoder<'static, CappedBuf>),
}

impl std::fmt::Debug for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Identity => "identity",
            Self::Gzip(_) => "gzip",
            Self::Deflate(_) => "deflate",
            Self::Brotli(_) => "br",
            Self::Zstd(_) => "zstd",
        })
    }
}

fn is_zlib_header(b0: u8, b1: u8) -> bool {
    (b0 & 0x0f) == 8 && (b0 >> 4) <= 7 && u16::from_be_bytes([b0, b1]).is_multiple_of(31)
}

impl Decoder {
    pub fn from_header(header: Option<&str>) -> Self {
        let codings = header
            .unwrap_or("")
            .split(',')
            .map(|part| part.trim())
            .filter(|part| !part.is_empty() && !part.eq_ignore_ascii_case("identity"))
            .collect::<Vec<_>>();
        // Wire bytes are the last coding's output, so decode in reverse.
        let mut layers = Vec::new();
        for coding in codings.into_iter().rev() {
            match coding.to_ascii_lowercase().as_str() {
                "gzip" | "x-gzip" => {
                    layers.push(Layer::Gzip(GzDecoder::new(CappedBuf::new(BODY_CAP))))
                }
                "deflate" => {
                    layers.push(Layer::Deflate(DeflateInner::Sniff(Vec::with_capacity(2))))
                }
                "br" => layers.push(Layer::Brotli(Box::new(
                    brotli_decompressor::DecompressorWriter::new(CappedBuf::new(BODY_CAP), 4096),
                ))),
                "zstd" => match zstd::stream::write::Decoder::new(CappedBuf::new(BODY_CAP)) {
                    Ok(decoder) => layers.push(Layer::Zstd(decoder)),
                    Err(err) => {
                        return Self::failed(format!("zstd 解码器不可用: {err}"));
                    }
                },
                other => return Self::failed(format!("不支持的 Content-Encoding: {other}")),
            }
        }
        if layers.is_empty() {
            layers.push(Layer::Identity);
        }
        Self {
            layers,
            emitted: 0,
            truncated: false,
            error: None,
            finished: false,
        }
    }

    fn failed(error: String) -> Self {
        Self {
            layers: Vec::new(),
            emitted: 0,
            truncated: false,
            error: Some(error),
            finished: true,
        }
    }

    #[cfg(test)]
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Feeds wire bytes and returns newly decoded bytes within the cap.
    pub fn push(&mut self, input: &[u8]) -> Vec<u8> {
        if self.finished || self.truncated || self.emitted >= BODY_CAP || input.is_empty() {
            if self.emitted >= BODY_CAP {
                self.truncated = true;
            }
            return Vec::new();
        }
        match self.pump(input, false) {
            Ok(bytes) => bytes,
            Err(err) => {
                self.error = Some(err);
                self.finished = true;
                Vec::new()
            }
        }
    }

    pub fn finish(&mut self) -> Vec<u8> {
        if self.finished || self.truncated || self.emitted >= BODY_CAP {
            self.finished = true;
            if self.emitted >= BODY_CAP {
                self.truncated = true;
            }
            return Vec::new();
        }
        self.finished = true;
        match self.pump(&[], true) {
            Ok(bytes) => bytes,
            Err(err) => {
                self.error = Some(err);
                Vec::new()
            }
        }
    }

    fn pump(&mut self, input: &[u8], finish: bool) -> Result<Vec<u8>, String> {
        let room = BODY_CAP.saturating_sub(self.emitted);
        let mut current = input.to_vec();
        let last = self.layers.len().saturating_sub(1);
        let mut final_hit_limit = false;
        for (index, layer) in self.layers.iter_mut().enumerate() {
            let is_last = index == last;
            let layer_limit = if is_last { room } else { BODY_CAP + 64 * 1024 };
            let (out, hit_limit) = layer.consume(&current, finish, layer_limit)?;
            current = out;
            if is_last && hit_limit {
                final_hit_limit = true;
            }
        }
        let out = self.take(current)?;
        if final_hit_limit {
            self.truncated = true;
        }
        Ok(out)
    }

    fn take(&mut self, mut bytes: Vec<u8>) -> Result<Vec<u8>, String> {
        if self.emitted >= BODY_CAP {
            self.truncated = true;
            return Ok(Vec::new());
        }
        let room = BODY_CAP - self.emitted;
        if bytes.len() > room {
            bytes.truncate(room);
            self.truncated = true;
        }
        self.emitted += bytes.len();
        Ok(bytes)
    }
}

impl Layer {
    fn consume(
        &mut self,
        input: &[u8],
        finish: bool,
        limit: usize,
    ) -> Result<(Vec<u8>, bool), String> {
        match self {
            Self::Identity => {
                if input.len() > limit {
                    Ok((input[..limit].to_vec(), true))
                } else {
                    Ok((input.to_vec(), false))
                }
            }
            Self::Gzip(decoder) => {
                decoder.get_mut().set_limit(limit);
                if !input.is_empty() {
                    decoder
                        .write_all(input)
                        .map_err(|err| format!("解压失败: {err}"))?;
                }
                if finish && !decoder.get_ref().hit_limit {
                    decoder
                        .try_finish()
                        .map_err(|err| format!("解压失败: {err}"))?;
                }
                Ok(decoder.get_mut().take())
            }
            Self::Deflate(inner) => consume_deflate(inner, input, finish, limit),
            Self::Brotli(decoder) => {
                decoder.get_mut().set_limit(limit);
                if !input.is_empty() {
                    decoder
                        .write_all(input)
                        .map_err(|err| format!("解压失败: {err}"))?;
                }
                if finish && !decoder.get_ref().hit_limit {
                    decoder.flush().map_err(|err| format!("解压失败: {err}"))?;
                    decoder.close().map_err(|err| format!("解压失败: {err}"))?;
                }
                Ok(decoder.get_mut().take())
            }
            Self::Zstd(decoder) => {
                decoder.get_mut().set_limit(limit);
                if !input.is_empty() {
                    decoder
                        .write_all(input)
                        .map_err(|err| format!("解压失败: {err}"))?;
                }
                if finish && !decoder.get_ref().hit_limit {
                    decoder
                        .do_finish()
                        .map_err(|err| format!("解压失败: {err}"))?;
                }
                Ok(decoder.get_mut().take())
            }
        }
    }
}

fn consume_deflate(
    inner: &mut DeflateInner,
    input: &[u8],
    finish: bool,
    limit: usize,
) -> Result<(Vec<u8>, bool), String> {
    let pending = if let DeflateInner::Sniff(head) = inner {
        let need = 2usize.saturating_sub(head.len());
        let take_n = input.len().min(need);
        head.extend_from_slice(&input[..take_n]);
        if head.len() < 2 && !finish {
            return Ok((Vec::new(), false));
        }
        let mut combined = std::mem::take(head);
        combined.extend_from_slice(&input[take_n..]);
        if combined.len() >= 2 && is_zlib_header(combined[0], combined[1]) {
            *inner = DeflateInner::Zlib(ZlibDecoder::new(CappedBuf::new(limit)));
        } else {
            *inner = DeflateInner::Raw(DeflateDecoder::new(CappedBuf::new(limit)));
        }
        Some(combined)
    } else {
        None
    };
    let to_write = pending.as_deref().unwrap_or(input);
    match inner {
        DeflateInner::Sniff(_) => unreachable!(),
        DeflateInner::Zlib(decoder) => {
            decoder.get_mut().set_limit(limit);
            if !to_write.is_empty() {
                decoder
                    .write_all(to_write)
                    .map_err(|err| format!("解压失败: {err}"))?;
            }
            if finish && !decoder.get_ref().hit_limit {
                decoder
                    .try_finish()
                    .map_err(|err| format!("解压失败: {err}"))?;
            }
            Ok(decoder.get_mut().take())
        }
        DeflateInner::Raw(decoder) => {
            decoder.get_mut().set_limit(limit);
            if !to_write.is_empty() {
                decoder
                    .write_all(to_write)
                    .map_err(|err| format!("解压失败: {err}"))?;
            }
            if finish && !decoder.get_ref().hit_limit {
                decoder
                    .try_finish()
                    .map_err(|err| format!("解压失败: {err}"))?;
            }
            Ok(decoder.get_mut().take())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
    use std::io::Write;

    #[test]
    fn gzip_roundtrip_and_cap() {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        let plain = vec![b'a'; 64];
        encoder.write_all(&plain).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut decoder = Decoder::from_header(Some("gzip"));
        let mut out = decoder.push(&compressed);
        out.extend(decoder.finish());
        assert_eq!(out, plain);
        assert!(decoder.error().is_none());
    }

    #[test]
    fn deflate_handles_both_zlib_rfc1950_and_raw_rfc1951() {
        let plain = b"hello standard zlib and raw deflate!".to_vec();

        // RFC 1950 zlib-wrapped deflate (standard HTTP Content-Encoding: deflate)
        let mut zlib_enc = ZlibEncoder::new(Vec::new(), Compression::default());
        zlib_enc.write_all(&plain).unwrap();
        let zlib_bytes = zlib_enc.finish().unwrap();

        let mut dec = Decoder::from_header(Some("deflate"));
        let mut out = dec.push(&zlib_bytes[..1]);
        out.extend(dec.push(&zlib_bytes[1..]));
        out.extend(dec.finish());
        assert_eq!(out, plain);
        assert!(dec.error().is_none());

        // Raw RFC 1951 deflate
        let mut raw_enc = DeflateEncoder::new(Vec::new(), Compression::default());
        raw_enc.write_all(&plain).unwrap();
        let raw_bytes = raw_enc.finish().unwrap();

        let mut dec_raw = Decoder::from_header(Some("deflate"));
        let mut out_raw = dec_raw.push(&raw_bytes[..1]);
        out_raw.extend(dec_raw.push(&raw_bytes[1..]));
        out_raw.extend(dec_raw.finish());
        assert_eq!(out_raw, plain);
        assert!(dec_raw.error().is_none());
    }

    #[test]
    fn brotli_zstd_and_multilayer_decode() {
        let plain = b"multi-layer compression payload\n".repeat(32);

        // Standalone zstd
        let zstd_bytes = zstd::stream::encode_all(plain.as_slice(), 1).unwrap();
        let mut zdec = Decoder::from_header(Some("zstd"));
        let mut zout = zdec.push(&zstd_bytes);
        zout.extend(zdec.finish());
        assert_eq!(zout, plain);
        assert!(zdec.error().is_none());

        // Multi-layer: gzip first, then brotli ("gzip, br")
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&plain).unwrap();
        let gz_bytes = gz.finish().unwrap();
        let mut br_bytes = Vec::new();
        {
            let mut br = brotli::CompressorWriter::new(&mut br_bytes, 4096, 5, 22);
            br.write_all(&gz_bytes).unwrap();
        }
        let mut mdec = Decoder::from_header(Some("gzip, br"));
        let mut mout = mdec.push(&br_bytes);
        mout.extend(mdec.finish());
        assert_eq!(mout, plain);
        assert!(mdec.error().is_none());
    }

    #[test]
    fn truncated_compressed_streams_report_error_on_finish() {
        let plain = b"payload that gets cut off mid-stream".repeat(16);

        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&plain).unwrap();
        let gz_bytes = gz.finish().unwrap();
        let mut dec = Decoder::from_header(Some("gzip"));
        let _ = dec.push(&gz_bytes[..gz_bytes.len() - 4]);
        let _ = dec.finish();
        assert!(dec.error().is_some(), "truncated gzip must report error");

        let mut br_bytes = Vec::new();
        {
            let mut br = brotli::CompressorWriter::new(&mut br_bytes, 4096, 5, 22);
            br.write_all(&plain).unwrap();
        }
        let mut bdec = Decoder::from_header(Some("br"));
        let _ = bdec.push(&br_bytes[..br_bytes.len() / 2]);
        let _ = bdec.finish();
        assert!(bdec.error().is_some(), "truncated brotli must report error");

        let zstd_bytes = zstd::stream::encode_all(plain.as_slice(), 1).unwrap();
        let mut zdec = Decoder::from_header(Some("zstd"));
        let _ = zdec.push(&zstd_bytes[..zstd_bytes.len() / 2]);
        let _ = zdec.finish();
        assert!(zdec.error().is_some(), "truncated zstd must report error");
    }

    #[test]
    fn single_chunk_compression_bomb_stays_capped_without_error() {
        let huge = vec![b'x'; BODY_CAP * 4];
        let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
        gz.write_all(&huge).unwrap();
        let bomb = gz.finish().unwrap();

        let mut decoder = Decoder::from_header(Some("gzip"));
        let mut out = decoder.push(&bomb);
        out.extend(decoder.finish());
        assert_eq!(out.len(), BODY_CAP);
        assert!(decoder.truncated());
        assert!(decoder.error().is_none());
    }

    #[test]
    fn identity_stops_at_the_body_cap() {
        let mut decoder = Decoder::from_header(None);
        let chunk = vec![1u8; BODY_CAP + 10];
        let out = decoder.push(&chunk);
        assert_eq!(out.len(), BODY_CAP);
        assert!(decoder.truncated());
    }
}
