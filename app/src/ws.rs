//! WebSocket frame parser. Bytes are forwarded immediately; this only
//! watches a copy. Invalid framing stops the parser and leaves the splice
//! alone.

#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

const MAX_FRAME_PAYLOAD: u64 = 8 * 1024 * 1024;
const MAX_FRAME_BUF: usize = MAX_FRAME_PAYLOAD as usize + 14;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsFrame {
    pub opcode: u8,
    pub payload: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct FrameFeed {
    buf: Vec<u8>,
    fragment_opcode: Option<u8>,
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
                Ok(Some((consumed, fin, mut frame))) => {
                    self.buf.drain(..consumed);
                    match frame.opcode {
                        0x1 | 0x2 => {
                            if self.fragment_opcode.is_some() {
                                self.gave_up = true;
                                self.buf.clear();
                                break;
                            }
                            if !fin {
                                self.fragment_opcode = Some(frame.opcode);
                            }
                        }
                        0x0 => {
                            if let Some(orig) = self.fragment_opcode {
                                frame.opcode = orig;
                                if fin {
                                    self.fragment_opcode = None;
                                }
                            }
                        }
                        _ => {}
                    }
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
        if self.buf.len() > MAX_FRAME_BUF {
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
    match opcode {
        0x1 | 0x0 => String::from_utf8_lossy(payload).into_owned(),
        0x8 => {
            if payload.is_empty() {
                String::new()
            } else if payload.len() >= 2 {
                let code = u16::from_be_bytes([payload[0], payload[1]]);
                let reason = String::from_utf8_lossy(&payload[2..]);
                if reason.is_empty() {
                    code.to_string()
                } else {
                    format!("{code} {reason}")
                }
            } else {
                format!("{:02x}", payload[0])
            }
        }
        _ if payload.is_empty() => String::new(),
        _ => {
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
}

fn take_frame(buf: &[u8]) -> Result<Option<(usize, bool, WsFrame)>, ()> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let b0 = buf[0];
    let b1 = buf[1];
    if b0 & 0x70 != 0 {
        return Err(());
    }
    let fin = b0 & 0x80 != 0;
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
        if !(65536..=MAX_FRAME_PAYLOAD).contains(&len) {
            return Err(());
        }
    }
    if matches!(opcode, 0x8..=0xA) && (!fin || len > 125) {
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
    Ok(Some((total, fin, WsFrame { opcode, payload })))
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

    #[test]
    fn close_status_code_and_reason_are_decoded() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&1000u16.to_be_bytes());
        payload.extend_from_slice(b"normal closure");
        assert_eq!(frame_text(0x8, &payload), "1000 normal closure");
        assert_eq!(frame_text(0x8, &1001u16.to_be_bytes()), "1001");
        assert_eq!(frame_text(0x8, &[]), "");
    }

    #[test]
    fn continuation_frames_inherit_text_or_binary_opcode_even_across_ping() {
        let mut feed = FrameFeed::default();
        // Fragmented binary message: 0x02 (!fin) -> Ping (0x89, fin) -> 0x80 (fin)
        let mut stream = Vec::new();
        stream.extend_from_slice(&[0x02, 0x02, 0xde, 0xad]);
        stream.extend_from_slice(&[0x89, 0x01, 0x42]);
        stream.extend_from_slice(&[0x80, 0x02, 0xbe, 0xef]);

        let frames = feed.push(&stream);
        assert_eq!(frames.len(), 3);
        assert_eq!(frame_label(frames[0].opcode), "二进制");
        assert_eq!(frame_text(frames[0].opcode, &frames[0].payload), "dead");
        assert_eq!(frame_label(frames[1].opcode), "Ping");
        assert_eq!(frame_label(frames[2].opcode), "二进制");
        assert_eq!(frame_text(frames[2].opcode, &frames[2].payload), "beef");

        // Next fragmented text message: 0x01 (!fin) -> 0x80 (fin)
        let frames = feed.push(&[0x01, 0x02, b'h', b'i', 0x80, 0x01, b'!']);
        assert_eq!(frames.len(), 2);
        assert_eq!(frame_label(frames[0].opcode), "文本");
        assert_eq!(frame_text(frames[0].opcode, &frames[0].payload), "hi");
        assert_eq!(frame_label(frames[1].opcode), "文本");
        assert_eq!(frame_text(frames[1].opcode, &frames[1].payload), "!");
    }

    #[test]
    fn extended_lengths_126_and_127_over_1mib_and_invalid_control_frames() {
        // 126 extended length (200 bytes)
        let mut f126 = vec![0x81, 126];
        f126.extend_from_slice(&200u16.to_be_bytes());
        f126.extend(std::iter::repeat_n(b'a', 200));

        // 127 extended length (1.5 MiB split across multiple pushes — previously dropped by 1 MiB cap)
        let big_len = 1536 * 1024;
        let mut f127 = vec![0x82, 127];
        f127.extend_from_slice(&(big_len as u64).to_be_bytes());
        f127.extend(std::iter::repeat_n(0xab, big_len));

        let mut feed = FrameFeed::default();
        assert_eq!(feed.push(&f126).len(), 1);
        // Push first 1.2 MiB (incomplete frame > 1 MiB), then the rest
        assert!(feed.push(&f127[..1200 * 1024]).is_empty());
        let done = feed.push(&f127[1200 * 1024..]);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].payload.len(), big_len);

        // Fragmented control frame (Ping with FIN=0: 0x09) must stop parser
        let mut bad_ctrl = FrameFeed::default();
        assert!(bad_ctrl.push(&[0x09, 0x00, 0x81, 0x01, b'x']).is_empty());
        assert!(bad_ctrl.push(&[0x81, 0x01, b'y']).is_empty());

        // RSV1 set (0xC1) must stop parser cleanly
        let mut rsv = FrameFeed::default();
        assert!(rsv.push(&[0xc1, 0x01, b'x']).is_empty());
        assert!(rsv.push(&[0x81, 0x01, b'y']).is_empty());
    }
}
