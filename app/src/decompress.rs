//! Streaming Content-Encoding decode for the side copy.
//!
//! The forwarded bytes stay compressed. Only the recorded copy is decoded.
//! The cap is on decoded bytes.

use std::io::Write;

use flate2::write::{DeflateDecoder, GzDecoder};

use crate::session::BODY_CAP;

#[derive(Debug)]
pub struct Decoder {
    layers: Vec<Layer>,
    emitted: usize,
    truncated: bool,
    error: Option<String>,
    finished: bool,
}

enum Layer {
    Identity,
    Gzip(GzDecoder<Vec<u8>>),
    Deflate(DeflateDecoder<Vec<u8>>),
    Brotli(Box<brotli::DecompressorWriter<Vec<u8>>>),
    Zstd(zstd::stream::write::Decoder<'static, Vec<u8>>),
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
                "gzip" | "x-gzip" => layers.push(Layer::Gzip(GzDecoder::new(Vec::new()))),
                "deflate" => layers.push(Layer::Deflate(DeflateDecoder::new(Vec::new()))),
                "br" => layers.push(Layer::Brotli(Box::new(brotli::DecompressorWriter::new(
                    Vec::new(),
                    4096,
                )))),
                "zstd" => match zstd::stream::write::Decoder::new(Vec::new()) {
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
        if self.finished {
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
        let mut current = input.to_vec();
        let last = self.layers.len().saturating_sub(1);
        for (index, layer) in self.layers.iter_mut().enumerate() {
            let end = finish && index == last;
            current = layer.consume(&current, finish)?;
            if end {
                break;
            }
        }
        // Intermediate layers must flush on finish too. `finish` is applied
        // to every layer so gzip trailers are consumed before the next layer.
        if finish && self.layers.len() > 1 {
            // `consume(..., finish)` already ran for each layer above.
        }
        self.take(current)
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
    fn consume(&mut self, input: &[u8], finish: bool) -> Result<Vec<u8>, String> {
        match self {
            Self::Identity => Ok(input.to_vec()),
            Self::Gzip(decoder) => write_new(decoder, input, finish),
            Self::Deflate(decoder) => write_new(decoder, input, finish),
            Self::Brotli(decoder) => write_new(decoder.as_mut(), input, finish),
            Self::Zstd(decoder) => write_new(decoder, input, finish),
        }
    }
}

fn write_new<W: Write + OutputBuf>(
    decoder: &mut W,
    input: &[u8],
    finish: bool,
) -> Result<Vec<u8>, String> {
    let before = decoder.output_len();
    if !input.is_empty() {
        decoder
            .write_all(input)
            .map_err(|err| format!("解压失败: {err}"))?;
    }
    if finish {
        decoder.flush().ok();
        // GzDecoder finishes on drop; try finish if the type supports it via flush.
    }
    let output = decoder.output_buf();
    Ok(output.get(before..).unwrap_or_default().to_vec())
}

trait OutputBuf {
    fn output_len(&self) -> usize;
    fn output_buf(&self) -> &[u8];
}

impl OutputBuf for GzDecoder<Vec<u8>> {
    fn output_len(&self) -> usize {
        self.get_ref().len()
    }
    fn output_buf(&self) -> &[u8] {
        self.get_ref()
    }
}

impl OutputBuf for DeflateDecoder<Vec<u8>> {
    fn output_len(&self) -> usize {
        self.get_ref().len()
    }
    fn output_buf(&self) -> &[u8] {
        self.get_ref()
    }
}

impl OutputBuf for brotli::DecompressorWriter<Vec<u8>> {
    fn output_len(&self) -> usize {
        self.get_ref().len()
    }
    fn output_buf(&self) -> &[u8] {
        self.get_ref()
    }
}

impl OutputBuf for zstd::stream::write::Decoder<'static, Vec<u8>> {
    fn output_len(&self) -> usize {
        self.get_ref().len()
    }
    fn output_buf(&self) -> &[u8] {
        self.get_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
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
    fn identity_stops_at_the_body_cap() {
        let mut decoder = Decoder::from_header(None);
        let chunk = vec![1u8; BODY_CAP + 10];
        let out = decoder.push(&chunk);
        assert_eq!(out.len(), BODY_CAP);
        assert!(decoder.truncated());
    }
}
