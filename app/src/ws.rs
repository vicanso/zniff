//! WebSocket frame parser. Bytes are forwarded immediately; this only
//! watches a copy. Invalid framing stops the parser and leaves the splice
//! alone.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsFrame {
    pub opcode: u8,
    pub payload: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct FrameFeed {
    buf: Vec<u8>,
    gave_up: bool,
}

impl FrameFeed {
    pub fn push(&mut self, data: &[u8]) -> Vec<WsFrame> {
        if self.gave_up || data.is_empty() {
            return Vec::new();
        }
        self.buf.extend_from_slice(data);
        let mut frames = Vec::new();
        loop {
            match take_frame(&self.buf) {
                Ok(Some((consumed, frame))) => {
                    self.buf.drain(..consumed);
                    frames.push(frame);
                }
                Ok(None) => break,
                Err(()) => {
                    self.gave_up = true;
                    self.buf.clear();
                    break;
                }
            }
        }
        if self.buf.len() > 1024 * 1024 {
            self.gave_up = true;
            self.buf.clear();
        }
        frames
    }
}

pub fn frame_label(opcode: u8) -> &'static str {
    match opcode {
        0x1 => "文本",
        0x2 => "二进制",
        0x8 => "关闭",
        0x9 => "Ping",
        0xA => "Pong",
        0x0 => "续帧",
        _ => "帧",
    }
}

pub fn frame_text(opcode: u8, payload: &[u8]) -> String {
    if opcode == 0x1 || opcode == 0x8 || opcode == 0x0 {
        String::from_utf8_lossy(payload).into_owned()
    } else if payload.is_empty() {
        String::new()
    } else {
        let shown = payload.len().min(64);
        let mut out = String::new();
        for byte in &payload[..shown] {
            out.push_str(&format!("{byte:02x}"));
        }
        if payload.len() > shown {
            out.push_str(&format!("…{}B", payload.len()));
        }
        out
    }
}

fn take_frame(buf: &[u8]) -> Result<Option<(usize, WsFrame)>, ()> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let b0 = buf[0];
    let b1 = buf[1];
    if b0 & 0x70 != 0 {
        return Err(());
    }
    let opcode = b0 & 0x0f;
    if opcode > 0xA || matches!(opcode, 0x3..=0x7) {
        return Err(());
    }
    let masked = b1 & 0x80 != 0;
    let mut len = (b1 & 0x7f) as u64;
    let mut offset = 2;
    if len == 126 {
        if buf.len() < 4 {
            return Ok(None);
        }
        len = u16::from_be_bytes([buf[2], buf[3]]) as u64;
        offset = 4;
        if len < 126 {
            return Err(());
        }
    } else if len == 127 {
        if buf.len() < 10 {
            return Ok(None);
        }
        len = u64::from_be_bytes(buf[2..10].try_into().unwrap());
        offset = 10;
        if len < 65536 || len > 8 * 1024 * 1024 {
            return Err(());
        }
    }
    if matches!(opcode, 0x8 | 0x9 | 0xA) && len > 125 {
        return Err(());
    }
    let mask_len = if masked { 4 } else { 0 };
    let total = offset + mask_len + len as usize;
    if buf.len() < total {
        return Ok(None);
    }
    let mut payload = buf[offset + mask_len..total].to_vec();
    if masked {
        let key = &buf[offset..offset + 4];
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= key[index % 4];
        }
    }
    Ok(Some((
        total,
        WsFrame { opcode, payload },
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_masked_text_frame() {
        let payload = b"hi";
        let key = [1u8, 2, 3, 4];
        let mut frame = vec![0x81, 0x80 | payload.len() as u8];
        frame.extend_from_slice(&key);
        for (index, byte) in payload.iter().enumerate() {
            frame.push(byte ^ key[index % 4]);
        }
        let mut feed = FrameFeed::default();
        let parsed = feed.push(&frame);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].opcode, 0x1);
        assert_eq!(parsed[0].payload, b"hi");
        assert!(feed.push(b"ping").is_empty());
    }
}
