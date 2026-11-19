#[allow(unused_imports)]
use super::*;

pub(super) fn file_type(path: &std::path::Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "json" | "map" => "application/json",
        "js" | "mjs" | "cjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "wasm" => "application/wasm",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "pdf" => "application/pdf",
        "txt" | "log" | "csv" => "text/plain",
        _ => "application/octet-stream",
    }
}

pub(super) const THROTTLE_CHUNK_BYTES: usize = 4096;

pub(super) fn throttle_sleep_duration(bytes: usize, bps: Option<u64>) -> Duration {
    let Some(bps) = bps.filter(|&rate| rate > 0) else {
        return Duration::ZERO;
    };
    if bytes == 0 {
        return Duration::ZERO;
    }
    let nanos = (bytes as u128).saturating_mul(1_000_000_000) / (bps as u128);
    Duration::from_nanos(nanos.min(30_000_000_000) as u64)
}

pub(super) async fn write_all_throttled<W>(to: &mut W, data: &[u8], bps: Option<u64>) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let Some(bps) = bps.filter(|&rate| rate > 0) else {
        to.write_all(data).await?;
        return Ok(());
    };
    for chunk in data.chunks(THROTTLE_CHUNK_BYTES) {
        to.write_all(chunk).await?;
        let pause = throttle_sleep_duration(chunk.len(), Some(bps));
        if !pause.is_zero() {
            tokio::time::sleep(pause).await;
        }
    }
    Ok(())
}

pub(super) fn response_has_no_body(method: &str, status: u16) -> bool {
    method.eq_ignore_ascii_case("HEAD") || matches!(status, 100..=199 | 204 | 304)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn copy_message_body<R, W>(
    from: &mut R,
    to: &mut W,
    headers: &[(String, String)],
    hub: &Hub,
    id: u64,
    direction: Direction,
    encoding: Option<String>,
    throttle_bps: Option<u64>,
    no_body: bool,
) -> Result<bool>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    if no_body {
        return Ok(true);
    }
    let mut decoder = Decoder::from_header(encoding.as_deref());
    if let Some(error) = decoder.error() {
        hub.set_encoding_error(id, direction, error.to_string());
    }
    let chunk_cap = if throttle_bps.is_some() {
        THROTTLE_CHUNK_BYTES
    } else {
        16 * 1024
    };
    let reusable = if let Some(length) = content_length(headers) {
        let mut left = length;
        let mut buf = vec![0u8; chunk_cap];
        while left > 0 {
            let want = left.min(buf.len());
            let n = from.read(&mut buf[..want]).await?;
            if n == 0 {
                break;
            }
            write_all_throttled(to, &buf[..n], throttle_bps).await?;
            let decoded = decoder.push(&buf[..n]);
            let error = decoder.error().map(|err| err.to_string());
            hub.record_chunk(id, direction, n as u64, &decoded, error.as_deref());
            left -= n;
        }
        left == 0
    } else if is_chunked(headers) {
        copy_chunked(from, to, hub, id, direction, &mut decoder, throttle_bps).await?;
        true
    } else if direction == Direction::Response {
        let mut buf = vec![0u8; chunk_cap];
        loop {
            let n = from.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            write_all_throttled(to, &buf[..n], throttle_bps).await?;
            let decoded = decoder.push(&buf[..n]);
            let error = decoder.error().map(|err| err.to_string());
            hub.record_chunk(id, direction, n as u64, &decoded, error.as_deref());
        }
        false
    } else {
        true
    };
    let tail = decoder.finish();
    let error = decoder.error().map(|err| err.to_string());
    hub.record_chunk(id, direction, 0, &tail, error.as_deref());
    Ok(reusable)
}

pub(super) async fn copy_chunked<R, W>(
    from: &mut R,
    to: &mut W,
    hub: &Hub,
    id: u64,
    direction: Direction,
    decoder: &mut Decoder,
    throttle_bps: Option<u64>,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let chunk_cap = if throttle_bps.is_some() {
        THROTTLE_CHUNK_BYTES
    } else {
        16 * 1024
    };
    loop {
        let size_line = read_line(from).await?;
        to.write_all(&size_line).await?;
        let text = String::from_utf8_lossy(&size_line);
        let size_text = text.trim().split(';').next().unwrap_or("0");
        let size = usize::from_str_radix(size_text.trim(), 16).unwrap_or(0);
        if size == 0 {
            let trailer = read_line(from).await?;
            to.write_all(&trailer).await?;
            break;
        }
        let mut left = size;
        let mut buf = vec![0u8; chunk_cap];
        while left > 0 {
            let want = left.min(buf.len());
            let n = from.read(&mut buf[..want]).await?;
            if n == 0 {
                anyhow::bail!("分块 body 提前结束");
            }
            write_all_throttled(to, &buf[..n], throttle_bps).await?;
            let decoded = decoder.push(&buf[..n]);
            let error = decoder.error().map(|err| err.to_string());
            hub.record_chunk(id, direction, n as u64, &decoded, error.as_deref());
            left -= n;
        }
        let crlf = read_exact(from, 2).await?;
        to.write_all(&crlf).await?;
    }
    Ok(())
}

pub(super) fn status_and_reason(start: &str) -> (u16, String) {
    let mut parts = start.split_whitespace();
    let _version = parts.next();
    let status = parts.next().and_then(|code| code.parse().ok()).unwrap_or(0);
    let reason: String = parts.collect::<Vec<_>>().join(" ");
    let reason = if reason.is_empty() {
        reason_phrase(status).to_string()
    } else {
        reason
    };
    (status, reason)
}

pub(super) fn reason_phrase(status: u16) -> &'static str {
    StatusCode::from_u16(status)
        .ok()
        .and_then(|code| code.canonical_reason())
        .unwrap_or("OK")
}

pub(super) fn render_response_head(
    status: u16,
    reason: &str,
    headers: &[(String, String)],
) -> String {
    let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    head
}

pub(super) async fn write_bytes<S>(
    client: &mut S,
    status: u16,
    reason: &str,
    headers: &[(String, String)],
    body: &[u8],
    throttle_bps: Option<u64>,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let mut headers: Vec<(String, String)> = headers
        .iter()
        .filter(|(name, _)| {
            !name.eq_ignore_ascii_case("content-length")
                && !name.eq_ignore_ascii_case("transfer-encoding")
        })
        .cloned()
        .collect();
    headers.push(("Content-Length".into(), body.len().to_string()));
    let head = render_response_head(status, reason, &headers);
    client.write_all(head.as_bytes()).await?;
    write_all_throttled(client, body, throttle_bps).await?;
    Ok(())
}

/// `Ok(None)` means the body is larger than `cap` and was not consumed.
pub(super) async fn read_capped_body<R>(
    reader: &mut R,
    headers: &[(String, String)],
    cap: usize,
    no_body: bool,
) -> Result<Option<Vec<u8>>>
where
    R: AsyncRead + Unpin,
{
    if no_body {
        return Ok(Some(Vec::new()));
    }
    if let Some(length) = content_length(headers) {
        if length > cap {
            return Ok(None);
        }
        return Ok(Some(read_exact(reader, length).await?));
    }
    if is_chunked(headers) {
        let mut out = Vec::new();
        loop {
            let size_line = read_line(reader).await?;
            let text = String::from_utf8_lossy(&size_line);
            let size =
                usize::from_str_radix(text.trim().split(';').next().unwrap_or("0").trim(), 16)
                    .unwrap_or(0);
            if size == 0 {
                let _ = read_line(reader).await?;
                break;
            }
            if out.len().saturating_add(size) > cap {
                anyhow::bail!("响应断点跳过，body 超过 1 MB");
            }
            out.extend(read_exact(reader, size).await?);
            let _ = read_exact(reader, 2).await?;
        }
        return Ok(Some(out));
    }
    let mut out = Vec::new();
    let mut buf = vec![0u8; 8 * 1024];
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        if out.len() + n > cap {
            anyhow::bail!("响应断点跳过，body 超过 1 MB");
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok(Some(out))
}

pub(super) fn content_length(headers: &[(String, String)]) -> Option<usize> {
    header_value(headers, "content-length")?.parse().ok()
}

pub(super) fn is_chunked(headers: &[(String, String)]) -> bool {
    header_value(headers, "transfer-encoding")
        .map(|value| value.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false)
}

/// Drops `Sec-WebSocket-Extensions` so a handshake cannot negotiate
/// `permessage-deflate`. The forwarded frames stay uncompressed and the
/// side-copy parser can read them. RSV1 on a frame still stops parsing only.
pub(super) fn strip_ws_extension(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for line in bytes.split_inclusive(|&byte| byte == b'\n') {
        let name = line.split(|&byte| byte == b':').next().unwrap_or(line);
        let name = trim_ascii(name);
        if name.eq_ignore_ascii_case(b"sec-websocket-extensions") {
            continue;
        }
        out.extend_from_slice(line);
    }
    out
}

pub(super) fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map(|index| index + 1)
        .unwrap_or(start);
    &bytes[start..end]
}

pub(super) fn header_has_expect_100_continue(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("expect")
            && value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("100-continue"))
    })
}

pub(super) fn header_is_websocket(headers: &[(String, String)]) -> bool {
    header_value(headers, "upgrade")
        .map(|value| value.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
}

pub(super) fn header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

pub(super) fn request_target(head: &Head) -> Result<(String, String, u16, String)> {
    let mut parts = head.start.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    if let Some(rest) = target
        .strip_prefix("http://")
        .or_else(|| target.strip_prefix("https://"))
    {
        let (authority, path) = match rest.find('/') {
            Some(index) => (&rest[..index], rest[index..].to_string()),
            None => (rest, "/".to_string()),
        };
        let default_port = if target.starts_with("https://") {
            443
        } else {
            80
        };
        let (host, port) = split_host_port(authority, default_port)?;
        let path = if path.is_empty() { "/".into() } else { path };
        return Ok((method, host, port, path));
    }
    let host_header = header_value(&head.headers, "host").unwrap_or_default();
    let (host, port) = split_host_port(&host_header, 80)?;
    Ok((method, host, port, target))
}

pub(super) fn split_host_port(authority: &str, default_port: u16) -> Result<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, port) = rest.split_once(']').context("IPv6 主机")?;
        let port = port.trim_start_matches(':');
        let port = if port.is_empty() {
            default_port
        } else {
            port.parse().context("端口")?
        };
        return Ok((host.to_string(), port));
    }
    if let Some((host, port)) = authority.rsplit_once(':')
        && let Ok(port) = port.parse::<u16>()
    {
        return Ok((host.to_string(), port));
    }
    if authority.is_empty() {
        anyhow::bail!("缺少主机名");
    }
    Ok((authority.to_string(), default_port))
}

pub(super) struct Head {
    pub(super) start: String,
    pub(super) headers: Vec<(String, String)>,
}

pub(super) fn parse_head(bytes: &[u8]) -> Result<Head> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.split("\r\n");
    let start = lines.next().unwrap_or("").trim().to_string();
    if start.is_empty() {
        anyhow::bail!("空的请求头");
    }
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }
    Ok(Head { start, headers })
}

pub(super) async fn read_header_block<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<(Vec<u8>, Vec<u8>)> {
    read_header_block_prefixed(reader, Vec::new()).await
}

pub(super) async fn read_header_block_prefixed<R: AsyncRead + Unpin>(
    reader: &mut R,
    mut buf: Vec<u8>,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut tmp = [0u8; 2048];
    loop {
        if let Some(end) = header_end(&buf) {
            let rest = buf.split_off(end);
            return Ok((buf, rest));
        }
        if buf.len() > 64 * 1024 {
            anyhow::bail!("请求头超过 64KB");
        }
        let n = timeout(IO_TIMEOUT, reader.read(&mut tmp))
            .await
            .context("读请求头超时")??;
        if n == 0 {
            anyhow::bail!("连接在请求头结束前关闭");
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

pub(super) fn header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

pub(super) async fn read_line<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = reader.read(&mut byte).await?;
        if n == 0 {
            anyhow::bail!("分块大小行提前结束");
        }
        line.push(byte[0]);
        if line.ends_with(b"\r\n") {
            return Ok(line);
        }
        if line.len() > 4096 {
            anyhow::bail!("分块大小行过长");
        }
    }
}

pub(super) async fn read_exact<R: AsyncRead + Unpin>(
    reader: &mut R,
    len: usize,
) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

pub(super) enum LimitedRead {
    Whole(Vec<u8>),
    Overflow { wire_prefix: Vec<u8> },
}

pub(super) async fn read_chunked_limited<R: AsyncRead + Unpin>(
    reader: &mut R,
    cap: usize,
) -> Result<LimitedRead> {
    let mut payload = Vec::new();
    let mut wire = Vec::new();
    loop {
        let size_line = read_line(reader).await?;
        wire.extend_from_slice(&size_line);
        let text = String::from_utf8_lossy(&size_line);
        let size_text = text.trim().split(';').next().unwrap_or("0").trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| anyhow!("非法分块长度"))?;
        if size == 0 {
            loop {
                let trailer = read_line(reader).await?;
                wire.extend_from_slice(&trailer);
                if trailer == b"\r\n" || trailer == b"\n" || trailer.is_empty() {
                    break;
                }
            }
            return Ok(LimitedRead::Whole(payload));
        }
        if payload.len().saturating_add(size) > cap {
            return Ok(LimitedRead::Overflow { wire_prefix: wire });
        }
        let chunk = read_exact(reader, size).await?;
        wire.extend_from_slice(&chunk);
        payload.extend_from_slice(&chunk);
        let crlf = read_exact(reader, 2).await?;
        wire.extend_from_slice(&crlf);
    }
}

pub(super) async fn read_eof_limited<R: AsyncRead + Unpin>(
    reader: &mut R,
    cap: usize,
) -> Result<LimitedRead> {
    let mut buf = Vec::new();
    let mut scratch = vec![0u8; 8 * 1024];
    loop {
        let n = reader.read(&mut scratch).await?;
        if n == 0 {
            return Ok(LimitedRead::Whole(buf));
        }
        if buf.len().saturating_add(n) > cap {
            buf.extend_from_slice(&scratch[..n]);
            return Ok(LimitedRead::Overflow { wire_prefix: buf });
        }
        buf.extend_from_slice(&scratch[..n]);
    }
}

pub(super) struct Prefixed<S> {
    pub(super) prefix: Vec<u8>,
    pub(super) pos: usize,
    pub(super) inner: S,
}

impl<S> Prefixed<S> {
    pub(super) fn rewind(&mut self, mut wire_prefix: Vec<u8>) {
        if self.pos < self.prefix.len() {
            wire_prefix.extend_from_slice(&self.prefix[self.pos..]);
        }
        self.prefix = wire_prefix;
        self.pos = 0;
    }
}

impl Prefixed<TcpStream> {
    pub(super) fn into_idle(self) -> Option<TcpStream> {
        if self.pos >= self.prefix.len() {
            Some(self.inner)
        } else {
            None
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Prefixed<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.pos < self.prefix.len() {
            let n = (self.prefix.len() - self.pos).min(buf.remaining());
            buf.put_slice(&self.prefix[self.pos..self.pos + n]);
            self.pos += n;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Prefixed<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub(super) async fn copy_bidirectional<A, B>(a: &mut A, b: &mut B) -> Result<(u64, u64)>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    Ok(tokio::io::copy_bidirectional(a, b).await?)
}

/// Forwards bytes immediately and parses WebSocket frames from the copy.
/// A bad frame stops the parser only. The splice keeps going.
pub(super) async fn copy_with_messages<A, B>(
    client: &mut A,
    origin: &mut B,
    hub: &Hub,
    id: u64,
    already_to_client: &[u8],
) -> Result<()>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let mut to_origin = FrameFeed::default();
    let mut to_client = FrameFeed::default();
    if !already_to_client.is_empty() {
        note_frames(
            &mut to_client,
            already_to_client,
            hub,
            id,
            Direction::Response,
        );
    }
    let mut client_buf = vec![0u8; 16 * 1024];
    let mut origin_buf = vec![0u8; 16 * 1024];
    let mut client_done = false;
    let mut origin_done = false;
    loop {
        if client_done && origin_done {
            break;
        }
        tokio::select! {
            result = client.read(&mut client_buf), if !client_done => {
                let n = result?;
                if n == 0 {
                    client_done = true;
                    origin.shutdown().await.ok();
                } else {
                    note_frames(&mut to_origin, &client_buf[..n], hub, id, Direction::Request);
                    origin.write_all(&client_buf[..n]).await?;
                }
            }
            result = origin.read(&mut origin_buf), if !origin_done => {
                let n = result?;
                if n == 0 {
                    origin_done = true;
                    client.shutdown().await.ok();
                } else {
                    note_frames(&mut to_client, &origin_buf[..n], hub, id, Direction::Response);
                    client.write_all(&origin_buf[..n]).await?;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn note_frames(
    feed: &mut FrameFeed,
    data: &[u8],
    hub: &Hub,
    id: u64,
    direction: Direction,
) {
    for frame in feed.push(data) {
        hub.add_message(
            id,
            direction,
            ws::frame_label(frame.opcode),
            ws::frame_text(frame.opcode, &frame.payload),
        );
    }
}

/// Records a decompressed side copy while the original frames pass through.
pub(super) struct RecordingBody<B> {
    pub(super) inner: Option<B>,
    pub(super) once: Option<Bytes>,
    pub(super) record: Option<Record>,
    pub(super) throttle_bps: Option<u64>,
    pub(super) sleep: Option<Pin<Box<tokio::time::Sleep>>>,
    pub(super) pending_frame: Option<Frame<Bytes>>,
}

pub(super) struct Record {
    pub(super) hub: Arc<Hub>,
    pub(super) id: u64,
    pub(super) direction: Direction,
    pub(super) decoder: Decoder,
    pub(super) finished: bool,
}

impl<B> RecordingBody<B> {
    pub(super) fn new(
        inner: B,
        hub: Arc<Hub>,
        id: u64,
        direction: Direction,
        encoding: Option<&str>,
    ) -> Self {
        let decoder = Decoder::from_header(encoding);
        if let Some(error) = decoder.error() {
            hub.set_encoding_error(id, direction, error.to_string());
        }
        Self {
            inner: Some(inner),
            once: None,
            record: Some(Record {
                hub,
                id,
                direction,
                decoder,
                finished: false,
            }),
            throttle_bps: None,
            sleep: None,
            pending_frame: None,
        }
    }

    pub(super) fn passthrough(inner: B) -> Self {
        Self {
            inner: Some(inner),
            once: None,
            record: None,
            throttle_bps: None,
            sleep: None,
            pending_frame: None,
        }
    }

    pub(super) fn from_bytes(bytes: impl Into<Bytes>) -> Self {
        let bytes = bytes.into();
        Self {
            inner: None,
            once: if bytes.is_empty() { None } else { Some(bytes) },
            record: None,
            throttle_bps: None,
            sleep: None,
            pending_frame: None,
        }
    }

    pub(super) fn with_throttle(mut self, bps: Option<u64>) -> Self {
        self.throttle_bps = bps.filter(|&rate| rate > 0);
        self
    }

    /// Yields `prefix` first, then `inner`. Only `inner` is recorded; the caller
    /// already stored `prefix`.
    pub(super) fn prefixed(
        prefix: Bytes,
        inner: B,
        hub: Arc<Hub>,
        id: u64,
        direction: Direction,
        encoding: Option<&str>,
    ) -> Self {
        let mut body = Self::new(inner, hub, id, direction, encoding);
        if !prefix.is_empty() {
            body.once = Some(prefix);
        }
        body
    }

    /// Yields `prefix` first, then `inner`, feeding both through the same
    /// streaming `Decoder` when a rule-buffered stream exceeded `BODY_CAP`.
    pub(super) fn stream_with_prefix(
        prefix: Bytes,
        inner: B,
        hub: Arc<Hub>,
        id: u64,
        direction: Direction,
        encoding: Option<&str>,
    ) -> Self {
        let mut body = Self::new(inner, hub, id, direction, encoding);
        if !prefix.is_empty() {
            body.record_bytes(&prefix);
            body.once = Some(prefix);
        }
        body
    }

    pub(super) fn record_bytes(&mut self, bytes: &[u8]) {
        let Some(record) = self.record.as_mut() else {
            return;
        };
        let decoded = record.decoder.push(bytes);
        let error = record.decoder.error().map(|err| err.to_string());
        record.hub.record_chunk(
            record.id,
            record.direction,
            bytes.len() as u64,
            &decoded,
            error.as_deref(),
        );
    }

    pub(super) fn finish_record(&mut self) {
        let Some(record) = self.record.as_mut() else {
            return;
        };
        if record.finished {
            return;
        }
        record.finished = true;
        let direction = record.direction;
        let id = record.id;
        let hub = record.hub.clone();
        let decoded = record.decoder.finish();
        record
            .hub
            .append_body(record.id, record.direction, &decoded);
        if direction == Direction::Response {
            hub.mark_finished(id);
        }
    }
}

impl<B> Drop for RecordingBody<B> {
    fn drop(&mut self) {
        // An empty body (304, HEAD) is dropped after `is_end_stream` without
        // `poll_frame` ever running.
        self.finish_record();
    }
}

impl<B> Body for RecordingBody<B>
where
    B: Body<Data = Bytes> + Unpin,
{
    type Data = Bytes;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        use std::future::Future as _;
        let this = self.get_mut();
        loop {
            if let Some(sleep) = this.sleep.as_mut() {
                if sleep.as_mut().poll(cx).is_pending() {
                    return Poll::Pending;
                }
                this.sleep = None;
            }
            if let Some(frame) = this.pending_frame.take() {
                if this.once.is_none()
                    && this
                        .inner
                        .as_ref()
                        .is_none_or(|inner| inner.is_end_stream())
                {
                    this.finish_record();
                }
                return Poll::Ready(Some(Ok(frame)));
            }
            if let Some(mut bytes) = this.once.take() {
                let chunk = if this.throttle_bps.is_some() && bytes.len() > THROTTLE_CHUNK_BYTES {
                    let head = bytes.split_to(THROTTLE_CHUNK_BYTES);
                    this.once = Some(bytes);
                    head
                } else {
                    bytes
                };
                let pause = throttle_sleep_duration(chunk.len(), this.throttle_bps);
                if !pause.is_zero() {
                    this.pending_frame = Some(Frame::data(chunk));
                    this.sleep = Some(Box::pin(tokio::time::sleep(pause)));
                    continue;
                }
                return Poll::Ready(Some(Ok(Frame::data(chunk))));
            }
            let Some(inner) = this.inner.as_mut() else {
                this.finish_record();
                return Poll::Ready(None);
            };
            match Pin::new(inner).poll_frame(cx) {
                Poll::Ready(Some(Ok(frame))) => {
                    let mut pause = Duration::ZERO;
                    if let Some(data) = frame.data_ref() {
                        let bytes = data.clone();
                        this.record_bytes(&bytes);
                        pause = throttle_sleep_duration(bytes.len(), this.throttle_bps);
                    }
                    if !pause.is_zero() {
                        this.pending_frame = Some(frame);
                        this.sleep = Some(Box::pin(tokio::time::sleep(pause)));
                        continue;
                    }
                    // hyper stops polling once `is_end_stream` is true, so the
                    // trailing `None` never arrives for the last DATA frame.
                    if this
                        .inner
                        .as_ref()
                        .is_some_and(|inner| inner.is_end_stream())
                    {
                        this.finish_record();
                    }
                    return Poll::Ready(Some(Ok(frame)));
                }
                Poll::Ready(None) => {
                    this.finish_record();
                    return Poll::Ready(None);
                }
                other => return other,
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        if self.once.is_some() || self.pending_frame.is_some() || self.sleep.is_some() {
            return false;
        }
        self.inner
            .as_ref()
            .map(|inner| inner.is_end_stream())
            .unwrap_or(true)
    }

    fn size_hint(&self) -> http_body::SizeHint {
        if let Some(bytes) = &self.once
            && self.pending_frame.is_none()
            && self.inner.is_none()
        {
            let mut hint = http_body::SizeHint::new();
            hint.set_exact(bytes.len() as u64);
            return hint;
        }
        self.inner
            .as_ref()
            .map(|inner| inner.size_hint())
            .unwrap_or_default()
    }
}
