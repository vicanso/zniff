//! Enough of a TLS ClientHello to read SNI and ALPN, plus the raw bytes
//! so a failed probe can be replayed onto a fresh origin connection.

use tokio::io::{AsyncRead, AsyncReadExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientHello {
    pub sni: Option<String>,
    pub alpn: Vec<Vec<u8>>,
    /// Every byte read, including full TLS records.
    pub raw: Vec<u8>,
}

#[derive(Debug)]
pub enum ReadHello {
    Hello(ClientHello),
    /// Not a ClientHello. `raw` is what was consumed and must be replayed.
    NotTls {
        raw: Vec<u8>,
    },
}

/// Reads one ClientHello. Stops once the handshake message is complete,
/// or as soon as the bytes are clearly not TLS.
pub async fn read_client_hello<R: AsyncRead + Unpin>(reader: &mut R) -> std::io::Result<ReadHello> {
    let mut raw = Vec::new();
    let mut handshake = Vec::new();
    loop {
        let mut header = [0u8; 5];
        if raw.is_empty() {
            match reader.read_exact(&mut header).await {
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(ReadHello::NotTls { raw });
                }
                Err(err) => return Err(err),
            }
        } else {
            reader.read_exact(&mut header).await?;
        }
        if header[0] != 0x16 {
            raw.extend_from_slice(&header);
            return Ok(ReadHello::NotTls { raw });
        }
        let record_len = u16::from_be_bytes([header[3], header[4]]) as usize;
        if record_len > 16 * 1024 + 2048 {
            raw.extend_from_slice(&header);
            return Ok(ReadHello::NotTls { raw });
        }
        let mut fragment = vec![0u8; record_len];
        if let Err(err) = reader.read_exact(&mut fragment).await {
            raw.extend_from_slice(&header);
            raw.extend_from_slice(&fragment);
            if err.kind() == std::io::ErrorKind::UnexpectedEof {
                return Ok(ReadHello::NotTls { raw });
            }
            return Err(err);
        }
        raw.extend_from_slice(&header);
        raw.extend_from_slice(&fragment);
        handshake.extend_from_slice(&fragment);
        match parse_handshake(&handshake) {
            Parse::NeedMore => continue,
            Parse::NotHello => return Ok(ReadHello::NotTls { raw }),
            Parse::Done { sni, alpn } => {
                return Ok(ReadHello::Hello(ClientHello { sni, alpn, raw }));
            }
        }
    }
}

enum Parse {
    NeedMore,
    NotHello,
    Done {
        sni: Option<String>,
        alpn: Vec<Vec<u8>>,
    },
}

fn parse_handshake(buf: &[u8]) -> Parse {
    if buf.is_empty() {
        return Parse::NeedMore;
    }
    if buf[0] != 0x01 {
        return Parse::NotHello;
    }
    if buf.len() < 4 {
        return Parse::NeedMore;
    }
    let msg_len = u24(&buf[1..4]);
    if buf.len() < 4 + msg_len {
        return Parse::NeedMore;
    }
    let body = &buf[4..4 + msg_len];
    match parse_client_hello_body(body) {
        Some((sni, alpn)) => Parse::Done { sni, alpn },
        None => Parse::NotHello,
    }
}

fn parse_client_hello_body(body: &[u8]) -> Option<(Option<String>, Vec<Vec<u8>>)> {
    if body.len() < 34 {
        return None;
    }
    let mut i = 34; // version + random
    let session_len = *body.get(i)? as usize;
    i += 1 + session_len;
    if i + 2 > body.len() {
        return None;
    }
    let cipher_len = u16::from_be_bytes([body[i], body[i + 1]]) as usize;
    i += 2 + cipher_len;
    let comp_len = *body.get(i)? as usize;
    i += 1 + comp_len;
    if i == body.len() {
        return Some((None, Vec::new()));
    }
    if i + 2 > body.len() {
        return None;
    }
    let ext_len = u16::from_be_bytes([body[i], body[i + 1]]) as usize;
    i += 2;
    let ext_end = i.checked_add(ext_len)?;
    if ext_end > body.len() {
        return None;
    }
    let mut sni = None;
    let mut alpn = Vec::new();
    while i + 4 <= ext_end {
        let kind = u16::from_be_bytes([body[i], body[i + 1]]);
        let len = u16::from_be_bytes([body[i + 2], body[i + 3]]) as usize;
        i += 4;
        let data = body.get(i..i + len)?;
        i += len;
        match kind {
            0x0000 => sni = parse_sni(data).or(sni),
            0x0010 => alpn = parse_alpn(data),
            _ => {}
        }
    }
    Some((sni, alpn))
}

fn parse_sni(data: &[u8]) -> Option<String> {
    if data.len() < 5 {
        return None;
    }
    let list_len = u16::from_be_bytes([data[0], data[1]]) as usize;
    if data.len() < 2 + list_len || data[2] != 0x00 {
        return None;
    }
    let name_len = u16::from_be_bytes([data[3], data[4]]) as usize;
    let name = data.get(5..5 + name_len)?;
    let text = String::from_utf8(name.to_vec()).ok()?;
    if text.is_empty() { None } else { Some(text) }
}

fn parse_alpn(data: &[u8]) -> Vec<Vec<u8>> {
    if data.len() < 2 {
        return Vec::new();
    }
    let list_len = u16::from_be_bytes([data[0], data[1]]) as usize;
    let mut i = 2;
    let end = (2 + list_len).min(data.len());
    let mut out = Vec::new();
    while i < end {
        let len = data[i] as usize;
        i += 1;
        let Some(proto) = data.get(i..i + len) else {
            break;
        };
        out.push(proto.to_vec());
        i += len;
    }
    out
}

fn u24(bytes: &[u8]) -> usize {
    ((bytes[0] as usize) << 16) | ((bytes[1] as usize) << 8) | bytes[2] as usize
}

/// Builds a one-record ClientHello for tests.
#[cfg(test)]
pub fn craft_client_hello(sni: Option<&str>, alpn: &[&[u8]]) -> Vec<u8> {
    let mut extensions = Vec::new();
    if let Some(name) = sni {
        let mut body = Vec::new();
        let name = name.as_bytes();
        body.extend_from_slice(&(name.len() as u16 + 3).to_be_bytes());
        body.push(0);
        body.extend_from_slice(&(name.len() as u16).to_be_bytes());
        body.extend_from_slice(name);
        push_ext(&mut extensions, 0x0000, &body);
    }
    if !alpn.is_empty() {
        let mut list = Vec::new();
        for proto in alpn {
            list.push(proto.len() as u8);
            list.extend_from_slice(proto);
        }
        let mut body = Vec::new();
        body.extend_from_slice(&(list.len() as u16).to_be_bytes());
        body.extend_from_slice(&list);
        push_ext(&mut extensions, 0x0010, &body);
    }
    let mut hello = Vec::new();
    hello.extend_from_slice(&[0x03, 0x03]);
    hello.extend_from_slice(&[0x11; 32]);
    hello.push(0); // session id
    hello.extend_from_slice(&2u16.to_be_bytes());
    hello.extend_from_slice(&[0x00, 0x2f]); // one cipher
    hello.push(1);
    hello.push(0); // null compression
    hello.extend_from_slice(&(extensions.len() as u16).to_be_bytes());
    hello.extend_from_slice(&extensions);

    let mut message = Vec::new();
    message.push(0x01);
    let len = hello.len();
    message.push((len >> 16) as u8);
    message.push((len >> 8) as u8);
    message.push(len as u8);
    message.extend_from_slice(&hello);

    let mut record = Vec::new();
    record.push(0x16);
    record.extend_from_slice(&[0x03, 0x01]);
    record.extend_from_slice(&(message.len() as u16).to_be_bytes());
    record.extend_from_slice(&message);
    record
}

#[cfg(test)]
fn push_ext(out: &mut Vec<u8>, kind: u16, body: &[u8]) {
    out.extend_from_slice(&kind.to_be_bytes());
    out.extend_from_slice(&(body.len() as u16).to_be_bytes());
    out.extend_from_slice(body);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_sni_and_alpn() {
        let raw = craft_client_hello(Some("example.com"), &[b"h2", b"http/1.1"]);
        let parsed = match futures_block(&raw) {
            ReadHello::Hello(hello) => hello,
            ReadHello::NotTls { .. } => panic!("expected hello"),
        };
        assert_eq!(parsed.sni.as_deref(), Some("example.com"));
        assert_eq!(parsed.alpn, vec![b"h2".to_vec(), b"http/1.1".to_vec()]);
        assert_eq!(parsed.raw, raw);
    }

    #[test]
    fn rejects_non_tls() {
        let raw = b"GET / HTTP/1.1\r\n\r\n".to_vec();
        match futures_block(&raw) {
            ReadHello::NotTls { raw: got } => assert_eq!(&got[..4], b"GET "),
            ReadHello::Hello(_) => panic!("expected not tls"),
        }
    }

    fn futures_block(bytes: &[u8]) -> ReadHello {
        let mut cursor = Cursor::new(bytes.to_vec());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(read_client_hello(&mut cursor)).unwrap()
    }
}
